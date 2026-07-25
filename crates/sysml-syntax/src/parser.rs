//! Hand-written recursive-descent parser producing a rowan green tree.
//!
//! The parser is error-tolerant: it never panics on bad input and always
//! produces a tree whose text is exactly the input, plus diagnostics.
//!
//! The grammar intentionally over-approximates KerML/SysML v2: declarations,
//! statements and expressions share a tolerant "element tail" loop, so many
//! construct families parse through common machinery. The trade-off is
//! tolerance over rejection — some invalid models parse without errors.
//! Conformance is validated against the official SysML-v2-Release corpus
//! (see `sysml corpus` and the corpus regression tests).

use rowan::{Checkpoint, GreenNode, GreenNodeBuilder};

use crate::lexer::Token;
use crate::{Diagnostic, SyntaxKind, SyntaxKind::*, SyntaxNode};

/// Result of parsing: a green tree plus diagnostics.
#[derive(Debug, Clone)]
pub struct Parse {
    green: GreenNode,
    errors: Vec<Diagnostic>,
}

impl Parse {
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    pub fn errors(&self) -> &[Diagnostic] {
        &self.errors
    }

    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Parse a whole source file as SysML (see [`parse_dialect`]).
pub fn parse(text: &str) -> Parse {
    parse_dialect(text, crate::Dialect::SysML)
}

/// Parse a whole `.sysml` / `.kerml` source file in the given dialect.
pub fn parse_dialect(text: &str, dialect: crate::Dialect) -> Parse {
    let (tokens, lex_errors) = crate::lexer::lex_dialect(text, dialect);
    let mut parser = Parser {
        text,
        tokens,
        pos: 0,
        builder: GreenNodeBuilder::new(),
        errors: lex_errors,
    };
    parser.source_file();
    Parse {
        green: parser.builder.finish(),
        errors: parser.errors,
    }
}

struct Parser<'t> {
    text: &'t str,
    tokens: Vec<Token>,
    /// Index into `tokens`, pointing at the next not-yet-consumed token
    /// (trivia included).
    pos: usize,
    builder: GreenNodeBuilder<'static>,
    errors: Vec<Diagnostic>,
}

impl Parser<'_> {
    // --- cursor primitives -------------------------------------------------

    /// Kind of the n-th upcoming non-trivia token (`EOF` past the end).
    fn nth(&self, n: usize) -> SyntaxKind {
        self.tokens[self.pos..]
            .iter()
            .filter(|t| !t.kind.is_trivia())
            .nth(n)
            .map_or(EOF, |t| t.kind)
    }

    fn current(&self) -> SyntaxKind {
        self.nth(0)
    }

    fn at(&self, kind: SyntaxKind) -> bool {
        self.current() == kind
    }

    /// Byte range of the current non-trivia token (empty range at EOF).
    fn current_range(&self) -> std::ops::Range<usize> {
        self.tokens[self.pos..]
            .iter()
            .find(|t| !t.kind.is_trivia())
            .map_or(self.text.len()..self.text.len(), |t| t.range.clone())
    }

    fn token_into_builder(&mut self) {
        let token = self.tokens[self.pos].clone();
        self.builder
            .token(token.kind.into(), &self.text[token.range]);
        self.pos += 1;
    }

    fn flush_trivia(&mut self) {
        while self
            .tokens
            .get(self.pos)
            .is_some_and(|t| t.kind.is_trivia())
        {
            self.token_into_builder();
        }
    }

    /// Consume the current non-trivia token (attaching preceding trivia to
    /// the currently open node). No-op at EOF.
    fn bump(&mut self) {
        self.flush_trivia();
        if self.pos < self.tokens.len() {
            self.token_into_builder();
        }
    }

    fn eat(&mut self, kind: SyntaxKind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: SyntaxKind) {
        if !self.eat(kind) {
            self.error(format!("expected {kind:?}, found {:?}", self.current()));
        }
    }

    fn error(&mut self, message: impl Into<String>) {
        let range = self.current_range();
        self.errors.push(Diagnostic::new(&range, message));
    }

    // --- node primitives ---------------------------------------------------

    fn start_node(&mut self, kind: SyntaxKind) {
        self.flush_trivia();
        self.builder.start_node(kind.into());
    }

    fn checkpoint(&mut self) -> Checkpoint {
        self.flush_trivia();
        self.builder.checkpoint()
    }

    fn start_node_at(&mut self, checkpoint: Checkpoint, kind: SyntaxKind) {
        self.builder.start_node_at(checkpoint, kind.into());
    }

    fn finish_node(&mut self) {
        self.builder.finish_node();
    }

    /// Wrap the current token in an `ERROR` node and report `message`.
    fn error_and_bump(&mut self, message: impl Into<String>) {
        self.error(message);
        self.start_node(ERROR);
        self.bump();
        self.finish_node();
    }

    // --- token classes -----------------------------------------------------

    fn at_name(&self) -> bool {
        matches!(self.current(), IDENT | UNRESTRICTED_NAME)
    }

    fn nth_is_name(&self, n: usize) -> bool {
        matches!(self.nth(n), IDENT | UNRESTRICTED_NAME)
    }

    /// A bare name that the next token confirms to be a declaration
    /// (`level = 3;`, `x : T;`, enum values `red;`).
    fn at_anonymous_usage(&self) -> bool {
        self.at_name()
            && matches!(
                self.nth(1),
                COLON
                    | COLON_GT
                    | COLON_GT_GT
                    | COLON_COLON_GT
                    | EQ
                    | COLON_EQ
                    | SEMICOLON
                    | L_BRACE
                    | L_BRACKET
                    | DEFAULT_KW
            )
    }

    /// Does `(` open a parameter list (rather than an expression)?
    fn at_param_list(&self) -> bool {
        let k1 = self.nth(1);
        k1.is_modifier_kw()
            || k1.is_def_kind_kw()
            || k1 == L_BRACKET // `([1] myCart, [0..1] products)`
            || (matches!(k1, IDENT | UNRESTRICTED_NAME)
                && matches!(
                    self.nth(2),
                    COLON | COLON_GT | COLON_GT_GT | COLON_COLON_GT | COLON_EQ | DEFAULT_KW
                ))
    }

    fn at_expr_start(&self) -> bool {
        matches!(
            self.current(),
            IDENT
                | UNRESTRICTED_NAME
                | DECIMAL
                | REAL
                | STRING
                | TRUE_KW
                | FALSE_KW
                | NULL_KW
                | L_PAREN
                | MINUS
                | PLUS
                | TILDE
                | NOT_KW
                | IF_KW
                | AT
                | STAR
                | DOLLAR
                | NEW_KW
        )
    }

    // --- grammar: file and members -----------------------------------------

    fn source_file(&mut self) {
        self.builder.start_node(SOURCE_FILE.into());
        while !self.at(EOF) {
            self.member();
        }
        self.flush_trivia();
        self.finish_node();
    }

    /// One member of a namespace/definition body (or of the root namespace).
    fn member(&mut self) {
        let cp = self.checkpoint();
        // visibility and `#name` user-defined-keyword prefixes, in any order
        loop {
            if self.at(HASH) && self.nth_is_name(1) {
                self.opt_prefix_metadata();
            } else if self.current().is_visibility_kw() {
                self.bump();
            } else {
                break;
            }
        }
        match self.current() {
            PACKAGE_KW | LIBRARY_KW | STANDARD_KW | NAMESPACE_KW => self.package(cp),
            IMPORT_KW => self.import(cp, IMPORT),
            EXPOSE_KW => self.import(cp, EXPOSE),
            ALIAS_KW => self.alias(cp),
            DOC_KW => self.documentation(cp),
            COMMENT_KW => self.comment_elem(cp),
            COMMENT_BODY => {
                // a bare `/* ... */` is a comment element about its owner
                self.start_node_at(cp, COMMENT_ELEM);
                self.bump();
                self.finish_node();
            }
            REP_KW | LANGUAGE_KW => self.rep(cp),
            FILTER_KW => self.filter_member(cp),
            AT | AT_AT => self.metadata_annotation(cp),
            DEPENDENCY_KW => self.lead_stmt(cp, DEPENDENCY),
            CONNECT_KW | BIND_KW | ALLOCATE_KW => self.lead_stmt(cp, CONNECTOR_STMT),
            // `if` is handled as an expression statement: a conditional
            // result expression parses whole, and an action `if cond then t;`
            // continues via a `then` statement member.
            FIRST_KW | THEN_KW | ELSE_KW | WHILE_KW | UNTIL_KW | FOR_KW | LOOP_KW | MERGE_KW
            | DECIDE_KW | FORK_KW | JOIN_KW | SEND_KW | ACCEPT_KW | ASSIGN_KW | TERMINATE_KW
            | DO_KW | ENTRY_KW | EXIT_KW | TRANSITION_KW => self.lead_stmt(cp, CONTROL_STMT),
            SPECIALIZATION_KW | SUBCLASSIFIER_KW | SUBTYPE_KW | SUBSET_KW | REDEFINITION_KW
            | CONJUGATION_KW | DISJOINING_KW | DISJOINT_KW | INVERTING_KW | INVERSE_KW
            | FEATURING_KW | TYPING_KW => self.lead_stmt(cp, RELATION_STMT),
            PERFORM_KW | EXHIBIT_KW | EVENT_KW | INCLUDE_KW | SATISFY_KW | ASSERT_KW
            | ASSUME_KW | REQUIRE_KW | VERIFY_KW | FRAME_KW => self.adapter_usage(cp),
            NOT_KW
                if matches!(
                    self.nth(1),
                    PERFORM_KW
                        | EXHIBIT_KW
                        | EVENT_KW
                        | INCLUDE_KW
                        | SATISFY_KW
                        | ASSERT_KW
                        | ASSUME_KW
                        | REQUIRE_KW
                        | VERIFY_KW
                        | FRAME_KW
                ) =>
            {
                self.adapter_usage(cp)
            }
            // `#service def X { ... }` — definition with only user keywords
            DEF_KW => self.definition_or_usage(cp),
            // bare `locale "en_US" /* ... */` comment element
            LOCALE_KW => {
                self.start_node_at(cp, COMMENT_ELEM);
                self.bump();
                self.expect(STRING);
                self.expect(COMMENT_BODY);
                self.finish_node();
            }
            // `message` declarations may be named (`message messages : M ...`)
            SUBJECT_KW | ACTOR_KW | STAKEHOLDER_KW | OBJECTIVE_KW | RETURN_KW | VARIANT_KW
            | RENDER_KW | DEFAULT_KW | MESSAGE_KW => self.kw_usage(cp),
            k if k.is_modifier_kw() || k.is_def_kind_kw() => self.definition_or_usage(cp),
            // `:>> quantity = isq.L;` — a kind-less usage starting with a
            // feature specialization
            COLON_GT | COLON_GT_GT | COLON_COLON_GT | SPECIALIZES_KW | SUBSETS_KW
            | REDEFINES_KW | REFERENCES_KW | CROSSES_KW | EQ | COLON_EQ => {
                self.start_node_at(cp, USAGE);
                self.element_tail();
                self.finish_node();
            }
            // `level = 3;` / `x : T;` — a kind-less (anonymous) usage
            _ if self.at_anonymous_usage() => {
                self.start_node_at(cp, USAGE);
                self.opt_name();
                self.element_tail();
                self.finish_node();
            }
            _ if self.at_expr_start() => {
                self.start_node_at(cp, EXPR_STMT);
                self.expression();
                if self.at(L_BRACE) {
                    // structured control: `if i < 0 { ... }`
                    self.body();
                }
                self.eat(SEMICOLON);
                self.finish_node();
            }
            _ => self.error_and_bump(format!(
                "expected a member (definition, usage, package, import, ...), found {:?}",
                self.current()
            )),
        }
    }

    /// `standard? library? (package | namespace) Name? Body`
    fn package(&mut self, cp: Checkpoint) {
        self.start_node_at(cp, PACKAGE);
        self.eat(STANDARD_KW);
        self.eat(LIBRARY_KW);
        if !self.eat(PACKAGE_KW) && !self.eat(NAMESPACE_KW) {
            self.error(format!("expected `package`, found {:?}", self.current()));
        }
        self.opt_short_name();
        self.opt_name();
        self.element_tail();
        self.finish_node();
    }

    /// `import all? A::B(::* | ::**)? [filter]* Body`
    fn import(&mut self, cp: Checkpoint, node: SyntaxKind) {
        self.start_node_at(cp, node);
        self.bump();
        self.eat(ALL_KW);
        self.qualified_name(true);
        while self.at(L_BRACKET) {
            self.start_node(FILTER);
            self.bump();
            if !self.at(R_BRACKET) {
                self.expression();
            }
            self.expect(R_BRACKET);
            self.finish_node();
        }
        self.element_tail();
        self.finish_node();
    }

    /// `alias <s>? Name for A::B ;`
    fn alias(&mut self, cp: Checkpoint) {
        self.start_node_at(cp, ALIAS);
        self.bump();
        self.opt_short_name();
        self.opt_name();
        self.expect(FOR_KW);
        self.qualified_name_chain();
        self.element_tail();
        self.finish_node();
    }

    /// `doc Name? (locale "...")? /* ... */`
    fn documentation(&mut self, cp: Checkpoint) {
        self.start_node_at(cp, DOCUMENTATION);
        self.bump();
        self.opt_short_name();
        self.opt_name();
        if self.eat(LOCALE_KW) {
            self.expect(STRING);
        }
        self.expect(COMMENT_BODY);
        self.finish_node();
    }

    /// `comment Name? (about A, B)? (locale "...")? /* ... */`
    fn comment_elem(&mut self, cp: Checkpoint) {
        self.start_node_at(cp, COMMENT_ELEM);
        self.bump();
        self.opt_short_name();
        self.opt_name();
        if self.at(ABOUT_KW) {
            self.about_part();
        }
        if self.eat(LOCALE_KW) {
            self.expect(STRING);
        }
        self.expect(COMMENT_BODY);
        self.finish_node();
    }

    fn about_part(&mut self) {
        self.start_node(ABOUT);
        self.bump();
        self.type_ref();
        while self.eat(COMMA) {
            self.type_ref();
        }
        self.finish_node();
    }

    /// `rep Name? language "lang" /* ... */` (or bare `language "lang" /* */`)
    fn rep(&mut self, cp: Checkpoint) {
        self.start_node_at(cp, REP);
        if self.eat(REP_KW) {
            self.opt_short_name();
            self.opt_name();
            self.expect(LANGUAGE_KW);
        } else {
            self.bump(); // `language`
        }
        self.expect(STRING);
        self.expect(COMMENT_BODY);
        self.finish_node();
    }

    /// `filter expr ;`
    fn filter_member(&mut self, cp: Checkpoint) {
        self.start_node_at(cp, FILTER);
        self.bump();
        self.expression();
        self.expect(SEMICOLON);
        self.finish_node();
    }

    /// `@M about x; | @M { ... } | @@M ...`
    fn metadata_annotation(&mut self, cp: Checkpoint) {
        self.start_node_at(cp, METADATA_ANNOTATION);
        self.bump();
        if self.at_name() {
            self.qualified_name_chain();
        }
        self.element_tail();
        self.finish_node();
    }

    /// `connect a to b;`, `first a then b;`, `specialization s subtype ...`
    fn lead_stmt(&mut self, cp: Checkpoint, node: SyntaxKind) {
        self.start_node_at(cp, node);
        self.bump();
        self.element_tail();
        self.finish_node();
    }

    /// `perform action a ...`, `assert not constraint ...`, `event x.y;`, ...
    fn adapter_usage(&mut self, cp: Checkpoint) {
        self.start_node_at(cp, USAGE);
        self.eat(NOT_KW); // `not satisfy r1 by p;`
        self.bump();
        self.eat(NOT_KW);
        self.opt_prefix_metadata(); // `assume #goal constraint c;`
        if self.current().is_def_kind_kw() {
            self.def_kind_keywords();
            self.opt_short_name();
            self.opt_decl_name();
            if self.at(L_PAREN) {
                self.param_list();
            }
        }
        self.element_tail();
        self.finish_node();
    }

    /// `subject x : T;`, `return x;`, `variant part v;`, `render asTree;`
    fn kw_usage(&mut self, cp: Checkpoint) {
        self.start_node_at(cp, USAGE);
        self.bump();
        self.opt_short_name();
        self.opt_decl_name();
        self.element_tail();
        self.finish_node();
    }

    /// Bump a definition/usage kind keyword sequence (`part`, `use case`,
    /// `assoc struct`, `succession flow`, ...).
    fn def_kind_keywords(&mut self) {
        while self.current().is_def_kind_kw() {
            let k = self.current();
            self.bump();
            if k == USE_KW {
                self.eat(CASE_KW);
            }
            if !matches!(k, ASSOC_KW | SUCCESSION_KW) {
                break;
            }
        }
    }

    /// Definitions and usages share modifiers and kind keywords; `def` (or a
    /// KerML classifier kind) decides definition vs usage.
    fn opt_prefix_metadata(&mut self) {
        while self.at(HASH) && self.nth_is_name(1) {
            self.start_node(PREFIX_METADATA);
            self.bump();
            self.qualified_name_chain();
            self.finish_node();
        }
    }

    fn definition_or_usage(&mut self, cp: Checkpoint) {
        loop {
            if self.current().is_modifier_kw() {
                self.bump();
            } else if self.at(HASH) && self.nth_is_name(1) {
                self.opt_prefix_metadata();
            } else {
                break;
            }
        }
        // `abstract message messages : M ...` — message declarations after
        // modifiers behave like a usage kind
        self.eat(MESSAGE_KW);
        let is_classifier = matches!(
            self.current(),
            TYPE_KW
                | CLASSIFIER_KW
                | CLASS_KW
                | DATATYPE_KW
                | STRUCT_KW
                | ASSOC_KW
                | BEHAVIOR_KW
                | FUNCTION_KW
                | PREDICATE_KW
                | INTERACTION_KW
                | METACLASS_KW
        );
        self.def_kind_keywords();
        let node = if self.at(DEF_KW) || is_classifier {
            DEFINITION
        } else {
            USAGE
        };
        self.start_node_at(cp, node);
        self.eat(DEF_KW);
        self.eat(ALL_KW); // `struct all Curve` — sufficiency marker
        self.opt_prefix_metadata();
        self.opt_short_name();
        if node == DEFINITION {
            self.opt_name();
        } else {
            // usages: only take a name the next token confirms, so that
            // `flow p1.torque to p2.torque;` keeps `p1.torque` an expression
            self.opt_decl_name();
        }
        if self.at(L_PAREN) {
            self.param_list();
        }
        self.element_tail();
        self.finish_node();
    }

    /// Everything after the lead-in of a declaration or statement, up to and
    /// including the body (`;` or `{ ... }`). Tolerant by design: feature
    /// specializations, multiplicities, values, statement continuation
    /// keywords and expressions may appear in any order.
    fn element_tail(&mut self) {
        loop {
            match self.current() {
                SEMICOLON => {
                    self.start_node(BODY);
                    self.bump();
                    self.finish_node();
                    return;
                }
                L_BRACE => {
                    self.body();
                    return;
                }
                EOF | R_BRACE => {
                    self.error("expected `;` or `{`");
                    return;
                }
                COLON | TYPED_KW | DEFINED_KW => self.typing_part(),
                COLON_GT | SPECIALIZES_KW | SUBSETS_KW | CROSSES_KW => {
                    self.relationship_part(SUBSETTING)
                }
                COLON_GT_GT | REDEFINES_KW => self.relationship_part(REDEFINITION),
                COLON_COLON_GT | REFERENCES_KW => self.relationship_part(REFERENCES),
                CHAINS_KW | UNIONS_KW | INTERSECTS_KW | DIFFERENCES_KW | DISJOINT_KW
                | INVERSE_KW | FEATURED_KW | CONJUGATES_KW | CONJUGATE_KW => {
                    self.kerml_relation_part()
                }
                L_BRACKET => self.multiplicity(),
                EQ | COLON_EQ | DEFAULT_KW => self.value_part(),
                ABOUT_KW => self.about_part(),
                // `( cause1 ::> causer1, ... )` is a parameter list;
                // `for n in (1, 2, 3)` is an expression
                L_PAREN if self.at_param_list() => self.param_list(),
                L_PAREN => self.expression(),
                // statement continuation keywords and misc glue
                OF_KW | FROM_KW | TO_KW | VIA_KW | THEN_KW | ELSE_KW | FIRST_KW | ACCEPT_KW
                | AT_KW | AFTER_KW | WHEN_KW | UNTIL_KW | WHILE_KW | DO_KW | BY_KW | ALL_KW
                | PARALLEL_KW | GUARD_KW | EFFECT_KW | ASSIGN_KW | SEND_KW | TRIGGER_KW
                | MERGE_KW | DECIDE_KW | FORK_KW | JOIN_KW | TERMINATE_KW | LOOP_KW | NEW_KW
                | ORDERED_KW | NONUNIQUE_KW | CONNECT_KW | BIND_KW | MESSAGE_KW | ALLOCATE_KW
                | SUBTYPE_KW | SUBSET_KW | REDEFINITION_KW | TYPING_KW | SPECIALIZATION_KW
                | SUBCLASSIFIER_KW | COMMA | QUESTION | QUESTION_QUESTION | FAT_ARROW
                | LANGUAGE_KW => self.bump(),
                // nested declarations inside statements: `then perform body;`,
                // `then private action whileLoop { ... }`
                PERFORM_KW | EXHIBIT_KW | EVENT_KW | INCLUDE_KW | SATISFY_KW | ASSERT_KW
                | ASSUME_KW | REQUIRE_KW | VERIFY_KW | FRAME_KW => self.bump(),
                k if k.is_visibility_kw() => self.bump(),
                // `connect [1] lugNutPort ::> wheel.lugNutPort to ...` — a
                // named connector end; parse just the name and its
                // reference so `to`/`from` stay with the enclosing statement
                _ if self.at_name() && matches!(self.nth(1), COLON_COLON_GT | REFERENCES_KW) => {
                    self.start_node(USAGE);
                    self.opt_name();
                    self.relationship_part(REFERENCES);
                    self.finish_node();
                }
                // `end x [1..*] feature y : T redefines z;` — a nested
                // feature declaration ends the enclosing element
                k if k.is_def_kind_kw() && self.nth_is_name(1) => {
                    let nested = self.checkpoint();
                    self.definition_or_usage(nested);
                    return;
                }
                k if k.is_def_kind_kw() || k.is_modifier_kw() => self.bump(),
                _ if self.at_expr_start() => self.expression(),
                _ => self.error_and_bump(format!(
                    "unexpected token {:?} in declaration",
                    self.current()
                )),
            }
        }
    }

    /// `{ member* }`
    fn body(&mut self) {
        self.start_node(BODY);
        self.bump();
        while !matches!(self.current(), R_BRACE | EOF) {
            self.member();
        }
        self.expect(R_BRACE);
        self.finish_node();
    }

    /// A comma continues a type-reference list only when what follows looks
    /// like another plain reference — `in x : X, out y : Y` must leave the
    /// comma to the parameter list.
    fn continues_ref_list(&self) -> bool {
        self.at(COMMA)
            && (self.nth_is_name(1) || self.nth(1) == TILDE)
            && !matches!(
                self.nth(2),
                COLON | COLON_GT | COLON_GT_GT | COLON_COLON_GT | EQ | COLON_EQ | DEFAULT_KW
            )
    }

    /// `: T, U` / `typed by T` / `defined by T`
    fn typing_part(&mut self) {
        self.start_node(TYPING);
        if self.eat(TYPED_KW) || self.eat(DEFINED_KW) {
            self.expect(BY_KW);
        } else {
            self.bump(); // `:`
        }
        self.type_ref();
        while self.continues_ref_list() {
            self.bump();
            self.type_ref();
        }
        self.finish_node();
    }

    /// `(:> | specializes | subsets | crosses | :>> | redefines | ::> | references) T, U`
    fn relationship_part(&mut self, kind: SyntaxKind) {
        self.start_node(kind);
        self.bump();
        self.type_ref();
        while self.continues_ref_list() {
            self.bump();
            self.type_ref();
        }
        self.finish_node();
    }

    /// KerML: `chains a.b`, `disjoint from T`, `inverse of f`, `featured by T`,
    /// `unions T`, `conjugates T`, ...
    fn kerml_relation_part(&mut self) {
        self.start_node(RELATION);
        let lead = self.current();
        self.bump();
        match lead {
            DISJOINT_KW => {
                self.eat(FROM_KW);
            }
            INVERSE_KW => {
                self.eat(OF_KW);
            }
            FEATURED_KW => {
                self.eat(BY_KW);
            }
            _ => {}
        }
        self.type_ref();
        while self.continues_ref_list() {
            self.bump();
            self.type_ref();
        }
        self.finish_node();
    }

    /// `[ ... ]` — the contents are kept as loose tokens without
    /// further structure.
    fn multiplicity(&mut self) {
        self.start_node(MULTIPLICITY);
        self.bump();
        while !matches!(
            self.current(),
            R_BRACKET | SEMICOLON | L_BRACE | R_BRACE | EOF
        ) {
            self.bump();
        }
        self.expect(R_BRACKET);
        self.finish_node();
    }

    /// `= expr` / `:= expr` / `default (= | :=)? expr`
    fn value_part(&mut self) {
        self.start_node(VALUE);
        if self.eat(DEFAULT_KW) {
            let _ = self.eat(EQ) || self.eat(COLON_EQ);
        } else {
            self.bump(); // `=` or `:=`
        }
        if self.at_expr_start() || self.at(L_BRACE) {
            self.expression();
        }
        self.finish_node();
    }

    /// `( param, param )` where each param is a lightweight usage.
    fn param_list(&mut self) {
        self.start_node(PARAM_LIST);
        self.bump();
        while !matches!(self.current(), R_PAREN | EOF) {
            self.param();
            if !self.eat(COMMA) {
                break;
            }
        }
        self.expect(R_PAREN);
        self.finish_node();
    }

    fn param(&mut self) {
        self.start_node(USAGE);
        while self.current().is_modifier_kw() {
            self.bump();
        }
        if self.current().is_def_kind_kw() {
            self.def_kind_keywords();
        }
        if self.at(L_BRACKET) {
            // `[1] myCart` — multiplicity before the parameter name
            self.multiplicity();
        }
        self.opt_short_name();
        self.opt_name();
        loop {
            match self.current() {
                COLON | TYPED_KW | DEFINED_KW => self.typing_part(),
                COLON_GT | SPECIALIZES_KW | SUBSETS_KW => self.relationship_part(SUBSETTING),
                COLON_GT_GT | REDEFINES_KW => self.relationship_part(REDEFINITION),
                COLON_COLON_GT | REFERENCES_KW => self.relationship_part(REFERENCES),
                L_BRACKET => self.multiplicity(),
                EQ | COLON_EQ | DEFAULT_KW => self.value_part(),
                ORDERED_KW | NONUNIQUE_KW => self.bump(),
                _ => break,
            }
        }
        self.finish_node();
    }

    // --- names -------------------------------------------------------------

    fn opt_name(&mut self) {
        if self.at_name() {
            self.start_node(NAME);
            self.bump();
            self.finish_node();
        }
    }

    /// A name is only consumed when the following token confirms this is a
    /// declaration (used for `return x;` vs `return x * 2;`).
    fn opt_decl_name(&mut self) {
        if self.at_name()
            && matches!(
                self.nth(1),
                COLON
                    | COLON_GT
                    | COLON_GT_GT
                    | COLON_COLON_GT
                    | L_BRACKET
                    | L_PAREN
                    | EQ
                    | COLON_EQ
                    | SEMICOLON
                    | L_BRACE
                    | DEFAULT_KW
                    | ORDERED_KW
                    | NONUNIQUE_KW
                    | OF_KW
                    | FROM_KW
                    | TO_KW
                    | VIA_KW
                    | SPECIALIZES_KW
                    | SUBSETS_KW
                    | REDEFINES_KW
                    | REFERENCES_KW
                    | CROSSES_KW
                    | TYPED_KW
                    | DEFINED_KW
                    | CHAINS_KW
            )
        {
            self.opt_name();
        }
    }

    /// `<shortName>`
    fn opt_short_name(&mut self) {
        if self.at(LT) {
            self.start_node(SHORT_NAME);
            self.bump();
            while !matches!(self.current(), GT | SEMICOLON | L_BRACE | R_BRACE | EOF) {
                self.bump();
            }
            self.expect(GT);
            self.finish_node();
        }
    }

    /// A (possibly conjugated `~`) reference to a type or feature chain.
    fn type_ref(&mut self) {
        self.start_node(TYPE_REF);
        self.eat(TILDE);
        if self.at_name() || self.at(DOLLAR) {
            self.qualified_name_chain();
        } else {
            self.error(format!("expected a name, found {:?}", self.current()));
        }
        self.finish_node();
    }

    /// `A::B::C`, optionally ending in `::*` / `::**` (imports).
    fn qualified_name(&mut self, allow_wildcards: bool) {
        self.start_node(QUALIFIED_NAME);
        if self.at_name() || (allow_wildcards && matches!(self.current(), STAR | STAR_STAR)) {
            self.bump();
        } else {
            self.error(format!("expected a name, found {:?}", self.current()));
        }
        while self.at(COLON_COLON) {
            self.bump();
            match self.current() {
                IDENT | UNRESTRICTED_NAME => self.bump(),
                STAR | STAR_STAR if allow_wildcards => self.bump(),
                _ => {
                    self.error(format!(
                        "expected a name after `::`, found {:?}",
                        self.current()
                    ));
                    break;
                }
            }
        }
        self.finish_node();
    }

    /// `A::B.c.d` — qualified name that may continue as a feature chain
    /// (`$::A::B` roots at the global namespace).
    fn qualified_name_chain(&mut self) {
        self.start_node(QUALIFIED_NAME);
        if self.at_name() || self.at(DOLLAR) {
            self.bump();
        } else {
            self.error(format!("expected a name, found {:?}", self.current()));
        }
        while matches!(self.current(), COLON_COLON | DOT) && self.nth_is_name(1) {
            self.bump();
            self.bump();
        }
        self.finish_node();
    }

    // --- expressions (Pratt) ----------------------------------------------

    fn expression(&mut self) {
        self.expr_bp(0);
    }

    fn expr_bp(&mut self, min_bp: u8) {
        let cp = self.checkpoint();
        match self.current() {
            NOT_KW | MINUS | PLUS | TILDE | ALL_KW => {
                self.start_node(UNARY_EXPR);
                self.bump();
                self.expr_bp(13);
                self.finish_node();
            }
            AT | AT_AT => {
                self.start_node(UNARY_EXPR);
                self.bump();
                if self.at_name() {
                    self.qualified_name_chain();
                }
                self.finish_node();
            }
            // `(as Safety)` — classification with elided subject
            AS_KW | ISTYPE_KW | HASTYPE_KW | META_KW => {
                self.start_node(UNARY_EXPR);
                self.bump();
                self.type_ref();
                self.finish_node();
            }
            // `new Translation(args)` — constructor expression
            NEW_KW => {
                self.start_node(UNARY_EXPR);
                self.bump();
                if self.at_name() {
                    self.qualified_name_chain();
                } else {
                    self.error("expected a type name after `new`");
                }
                self.finish_node();
            }
            IF_KW => {
                // `if c ? t else f` (a lone `if c` is left open for
                // transition guards: `accept x if c then s`)
                self.start_node(COND_EXPR);
                self.bump();
                self.expr_bp(0);
                if self.eat(QUESTION) {
                    self.expr_bp(0);
                    if self.eat(ELSE_KW) {
                        self.expr_bp(0);
                    }
                }
                self.finish_node();
            }
            DECIMAL | REAL | STRING | TRUE_KW | FALSE_KW | NULL_KW | STAR | DOLLAR => {
                self.start_node(LITERAL);
                self.bump();
                self.finish_node();
            }
            IDENT | UNRESTRICTED_NAME => {
                self.start_node(NAME_REF);
                self.bump();
                while self.at(COLON_COLON) && self.nth_is_name(1) {
                    self.bump();
                    self.bump();
                }
                self.finish_node();
            }
            L_PAREN => {
                self.start_node(PAREN_EXPR);
                self.bump();
                if !self.at(R_PAREN) {
                    self.expr_bp(0);
                    while self.eat(COMMA) {
                        if self.at(R_PAREN) {
                            break;
                        }
                        self.expr_bp(0);
                    }
                }
                self.expect(R_PAREN);
                self.finish_node();
            }
            // `{ true }` — an expression body (e.g. parameter defaults)
            L_BRACE => self.body_expr(),
            _ => {
                self.error(format!(
                    "expected an expression, found {:?}",
                    self.current()
                ));
                return;
            }
        }
        loop {
            match self.current() {
                DOT | DOT_QUESTION if self.nth_is_name(1) => {
                    self.start_node_at(cp, PATH_EXPR);
                    self.bump();
                    self.bump();
                    self.finish_node();
                    continue;
                }
                // `list.?{in p; cond}` — filtering with a body expression
                DOT | DOT_QUESTION if self.nth(1) == L_BRACE => {
                    self.start_node_at(cp, PATH_EXPR);
                    self.bump();
                    self.body_expr();
                    self.finish_node();
                    continue;
                }
                ARROW => {
                    self.start_node_at(cp, ARROW_EXPR);
                    self.bump();
                    if self.at_name() {
                        self.bump();
                    } else {
                        self.error("expected a function name after `->`");
                    }
                    match self.current() {
                        L_PAREN => self.arg_list(),
                        L_BRACE => self.body_expr(),
                        UNRESTRICTED_NAME | STRING => self.bump(),
                        _ => {}
                    }
                    self.finish_node();
                    continue;
                }
                L_PAREN => {
                    self.start_node_at(cp, CALL_EXPR);
                    self.arg_list();
                    self.finish_node();
                    continue;
                }
                L_BRACKET => {
                    self.start_node_at(cp, INDEX_EXPR);
                    self.bump();
                    if !self.at(R_BRACKET) {
                        self.expr_bp(0);
                        while self.eat(COMMA) {
                            self.expr_bp(0);
                        }
                    }
                    self.expect(R_BRACKET);
                    self.finish_node();
                    continue;
                }
                HASH if self.nth(1) == L_PAREN => {
                    self.start_node_at(cp, INDEX_EXPR);
                    self.bump();
                    self.arg_list();
                    self.finish_node();
                    continue;
                }
                _ => {}
            }
            let Some((lbp, type_rhs)) = binary_bp(self.current()) else {
                break;
            };
            if lbp < min_bp {
                break;
            }
            self.start_node_at(cp, BINARY_EXPR);
            self.bump();
            if type_rhs {
                if self.at_name() || self.at(TILDE) {
                    self.type_ref();
                } else {
                    self.expr_bp(13);
                }
            } else {
                self.expr_bp(lbp + 1);
            }
            self.finish_node();
        }
    }

    /// `( expr, name = expr, ... )`
    fn arg_list(&mut self) {
        self.start_node(ARG_LIST);
        self.bump();
        while !matches!(self.current(), R_PAREN | EOF) {
            if self.at_name() && self.nth(1) == EQ {
                self.start_node(NAME_REF);
                self.bump();
                self.finish_node();
                self.bump();
                self.expr_bp(0);
            } else if self.eat(REDEFINES_KW) {
                self.opt_name();
                if self.eat(EQ) {
                    self.expr_bp(0);
                }
            } else if self.at_expr_start() {
                self.expr_bp(0);
            } else {
                self.error_and_bump("expected an argument");
            }
            if !self.eat(COMMA) {
                break;
            }
        }
        self.expect(R_PAREN);
        self.finish_node();
    }

    /// `{ ... }` used as an expression body (after `->collect` etc.)
    fn body_expr(&mut self) {
        self.start_node(BODY_EXPR);
        self.bump();
        while !matches!(self.current(), R_BRACE | EOF) {
            self.member();
        }
        self.expect(R_BRACE);
        self.finish_node();
    }
}

/// Left binding power of a binary operator; `true` marks operators whose
/// right operand is a type reference (`istype`, `as`, ...).
fn binary_bp(kind: SyntaxKind) -> Option<(u8, bool)> {
    let bp = match kind {
        QUESTION_QUESTION => (1, false),
        IMPLIES_KW => (2, false),
        OR_KW | PIPE => (3, false),
        XOR_KW => (4, false),
        AND_KW | AMP => (5, false),
        EQ_EQ | NOT_EQ | EQ_EQ_EQ | NOT_EQ_EQ => (6, false),
        ISTYPE_KW | HASTYPE_KW | AS_KW | META_KW => (7, true),
        LT | GT | LT_EQ | GT_EQ => (8, false),
        DOT_DOT => (9, false),
        PLUS | MINUS => (10, false),
        STAR | SLASH | PERCENT => (11, false),
        STAR_STAR | CARET => (12, false),
        _ => return None,
    };
    Some(bp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_ok(text: &str) -> Parse {
        let parse = parse(text);
        assert_eq!(parse.errors(), &[], "unexpected errors for:\n{text}");
        assert_eq!(parse.syntax().text().to_string(), text, "lossless violated");
        parse
    }

    #[test]
    fn minimal_definition() {
        let parse = check_ok("part def Vehicle;");
        let tree = format!("{:#?}", parse.syntax());
        let expected = r#"SOURCE_FILE@0..17
  DEFINITION@0..17
    PART_KW@0..4 "part"
    WHITESPACE@4..5 " "
    DEF_KW@5..8 "def"
    WHITESPACE@8..9 " "
    NAME@9..16
      IDENT@9..16 "Vehicle"
    BODY@16..17
      SEMICOLON@16..17 ";"
"#;
        assert_eq!(tree, expected);
    }

    #[test]
    fn package_with_members() {
        check_ok(
            "package VehicleModel {\n    import ScalarValues::*;\n    part def Vehicle {\n        attribute mass : Real = 1200.0;\n        part wheels : Wheel[4];\n    }\n}\n",
        );
    }

    #[test]
    fn specialization_and_redefinition() {
        check_ok("abstract part def PowerSource;");
        check_ok("part def Engine :> PowerSource, Producer { port fuelIn : FuelPort; }");
        check_ok("part e :>> engine { attribute :>> mass = 90.0; }");
        check_ok("part def Engine specializes PowerSource;");
        check_ok("ref part best subsets vehicles;");
    }

    #[test]
    fn docs_comments_aliases() {
        check_ok("package P { doc /* documented */ alias V for Q::Vehicle; }");
        check_ok("comment about Vehicle, Engine /* both of them */");
        check_ok("package P { // note\n //* block note */ part def X; }");
        check_ok("package P { /* bare comment */ }");
    }

    #[test]
    fn imports() {
        check_ok("import A;");
        check_ok("import A::B::C;");
        check_ok("import all Q::*;");
        check_ok("import Q::**;");
        check_ok("private import Q::'quoted name';");
        check_ok("import Q::* [@Safety];");
    }

    /// Every error-recovery arm keeps the full text and reports something.
    #[test]
    fn error_paths_recover() {
        for text in [
            "standard;",                 // `standard` without `package`
            "part x",                    // missing body at EOF
            "part x % ;",                // junk token in a declaration
            "part x : ;",                // missing type name
            "import *;",                 // wildcard head
            "import A::%;",              // junk after `::`
            "alias a for ;",             // missing alias target
            "attribute a = new ;",       // `new` without type
            "attribute b = (1, 2, );",   // trailing comma sequence
            "attribute c = x-> ;",       // arrow without function name
            "attribute d = a[1, 2];",    // multi-index
            "attribute e = x istype 5;", // classification with non-name
            "attribute f = f(%%);",      // broken argument
            "attribute g = f(redefines x = 1);",
            "attribute h = f(y = 2);",
            "calc sum(in x);", // usage with parameter list
            "action def A (part p : P, q :> r, s :>> t, u ::> v, w[2], k = 1 ordered);",
            "doc",                     // missing comment body
            "part p { @ }",            // annotation without name
            "#;",                      // stray hash
            "part x =",                // value at EOF
            "part y [1",               // unterminated multiplicity
            "part def <s",             // unterminated short name
            "perform action f(in x);", // adapter with parameter list
            "import ;",                // import without a target
            "attribute i = a[];",      // empty index
        ] {
            let parse = parse(text);
            assert_eq!(parse.syntax().text().to_string(), text, "lossless: {text}");
        }
    }

    #[test]
    fn error_recovery_keeps_text_and_reports() {
        let text = "part def { \n junk %% tokens \n part def Ok;";
        let parse = parse(text);
        assert!(!parse.errors().is_empty());
        assert_eq!(parse.syntax().text().to_string(), text);
    }

    #[test]
    fn directions_on_usages() {
        check_ok("port def P { in item x : X; out item y : Y; inout ref part z; }");
    }

    #[test]
    fn expressions() {
        check_ok("attribute a = 1 + 2 * 3;");
        check_ok("attribute b = (x >= 0) and not (y < z);");
        check_ok("attribute c = if x > 0 ? x else -x;");
        check_ok("attribute d = vals->select {in v; v > 0}->size();");
        check_ok("attribute e = 10 [SI::kg];");
        check_ok("attribute f = seq#(1);");
        check_ok("attribute g = a.b.c ** 2;");
        check_ok("attribute h = x istype Vehicle;");
        check_ok("constraint c { mass <= maxMass }");
    }

    #[test]
    fn calc_and_constraint_defs() {
        check_ok("calc def Force { in mass : Real; in acc : Real; mass * acc }");
        check_ok("calc def Sum(a : Real, b : Real) : Real { a + b }");
        check_ok("constraint def MaxMass { actual <= allowed }");
        check_ok(
            "requirement def R { subject v : Vehicle; require constraint { v.mass < 100.0 } }",
        );
    }

    #[test]
    fn states_and_actions() {
        check_ok("state def S { entry; then idle; state idle; transition first idle accept sig then busy; state busy; }");
        check_ok("action def A (in x : X, out y : Y) { first start; then s1; action s1; send x to y via p; }");
        check_ok("part p { perform action a { assign x := x + 1; } exhibit state s parallel { } }");
    }

    #[test]
    fn connections_and_flows() {
        check_ok("part p { connect w.hub to axle; bind a = b; flow of Fuel from tank.fuelOut to eng.fuelIn; }");
        check_ok("part p { message m of Sig from a to b; allocate x to y; }");
        check_ok("interface def I { end p1 : P; end p2 : ~P; }");
    }

    fn check_ok_kerml(text: &str) -> Parse {
        let parse = parse_dialect(text, crate::Dialect::KerML);
        assert_eq!(parse.errors(), &[], "unexpected errors for:\n{text}");
        assert_eq!(parse.syntax().text().to_string(), text, "lossless violated");
        parse
    }

    #[test]
    fn kerml_declarations() {
        check_ok_kerml("classifier Vehicle :> Thing { feature mass : Real; }");
        check_ok_kerml("datatype Real :> ScalarValue;");
        check_ok_kerml("function Plus { in a : Real; in b : Real; return : Real; }");
        check_ok_kerml("assoc struct Owns { end owner : Person; end owned : Thing; }");
        check_ok_kerml("feature f : T chains a.b;");
        check_ok_kerml("classifier A disjoint from B unions C, D;");
        check_ok_kerml("specialization s subtype A specializes B;");
        // SysML-only keywords are plain names in KerML
        check_ok_kerml("feature frame : Frame { feature entry : E; }");
    }

    #[test]
    fn metadata_and_extensions() {
        check_ok("@Safety;");
        check_ok("metadata def Safety { attribute level : Integer; }");
        check_ok("part p { @Safety { level = 3; } }");
        check_ok("#command action doIt;");
        check_ok("filter @Safety;");
    }

    #[test]
    fn short_names_and_quantities() {
        check_ok("attribute def <kg> Kilogram :> MassUnit;");
        check_ok("attribute <A> ampere : Ampere;");
    }

    #[test]
    fn requirements_and_cases() {
        check_ok("requirement r { subject s : System; assume constraint { s.ok } }");
        check_ok("use case def Drive { objective { doc /* drive */ } actor driver : Person; }");
        check_ok("analysis def Fuel { subject v : Vehicle; return fuelEconomy : Real; }");
        check_ok("verification def V { objective verifyMass { verify massReq; } }");
        check_ok("satisfy requirement massReq by vehicle;");
    }
}

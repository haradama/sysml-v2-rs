//! Syntax kinds shared by tokens and tree nodes.

/// One kind for every token and node in the syntax tree.
///
/// Token kinds come first, node kinds after `SOURCE_FILE`. `EOF` is a
/// sentinel used by the parser and never appears in a tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[allow(non_camel_case_types)]
#[repr(u16)]
pub enum SyntaxKind {
    // --- trivia tokens
    WHITESPACE = 0,
    /// `// ...` (to end of line)
    LINE_NOTE,
    /// `//* ... */`
    BLOCK_NOTE,

    // --- literal / name tokens
    /// `/* ... */` — the body of a `doc` or `comment` element (not trivia)
    COMMENT_BODY,
    IDENT,
    /// `'quoted name'`
    UNRESTRICTED_NAME,
    DECIMAL,
    REAL,
    STRING,

    // --- punctuation tokens
    L_BRACE,
    R_BRACE,
    L_PAREN,
    R_PAREN,
    L_BRACKET,
    R_BRACKET,
    SEMICOLON,
    COMMA,
    DOT,
    DOT_DOT,
    /// `.?`
    DOT_QUESTION,
    COLON,
    COLON_COLON,
    /// `:>`
    COLON_GT,
    /// `:>>`
    COLON_GT_GT,
    /// `::>`
    COLON_COLON_GT,
    /// `:=`
    COLON_EQ,
    EQ,
    /// `==`
    EQ_EQ,
    /// `===`
    EQ_EQ_EQ,
    /// `!=`
    NOT_EQ,
    /// `!==`
    NOT_EQ_EQ,
    /// `=>`
    FAT_ARROW,
    /// `->`
    ARROW,
    STAR,
    STAR_STAR,
    PLUS,
    MINUS,
    SLASH,
    PERCENT,
    CARET,
    LT,
    LT_EQ,
    GT,
    GT_EQ,
    AMP,
    PIPE,
    TILDE,
    QUESTION,
    /// `??`
    QUESTION_QUESTION,
    AT,
    /// `@@`
    AT_AT,
    HASH,
    DOLLAR,
    ERROR_TOKEN,

    // --- keyword tokens (contiguous: ABOUT_KW..=XOR_KW)
    ABOUT_KW,
    ABSTRACT_KW,
    ACCEPT_KW,
    ACTION_KW,
    ACTOR_KW,
    AFTER_KW,
    ALIAS_KW,
    ALL_KW,
    ALLOCATE_KW,
    ALLOCATION_KW,
    ANALYSIS_KW,
    AND_KW,
    AS_KW,
    ASSERT_KW,
    ASSIGN_KW,
    ASSOC_KW,
    ASSUME_KW,
    ASSUMPTION_KW,
    AT_KW,
    ATTRIBUTE_KW,
    BEHAVIOR_KW,
    BIND_KW,
    BINDING_KW,
    BOOL_KW,
    BY_KW,
    CALC_KW,
    CASE_KW,
    CHAINS_KW,
    CLASS_KW,
    CLASSIFIER_KW,
    COMMENT_KW,
    COMPOSITE_KW,
    CONCERN_KW,
    CONJUGATE_KW,
    CONJUGATES_KW,
    CONJUGATION_KW,
    CONNECT_KW,
    CONNECTION_KW,
    CONNECTOR_KW,
    CONST_KW,
    CONSTANT_KW,
    CONSTRAINT_KW,
    CROSSES_KW,
    DATATYPE_KW,
    DECIDE_KW,
    DEF_KW,
    DEFAULT_KW,
    DEFINED_KW,
    DEPENDENCY_KW,
    DERIVED_KW,
    DIFFERENCES_KW,
    DISJOINING_KW,
    DISJOINT_KW,
    DO_KW,
    DOC_KW,
    EFFECT_KW,
    ELSE_KW,
    END_KW,
    ENTRY_KW,
    ENUM_KW,
    EVENT_KW,
    EXHIBIT_KW,
    EXIT_KW,
    EXPOSE_KW,
    EXPR_KW,
    FALSE_KW,
    FEATURE_KW,
    FEATURED_KW,
    FEATURING_KW,
    FILTER_KW,
    FIRST_KW,
    FLOW_KW,
    FOR_KW,
    FORK_KW,
    FRAME_KW,
    FROM_KW,
    FUNCTION_KW,
    GUARD_KW,
    HASTYPE_KW,
    IF_KW,
    IMPLIES_KW,
    IMPORT_KW,
    IN_KW,
    INCLUDE_KW,
    INDIVIDUAL_KW,
    INOUT_KW,
    INTERACTION_KW,
    INTERFACE_KW,
    INTERSECTS_KW,
    INV_KW,
    INVERSE_KW,
    INVERTING_KW,
    ISTYPE_KW,
    ITEM_KW,
    JOIN_KW,
    LANGUAGE_KW,
    LIBRARY_KW,
    LOCALE_KW,
    LOOP_KW,
    MEMBER_KW,
    MERGE_KW,
    MESSAGE_KW,
    META_KW,
    METACLASS_KW,
    METADATA_KW,
    MULTIPLICITY_KW,
    NAMESPACE_KW,
    NEW_KW,
    NONUNIQUE_KW,
    NOT_KW,
    NULL_KW,
    OBJECTIVE_KW,
    OCCURRENCE_KW,
    OF_KW,
    OR_KW,
    ORDERED_KW,
    OUT_KW,
    PACKAGE_KW,
    PARALLEL_KW,
    PART_KW,
    PERFORM_KW,
    PORT_KW,
    PORTION_KW,
    PREDICATE_KW,
    PRIVATE_KW,
    PROTECTED_KW,
    PUBLIC_KW,
    REDEFINES_KW,
    REDEFINITION_KW,
    REF_KW,
    REFERENCES_KW,
    RENDER_KW,
    RENDERING_KW,
    REP_KW,
    REQUIRE_KW,
    REQUIREMENT_KW,
    RETURN_KW,
    SATISFY_KW,
    SEND_KW,
    SNAPSHOT_KW,
    SPECIALIZATION_KW,
    SPECIALIZES_KW,
    STAKEHOLDER_KW,
    STANDARD_KW,
    STATE_KW,
    STEP_KW,
    STRUCT_KW,
    SUBCLASSIFIER_KW,
    SUBJECT_KW,
    SUBSET_KW,
    SUBSETS_KW,
    SUBTYPE_KW,
    SUCCESSION_KW,
    TERMINATE_KW,
    THEN_KW,
    TIMESLICE_KW,
    TO_KW,
    TRANSITION_KW,
    TRIGGER_KW,
    TRUE_KW,
    TYPE_KW,
    TYPED_KW,
    TYPING_KW,
    UNIONS_KW,
    UNTIL_KW,
    USE_KW,
    VAR_KW,
    VARIANT_KW,
    VARIATION_KW,
    VERIFICATION_KW,
    VERIFY_KW,
    VIA_KW,
    VIEW_KW,
    VIEWPOINT_KW,
    WHEN_KW,
    WHILE_KW,
    XOR_KW,

    // --- nodes
    SOURCE_FILE,
    PACKAGE,
    /// `{ ... }` or `;`
    BODY,
    IMPORT,
    /// `expose A::*;` (view bodies) — same shape as an import
    EXPOSE,
    ALIAS,
    /// `doc /* ... */`
    DOCUMENTATION,
    /// `comment about X /* ... */`
    COMMENT_ELEM,
    /// `about X, Y`
    ABOUT,
    /// `rep name language "lang" /* ... */`
    REP,
    /// `filter expr;` (also `[expr]` filters on imports)
    FILTER,
    /// `#name` prefix before a declaration
    PREFIX_METADATA,
    /// `@M about x;` / `metadata m : M { ... }`
    METADATA_ANNOTATION,
    /// `part def X ...` / `classifier X ...` (KerML definitions have no `def`)
    DEFINITION,
    /// `part x : X ...`, incl. `perform`/`exhibit`/`subject`/... shorthands
    USAGE,
    /// `( in x : X, out y : Y )` on definitions/usages
    PARAM_LIST,
    /// declared name of an element
    NAME,
    /// `<shortName>`
    SHORT_NAME,
    QUALIFIED_NAME,
    /// a (possibly conjugated `~`) type reference in typings/specializations
    TYPE_REF,
    /// `: T` / `typed by T` / `defined by T`
    TYPING,
    /// `:> f` / `subsets` / `specializes` / `crosses`
    SUBSETTING,
    /// `:>> f` / `redefines f`
    REDEFINITION,
    /// `::> f` / `references f`
    REFERENCES,
    /// KerML relationship parts: `chains`, `disjoint from`, `unions`, ...
    RELATION,
    /// `[ 0..* ]`
    MULTIPLICITY,
    /// `= expr` / `:= expr` / `default expr`
    VALUE,
    /// `connect a to b;`, `bind x = y;`, `message ... from a to b;`, ...
    CONNECTOR_STMT,
    /// `first a then b;`, `if c then t;`, `send x via p;`, `entry; do a;`, ...
    CONTROL_STMT,
    /// KerML: `specialization s subtype A :> B;`, `conjugation ...`, ...
    RELATION_STMT,
    /// `dependency a to b;`
    DEPENDENCY,
    /// expression used directly as a body member (calc results, invariants)
    EXPR_STMT,
    // --- expression nodes
    LITERAL,
    NAME_REF,
    PAREN_EXPR,
    UNARY_EXPR,
    BINARY_EXPR,
    /// `if c ? t else f`
    COND_EXPR,
    /// `f(a, b = 1)`
    CALL_EXPR,
    ARG_LIST,
    /// `a.b.c`
    PATH_EXPR,
    /// `x#(i)` / `10 [SI::kg]`
    INDEX_EXPR,
    /// `list->select {in x; ...}`
    ARROW_EXPR,
    /// `{ ... }` used as an expression body (after `->`)
    BODY_EXPR,
    ERROR,

    /// Sentinel — never stored in a tree.
    EOF,
}

use SyntaxKind::*;

impl SyntaxKind {
    pub fn is_trivia(self) -> bool {
        matches!(self, WHITESPACE | LINE_NOTE | BLOCK_NOTE)
    }

    pub fn is_keyword(self) -> bool {
        (ABOUT_KW..=XOR_KW).contains(&self)
    }

    /// Keywords introducing a definition/usage kind (`part`, `classifier`, ...).
    pub fn is_def_kind_kw(self) -> bool {
        matches!(
            self,
            // SysML definition/usage kinds
            ATTRIBUTE_KW
                | ENUM_KW
                | OCCURRENCE_KW
                | PART_KW
                | ITEM_KW
                | PORT_KW
                | CONNECTION_KW
                | INTERFACE_KW
                | ALLOCATION_KW
                | ACTION_KW
                | CALC_KW
                | STATE_KW
                | CONSTRAINT_KW
                | REQUIREMENT_KW
                | CONCERN_KW
                | CASE_KW
                | ANALYSIS_KW
                | VERIFICATION_KW
                | USE_KW
                | VIEW_KW
                | VIEWPOINT_KW
                | RENDERING_KW
                | METADATA_KW
                | FLOW_KW
                | SUCCESSION_KW
                // KerML classifier kinds
                | TYPE_KW
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
                // KerML feature kinds
                | FEATURE_KW
                | STEP_KW
                | EXPR_KW
                | BOOL_KW
                | INV_KW
                | CONNECTOR_KW
                | BINDING_KW
                | MULTIPLICITY_KW
        )
    }

    /// Prefix modifiers that may precede a definition/usage kind.
    pub fn is_modifier_kw(self) -> bool {
        matches!(
            self,
            ABSTRACT_KW
                | VARIATION_KW
                | VARIANT_KW
                | DERIVED_KW
                | END_KW
                | INDIVIDUAL_KW
                | CONSTANT_KW
                | CONST_KW
                | COMPOSITE_KW
                | PORTION_KW
                | VAR_KW
                | REF_KW
                | IN_KW
                | OUT_KW
                | INOUT_KW
                | SNAPSHOT_KW
                | TIMESLICE_KW
                | MEMBER_KW
        )
    }

    pub fn is_visibility_kw(self) -> bool {
        matches!(self, PUBLIC_KW | PRIVATE_KW | PROTECTED_KW)
    }

    /// Is this keyword reserved in the SysML v2 textual notation?
    /// (Keywords of the other dialect are ordinary identifiers.)
    pub fn is_sysml_keyword(self) -> bool {
        use SyntaxKind::*;
        self.is_keyword()
            && !matches!(
                self,
                // not in the official RESERVED_KEYWORD list of either notation
                ASSUMPTION_KW | EFFECT_KW | GUARD_KW | TRIGGER_KW
                // KerML-only keywords
                | TYPED_KW
                | ASSOC_KW
                    | BEHAVIOR_KW
                    | BOOL_KW
                    | CHAINS_KW
                    | CLASS_KW
                    | CLASSIFIER_KW
                    | COMPOSITE_KW
                    | CONJUGATE_KW
                    | CONJUGATES_KW
                    | CONJUGATION_KW
                    | CONNECTOR_KW
                    | CONST_KW
                    | DATATYPE_KW
                    | DIFFERENCES_KW
                    | DISJOINING_KW
                    | DISJOINT_KW
                    | EXPR_KW
                    | FEATURE_KW
                    | FEATURED_KW
                    | FEATURING_KW
                    | FUNCTION_KW
                    | INTERACTION_KW
                    | INTERSECTS_KW
                    | INV_KW
                    | INVERSE_KW
                    | INVERTING_KW
                    | MEMBER_KW
                    | METACLASS_KW
                    | MULTIPLICITY_KW
                    | NAMESPACE_KW
                    | PORTION_KW
                    | PREDICATE_KW
                    | REDEFINITION_KW
                    | SPECIALIZATION_KW
                    | STEP_KW
                    | STRUCT_KW
                    | SUBCLASSIFIER_KW
                    | SUBSET_KW
                    | SUBTYPE_KW
                    | TYPE_KW
                    | TYPING_KW
                    | UNIONS_KW
                    | VAR_KW
            )
    }

    /// Is this keyword reserved in the KerML textual notation?
    pub fn is_kerml_keyword(self) -> bool {
        use SyntaxKind::*;
        self.is_keyword()
            && !matches!(
                self,
                // not in the official RESERVED_KEYWORD list of either notation
                ASSUMPTION_KW | EFFECT_KW | GUARD_KW | TRIGGER_KW
                // SysML-only keywords
                | NEW_KW
                | UNTIL_KW
                | ACCEPT_KW
                    | ACTION_KW
                    | ACTOR_KW
                    | AFTER_KW
                    | ALLOCATE_KW
                    | ALLOCATION_KW
                    | ANALYSIS_KW
                    | ASSERT_KW
                    | ASSIGN_KW
                    | ASSUME_KW
                    | AT_KW
                    | ATTRIBUTE_KW
                    | BIND_KW
                    | CALC_KW
                    | CASE_KW
                    | CONCERN_KW
                    | CONNECT_KW
                    | CONNECTION_KW
                    | CONSTANT_KW
                    | CONSTRAINT_KW
                    | DECIDE_KW
                    | DEF_KW
                    | DEFINED_KW
                    | DO_KW
                    | ENTRY_KW
                    | ENUM_KW
                    | EVENT_KW
                    | EXHIBIT_KW
                    | EXIT_KW
                    | EXPOSE_KW
                    | FORK_KW
                    | FRAME_KW
                    | INCLUDE_KW
                    | INDIVIDUAL_KW
                    | INTERFACE_KW
                    | ITEM_KW
                    | JOIN_KW
                    | LOOP_KW
                    | MERGE_KW
                    | MESSAGE_KW
                    | OBJECTIVE_KW
                    | OCCURRENCE_KW
                    | PARALLEL_KW
                    | PART_KW
                    | PERFORM_KW
                    | PORT_KW
                    | REF_KW
                    | RENDER_KW
                    | RENDERING_KW
                    | REQUIRE_KW
                    | REQUIREMENT_KW
                    | SATISFY_KW
                    | SEND_KW
                    | SNAPSHOT_KW
                    | STAKEHOLDER_KW
                    | STATE_KW
                    | SUBJECT_KW
                    | TERMINATE_KW
                    | TIMESLICE_KW
                    | TRANSITION_KW
                    | USE_KW
                    | VARIANT_KW
                    | VARIATION_KW
                    | VERIFICATION_KW
                    | VERIFY_KW
                    | VIA_KW
                    | VIEW_KW
                    | VIEWPOINT_KW
                    | WHEN_KW
                    | WHILE_KW
            )
    }

    pub fn from_keyword(ident: &str) -> Option<SyntaxKind> {
        let kw = match ident {
            "about" => ABOUT_KW,
            "abstract" => ABSTRACT_KW,
            "accept" => ACCEPT_KW,
            "action" => ACTION_KW,
            "actor" => ACTOR_KW,
            "after" => AFTER_KW,
            "alias" => ALIAS_KW,
            "all" => ALL_KW,
            "allocate" => ALLOCATE_KW,
            "allocation" => ALLOCATION_KW,
            "analysis" => ANALYSIS_KW,
            "and" => AND_KW,
            "as" => AS_KW,
            "assert" => ASSERT_KW,
            "assign" => ASSIGN_KW,
            "assoc" => ASSOC_KW,
            "assume" => ASSUME_KW,
            "assumption" => ASSUMPTION_KW,
            "at" => AT_KW,
            "attribute" => ATTRIBUTE_KW,
            "behavior" => BEHAVIOR_KW,
            "bind" => BIND_KW,
            "binding" => BINDING_KW,
            "bool" => BOOL_KW,
            "by" => BY_KW,
            "calc" => CALC_KW,
            "case" => CASE_KW,
            "chains" => CHAINS_KW,
            "class" => CLASS_KW,
            "classifier" => CLASSIFIER_KW,
            "comment" => COMMENT_KW,
            "composite" => COMPOSITE_KW,
            "concern" => CONCERN_KW,
            "conjugate" => CONJUGATE_KW,
            "conjugates" => CONJUGATES_KW,
            "conjugation" => CONJUGATION_KW,
            "connect" => CONNECT_KW,
            "connection" => CONNECTION_KW,
            "connector" => CONNECTOR_KW,
            "const" => CONST_KW,
            "constant" => CONSTANT_KW,
            "constraint" => CONSTRAINT_KW,
            "crosses" => CROSSES_KW,
            "datatype" => DATATYPE_KW,
            "decide" => DECIDE_KW,
            "def" => DEF_KW,
            "default" => DEFAULT_KW,
            "defined" => DEFINED_KW,
            "dependency" => DEPENDENCY_KW,
            "derived" => DERIVED_KW,
            "differences" => DIFFERENCES_KW,
            "disjoining" => DISJOINING_KW,
            "disjoint" => DISJOINT_KW,
            "do" => DO_KW,
            "doc" => DOC_KW,
            "effect" => EFFECT_KW,
            "else" => ELSE_KW,
            "end" => END_KW,
            "entry" => ENTRY_KW,
            "enum" => ENUM_KW,
            "event" => EVENT_KW,
            "exhibit" => EXHIBIT_KW,
            "exit" => EXIT_KW,
            "expose" => EXPOSE_KW,
            "expr" => EXPR_KW,
            "false" => FALSE_KW,
            "feature" => FEATURE_KW,
            "featured" => FEATURED_KW,
            "featuring" => FEATURING_KW,
            "filter" => FILTER_KW,
            "first" => FIRST_KW,
            "flow" => FLOW_KW,
            "for" => FOR_KW,
            "fork" => FORK_KW,
            "frame" => FRAME_KW,
            "from" => FROM_KW,
            "function" => FUNCTION_KW,
            "guard" => GUARD_KW,
            "hastype" => HASTYPE_KW,
            "if" => IF_KW,
            "implies" => IMPLIES_KW,
            "import" => IMPORT_KW,
            "in" => IN_KW,
            "include" => INCLUDE_KW,
            "individual" => INDIVIDUAL_KW,
            "inout" => INOUT_KW,
            "interaction" => INTERACTION_KW,
            "interface" => INTERFACE_KW,
            "intersects" => INTERSECTS_KW,
            "inv" => INV_KW,
            "inverse" => INVERSE_KW,
            "inverting" => INVERTING_KW,
            "istype" => ISTYPE_KW,
            "item" => ITEM_KW,
            "join" => JOIN_KW,
            "language" => LANGUAGE_KW,
            "library" => LIBRARY_KW,
            "locale" => LOCALE_KW,
            "loop" => LOOP_KW,
            "member" => MEMBER_KW,
            "merge" => MERGE_KW,
            "message" => MESSAGE_KW,
            "meta" => META_KW,
            "metaclass" => METACLASS_KW,
            "metadata" => METADATA_KW,
            "multiplicity" => MULTIPLICITY_KW,
            "namespace" => NAMESPACE_KW,
            "new" => NEW_KW,
            "nonunique" => NONUNIQUE_KW,
            "not" => NOT_KW,
            "null" => NULL_KW,
            "objective" => OBJECTIVE_KW,
            "occurrence" => OCCURRENCE_KW,
            "of" => OF_KW,
            "or" => OR_KW,
            "ordered" => ORDERED_KW,
            "out" => OUT_KW,
            "package" => PACKAGE_KW,
            "parallel" => PARALLEL_KW,
            "part" => PART_KW,
            "perform" => PERFORM_KW,
            "port" => PORT_KW,
            "portion" => PORTION_KW,
            "predicate" => PREDICATE_KW,
            "private" => PRIVATE_KW,
            "protected" => PROTECTED_KW,
            "public" => PUBLIC_KW,
            "redefines" => REDEFINES_KW,
            "redefinition" => REDEFINITION_KW,
            "ref" => REF_KW,
            "references" => REFERENCES_KW,
            "render" => RENDER_KW,
            "rendering" => RENDERING_KW,
            "rep" => REP_KW,
            "require" => REQUIRE_KW,
            "requirement" => REQUIREMENT_KW,
            "return" => RETURN_KW,
            "satisfy" => SATISFY_KW,
            "send" => SEND_KW,
            "snapshot" => SNAPSHOT_KW,
            "specialization" => SPECIALIZATION_KW,
            "specializes" => SPECIALIZES_KW,
            "stakeholder" => STAKEHOLDER_KW,
            "standard" => STANDARD_KW,
            "state" => STATE_KW,
            "step" => STEP_KW,
            "struct" => STRUCT_KW,
            "subclassifier" => SUBCLASSIFIER_KW,
            "subject" => SUBJECT_KW,
            "subset" => SUBSET_KW,
            "subsets" => SUBSETS_KW,
            "subtype" => SUBTYPE_KW,
            "succession" => SUCCESSION_KW,
            "terminate" => TERMINATE_KW,
            "then" => THEN_KW,
            "timeslice" => TIMESLICE_KW,
            "to" => TO_KW,
            "transition" => TRANSITION_KW,
            "trigger" => TRIGGER_KW,
            "true" => TRUE_KW,
            "type" => TYPE_KW,
            "typed" => TYPED_KW,
            "typing" => TYPING_KW,
            "unions" => UNIONS_KW,
            "until" => UNTIL_KW,
            "use" => USE_KW,
            "var" => VAR_KW,
            "variant" => VARIANT_KW,
            "variation" => VARIATION_KW,
            "verification" => VERIFICATION_KW,
            "verify" => VERIFY_KW,
            "via" => VIA_KW,
            "view" => VIEW_KW,
            "viewpoint" => VIEWPOINT_KW,
            "when" => WHEN_KW,
            "while" => WHILE_KW,
            "xor" => XOR_KW,
            _ => return None,
        };
        Some(kw)
    }
}

/// The rowan [`Language`](rowan::Language) implementation for SysML v2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SysMLLanguage {}

impl rowan::Language for SysMLLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> SyntaxKind {
        assert!(raw.0 <= EOF as u16);
        // SAFETY: SyntaxKind is #[repr(u16)] with contiguous discriminants
        // starting at 0, and the assert above bounds-checks the value.
        unsafe { std::mem::transmute::<u16, SyntaxKind>(raw.0) }
    }

    fn kind_to_raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        rowan::SyntaxKind(kind as u16)
    }
}

impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> Self {
        rowan::SyntaxKind(kind as u16)
    }
}

pub type SyntaxNode = rowan::SyntaxNode<SysMLLanguage>;
pub type SyntaxToken = rowan::SyntaxToken<SysMLLanguage>;
pub type SyntaxElement = rowan::SyntaxElement<SysMLLanguage>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_kind_round_trip() {
        for kind in [WHITESPACE, IDENT, PART_KW, SOURCE_FILE, ERROR, EOF] {
            let raw = <SysMLLanguage as rowan::Language>::kind_to_raw(kind);
            assert_eq!(<SysMLLanguage as rowan::Language>::kind_from_raw(raw), kind);
        }
        assert!(PART_KW.is_keyword());
        assert!(!IDENT.is_keyword());
    }
}

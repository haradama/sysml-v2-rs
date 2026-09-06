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
    /// `of Fuel` -- what a flow, succession flow or message carries.
    /// `FlowDeclaration : FlowUsage = ... ( 'of' ownedRelationship +=
    /// FlowPayloadFeatureMember )? ...`
    PAYLOAD,
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
        matches!(self.reserved_in(), Reserved::Both | Reserved::SysML)
    }

    /// Is this keyword reserved in the KerML textual notation?
    pub fn is_kerml_keyword(self) -> bool {
        matches!(self.reserved_in(), Reserved::Both | Reserved::KerML)
    }

    /// Which notations reserve this kind, `Neither` for anything that is
    /// not a keyword at all.
    fn reserved_in(self) -> Reserved {
        RESERVED_IN[self as usize]
    }

    pub fn from_keyword(ident: &str) -> Option<SyntaxKind> {
        KEYWORDS
            .binary_search_by_key(&ident, |(text, _, _)| text)
            .ok()
            .map(|found| KEYWORDS[found].1)
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

/// Which of the two notations reserve a keyword.
///
/// A keyword of one notation is an ordinary identifier in the other:
/// `frame` is a name in KerML, `step` is a name in SysML. Four words are
/// reserved by neither, because the specification's grammar spells them
/// inline (`{ kind = 'guard' }`) rather than reserving them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reserved {
    Both,
    SysML,
    KerML,
    Neither,
}

/// What each kind is reserved in, indexed by the kind itself.
///
/// Built from [`KEYWORDS`] at compile time. The two dialect predicates
/// used to be hand-kept lists of what each notation does *not* reserve,
/// so a keyword added to one list and forgotten in the other silently
/// became a name in a notation that reserves it.
const RESERVED_IN: [Reserved; SyntaxKind::EOF as usize + 1] = {
    let mut table = [Reserved::Neither; SyntaxKind::EOF as usize + 1];
    let mut i = 0;
    while i < KEYWORDS.len() {
        table[KEYWORDS[i].1 as usize] = KEYWORDS[i].2;
        i += 1;
    }
    table
};

/// Every keyword of the two textual notations with its token and the
/// notations that reserve it, in alphabetical order -- the single table
/// the lexer, the completion list, the two dialect predicates and the
/// conformance tests against the specification's BNF all read.
///
/// Four of these are reserved by neither notation (`assumption`,
/// `effect`, `guard`, `trigger`): the parser still accepts them as plain
/// names, and `tests/bnf_keywords.rs` holds it to that.
pub const KEYWORDS: &[(&str, SyntaxKind, Reserved)] = &[
    ("about", ABOUT_KW, Reserved::Both),
    ("abstract", ABSTRACT_KW, Reserved::Both),
    ("accept", ACCEPT_KW, Reserved::SysML),
    ("action", ACTION_KW, Reserved::SysML),
    ("actor", ACTOR_KW, Reserved::SysML),
    ("after", AFTER_KW, Reserved::SysML),
    ("alias", ALIAS_KW, Reserved::Both),
    ("all", ALL_KW, Reserved::Both),
    ("allocate", ALLOCATE_KW, Reserved::SysML),
    ("allocation", ALLOCATION_KW, Reserved::SysML),
    ("analysis", ANALYSIS_KW, Reserved::SysML),
    ("and", AND_KW, Reserved::Both),
    ("as", AS_KW, Reserved::Both),
    ("assert", ASSERT_KW, Reserved::SysML),
    ("assign", ASSIGN_KW, Reserved::SysML),
    ("assoc", ASSOC_KW, Reserved::KerML),
    ("assume", ASSUME_KW, Reserved::SysML),
    ("assumption", ASSUMPTION_KW, Reserved::Neither),
    ("at", AT_KW, Reserved::SysML),
    ("attribute", ATTRIBUTE_KW, Reserved::SysML),
    ("behavior", BEHAVIOR_KW, Reserved::KerML),
    ("bind", BIND_KW, Reserved::SysML),
    ("binding", BINDING_KW, Reserved::Both),
    ("bool", BOOL_KW, Reserved::KerML),
    ("by", BY_KW, Reserved::Both),
    ("calc", CALC_KW, Reserved::SysML),
    ("case", CASE_KW, Reserved::SysML),
    ("chains", CHAINS_KW, Reserved::KerML),
    ("class", CLASS_KW, Reserved::KerML),
    ("classifier", CLASSIFIER_KW, Reserved::KerML),
    ("comment", COMMENT_KW, Reserved::Both),
    ("composite", COMPOSITE_KW, Reserved::KerML),
    ("concern", CONCERN_KW, Reserved::SysML),
    ("conjugate", CONJUGATE_KW, Reserved::KerML),
    ("conjugates", CONJUGATES_KW, Reserved::KerML),
    ("conjugation", CONJUGATION_KW, Reserved::KerML),
    ("connect", CONNECT_KW, Reserved::SysML),
    ("connection", CONNECTION_KW, Reserved::SysML),
    ("connector", CONNECTOR_KW, Reserved::KerML),
    ("const", CONST_KW, Reserved::KerML),
    ("constant", CONSTANT_KW, Reserved::SysML),
    ("constraint", CONSTRAINT_KW, Reserved::SysML),
    ("crosses", CROSSES_KW, Reserved::Both),
    ("datatype", DATATYPE_KW, Reserved::KerML),
    ("decide", DECIDE_KW, Reserved::SysML),
    ("def", DEF_KW, Reserved::SysML),
    ("default", DEFAULT_KW, Reserved::Both),
    ("defined", DEFINED_KW, Reserved::SysML),
    ("dependency", DEPENDENCY_KW, Reserved::Both),
    ("derived", DERIVED_KW, Reserved::Both),
    ("differences", DIFFERENCES_KW, Reserved::KerML),
    ("disjoining", DISJOINING_KW, Reserved::KerML),
    ("disjoint", DISJOINT_KW, Reserved::KerML),
    ("do", DO_KW, Reserved::SysML),
    ("doc", DOC_KW, Reserved::Both),
    ("effect", EFFECT_KW, Reserved::Neither),
    ("else", ELSE_KW, Reserved::Both),
    ("end", END_KW, Reserved::Both),
    ("entry", ENTRY_KW, Reserved::SysML),
    ("enum", ENUM_KW, Reserved::SysML),
    ("event", EVENT_KW, Reserved::SysML),
    ("exhibit", EXHIBIT_KW, Reserved::SysML),
    ("exit", EXIT_KW, Reserved::SysML),
    ("expose", EXPOSE_KW, Reserved::SysML),
    ("expr", EXPR_KW, Reserved::KerML),
    ("false", FALSE_KW, Reserved::Both),
    ("feature", FEATURE_KW, Reserved::KerML),
    ("featured", FEATURED_KW, Reserved::KerML),
    ("featuring", FEATURING_KW, Reserved::KerML),
    ("filter", FILTER_KW, Reserved::Both),
    ("first", FIRST_KW, Reserved::Both),
    ("flow", FLOW_KW, Reserved::Both),
    ("for", FOR_KW, Reserved::Both),
    ("fork", FORK_KW, Reserved::SysML),
    ("frame", FRAME_KW, Reserved::SysML),
    ("from", FROM_KW, Reserved::Both),
    ("function", FUNCTION_KW, Reserved::KerML),
    ("guard", GUARD_KW, Reserved::Neither),
    ("hastype", HASTYPE_KW, Reserved::Both),
    ("if", IF_KW, Reserved::Both),
    ("implies", IMPLIES_KW, Reserved::Both),
    ("import", IMPORT_KW, Reserved::Both),
    ("in", IN_KW, Reserved::Both),
    ("include", INCLUDE_KW, Reserved::SysML),
    ("individual", INDIVIDUAL_KW, Reserved::SysML),
    ("inout", INOUT_KW, Reserved::Both),
    ("interaction", INTERACTION_KW, Reserved::KerML),
    ("interface", INTERFACE_KW, Reserved::SysML),
    ("intersects", INTERSECTS_KW, Reserved::KerML),
    ("inv", INV_KW, Reserved::KerML),
    ("inverse", INVERSE_KW, Reserved::KerML),
    ("inverting", INVERTING_KW, Reserved::KerML),
    ("istype", ISTYPE_KW, Reserved::Both),
    ("item", ITEM_KW, Reserved::SysML),
    ("join", JOIN_KW, Reserved::SysML),
    ("language", LANGUAGE_KW, Reserved::Both),
    ("library", LIBRARY_KW, Reserved::Both),
    ("locale", LOCALE_KW, Reserved::Both),
    ("loop", LOOP_KW, Reserved::SysML),
    ("member", MEMBER_KW, Reserved::KerML),
    ("merge", MERGE_KW, Reserved::SysML),
    ("message", MESSAGE_KW, Reserved::SysML),
    ("meta", META_KW, Reserved::Both),
    ("metaclass", METACLASS_KW, Reserved::KerML),
    ("metadata", METADATA_KW, Reserved::Both),
    ("multiplicity", MULTIPLICITY_KW, Reserved::KerML),
    ("namespace", NAMESPACE_KW, Reserved::KerML),
    ("new", NEW_KW, Reserved::Both),
    ("nonunique", NONUNIQUE_KW, Reserved::Both),
    ("not", NOT_KW, Reserved::Both),
    ("null", NULL_KW, Reserved::Both),
    ("objective", OBJECTIVE_KW, Reserved::SysML),
    ("occurrence", OCCURRENCE_KW, Reserved::SysML),
    ("of", OF_KW, Reserved::Both),
    ("or", OR_KW, Reserved::Both),
    ("ordered", ORDERED_KW, Reserved::Both),
    ("out", OUT_KW, Reserved::Both),
    ("package", PACKAGE_KW, Reserved::Both),
    ("parallel", PARALLEL_KW, Reserved::SysML),
    ("part", PART_KW, Reserved::SysML),
    ("perform", PERFORM_KW, Reserved::SysML),
    ("port", PORT_KW, Reserved::SysML),
    ("portion", PORTION_KW, Reserved::KerML),
    ("predicate", PREDICATE_KW, Reserved::KerML),
    ("private", PRIVATE_KW, Reserved::Both),
    ("protected", PROTECTED_KW, Reserved::Both),
    ("public", PUBLIC_KW, Reserved::Both),
    ("redefines", REDEFINES_KW, Reserved::Both),
    ("redefinition", REDEFINITION_KW, Reserved::KerML),
    ("ref", REF_KW, Reserved::SysML),
    ("references", REFERENCES_KW, Reserved::Both),
    ("render", RENDER_KW, Reserved::SysML),
    ("rendering", RENDERING_KW, Reserved::SysML),
    ("rep", REP_KW, Reserved::Both),
    ("require", REQUIRE_KW, Reserved::SysML),
    ("requirement", REQUIREMENT_KW, Reserved::SysML),
    ("return", RETURN_KW, Reserved::Both),
    ("satisfy", SATISFY_KW, Reserved::SysML),
    ("send", SEND_KW, Reserved::SysML),
    ("snapshot", SNAPSHOT_KW, Reserved::SysML),
    ("specialization", SPECIALIZATION_KW, Reserved::KerML),
    ("specializes", SPECIALIZES_KW, Reserved::Both),
    ("stakeholder", STAKEHOLDER_KW, Reserved::SysML),
    ("standard", STANDARD_KW, Reserved::Both),
    ("state", STATE_KW, Reserved::SysML),
    ("step", STEP_KW, Reserved::KerML),
    ("struct", STRUCT_KW, Reserved::KerML),
    ("subclassifier", SUBCLASSIFIER_KW, Reserved::KerML),
    ("subject", SUBJECT_KW, Reserved::SysML),
    ("subset", SUBSET_KW, Reserved::KerML),
    ("subsets", SUBSETS_KW, Reserved::Both),
    ("subtype", SUBTYPE_KW, Reserved::KerML),
    ("succession", SUCCESSION_KW, Reserved::Both),
    ("terminate", TERMINATE_KW, Reserved::SysML),
    ("then", THEN_KW, Reserved::Both),
    ("timeslice", TIMESLICE_KW, Reserved::SysML),
    ("to", TO_KW, Reserved::Both),
    ("transition", TRANSITION_KW, Reserved::SysML),
    ("trigger", TRIGGER_KW, Reserved::Neither),
    ("true", TRUE_KW, Reserved::Both),
    ("type", TYPE_KW, Reserved::KerML),
    ("typed", TYPED_KW, Reserved::KerML),
    ("typing", TYPING_KW, Reserved::KerML),
    ("unions", UNIONS_KW, Reserved::KerML),
    ("until", UNTIL_KW, Reserved::SysML),
    ("use", USE_KW, Reserved::SysML),
    ("var", VAR_KW, Reserved::KerML),
    ("variant", VARIANT_KW, Reserved::SysML),
    ("variation", VARIATION_KW, Reserved::SysML),
    ("verification", VERIFICATION_KW, Reserved::SysML),
    ("verify", VERIFY_KW, Reserved::SysML),
    ("via", VIA_KW, Reserved::SysML),
    ("view", VIEW_KW, Reserved::SysML),
    ("viewpoint", VIEWPOINT_KW, Reserved::SysML),
    ("when", WHEN_KW, Reserved::SysML),
    ("while", WHILE_KW, Reserved::SysML),
    ("xor", XOR_KW, Reserved::Both),
];

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

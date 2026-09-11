//! What the notation writes, read off the syntax tree.
//!
//! Every function here takes a syntax node and answers a question about
//! what was written: which parts of a declaration name a target and where
//! each segment is, which operands of a `connect` are its ends, which
//! library function an operator stands for. None of them touches the
//! model.
//!
//! They were written beside the resolver, which left one file holding both
//! the reading of the notation and the working out of what it means, the
//! two told apart only by whether the first argument was a `SyntaxNode`.

use sysml_model::ElementKind;
use sysml_syntax::{is_name_chain, SyntaxKind, SyntaxNode, TextRange};

/// The `TYPING`/`SUBSETTING`/`REDEFINITION`/`REFERENCES` parts of a
/// definition or usage node, with the name segments and range of each target.
#[allow(clippy::type_complexity)]
pub(crate) struct Target {
    pub(crate) segments: Vec<String>,
    /// whole qualified-name range
    pub(crate) range: TextRange,
    /// range of the final name segment (what a rename must replace)
    pub(crate) name_range: TextRange,
    /// range of each segment, in order -- the earlier ones name
    /// something too
    pub(crate) at: Vec<TextRange>,
    /// the segment depths a chained step ends at, in order
    pub(crate) chain: Vec<usize>,
}

/// Whether a member of a connector's parenthesised list is one of the
/// ends it relates.
///
/// `connect ( causeA, causeB )` writes each as a plain name;
/// `connector ps : P ([1] myCart, [0..1] products)` counts what each
/// relates in front of it, which the parser reads as a declaration. An
/// end that says what it refers to with `::>` is the declaration
/// itself, and is read where the connector's members are.
pub(crate) fn listed_end(child: &SyntaxNode) -> bool {
    matches!(child.kind(), SyntaxKind::NAME_REF | SyntaxKind::PATH_EXPR)
        || child.kind() == SyntaxKind::USAGE
            && !child
                .children()
                .any(|it| it.kind() == SyntaxKind::REFERENCES)
}

/// Whether the name after `connector` is the end it runs from.
///
/// KerML writes a connector's declaration only in front of a `from`, so
/// `connector eng to tanks.main1;` names no connector: it relates `eng`
/// to `tanks.main1`. The n-ary form writes its ends in parentheses and
/// may be named without one, so the `to` is what tells them apart.
pub(crate) fn names_an_end(node: &SyntaxNode) -> bool {
    let has = |wanted| {
        node.children_with_tokens()
            .filter_map(|it| it.into_token())
            .any(|it| it.kind() == wanted)
    };
    has(SyntaxKind::CONNECTOR_KW) && has(SyntaxKind::TO_KW) && !has(SyntaxKind::FROM_KW)
        || has(SyntaxKind::BINDING_KW)
            && !has(SyntaxKind::BIND_KW)
            && !has(SyntaxKind::OF_KW)
            && node.children().any(|it| it.kind() == SyntaxKind::VALUE)
}

/// Whether the `=` in a statement writes a connector end rather than a
/// value.
pub(crate) fn binds_an_end(node: &SyntaxNode) -> bool {
    node.children_with_tokens()
        .filter_map(|it| it.into_token())
        .any(|it| matches!(it.kind(), SyntaxKind::BINDING_KW | SyntaxKind::BIND_KW))
}

/// The two ends a `binding` binds.
///
/// SysML writes `binding [1] bind [0..*] base.edges = [0..*] be;` and
/// KerML `binding ab of a = b;` or `binding a = b;`, and in every one of
/// them a declaration stands only in front of the keyword that
/// introduces the first end. So without a `bind` or an `of` the
/// reference after `binding` is that end. The `=` takes the other,
/// which the parser keeps inside the value clause where nothing follows
/// it and beside the clause where a multiplicity does.
pub(crate) fn binding_operands(node: &SyntaxNode) -> Vec<SyntaxNode> {
    let is_end = |kind| {
        matches!(
            kind,
            SyntaxKind::NAME_REF | SyntaxKind::PATH_EXPR | SyntaxKind::NAME | SyntaxKind::TYPE_REF
        )
    };
    let mut out = Vec::new();
    let mut taking = names_an_end(node);
    for element in node.children_with_tokens() {
        match element {
            sysml_syntax::SyntaxElement::Token(token) => {
                if matches!(token.kind(), SyntaxKind::BIND_KW | SyntaxKind::OF_KW) {
                    taking = true;
                }
            }
            sysml_syntax::SyntaxElement::Node(child) => match child.kind() {
                // `bind [0..*] base.edges` counts the end before naming
                // it, and the count is not what the keyword introduced
                SyntaxKind::MULTIPLICITY => {}
                SyntaxKind::VALUE => {
                    out.extend(child.children().filter(|it| is_end(it.kind())));
                    taking = true;
                }
                kind if taking && is_end(kind) => {
                    out.push(child);
                    taking = false;
                }
                _ => {}
            },
        }
    }
    out
}

/// Whether a declaration was written as a member of its owner rather
/// than as a feature of it -- `member feature inCart;`, or the cross
/// feature standing between an `end` and the declaration after it.
pub(crate) fn written_as_member(node: &SyntaxNode) -> bool {
    let tokens = || {
        node.children_with_tokens()
            .filter_map(|it| it.into_token())
            .map(|it| it.kind())
    };
    tokens().any(|kind| kind == SyntaxKind::MEMBER_KW)
        || (tokens().any(|kind| kind == SyntaxKind::END_KW)
            && node
                .children()
                .any(|it| matches!(it.kind(), SyntaxKind::DEFINITION | SyntaxKind::USAGE)))
}

/// The segment depths at which a chained step ends.
///
/// `cart::product_account.inCart` names two features and not three:
/// `::` qualifies one name, and `.` steps from one feature to the next.
/// A name with no dot in it is one step, which is no chain at all --
/// the standard gives a feature either no chaining features or more
/// than one.
pub(crate) fn chain_steps(qname: &SyntaxNode) -> Vec<usize> {
    let mut steps = Vec::new();
    let mut at = 0;
    for token in qname.children_with_tokens().filter_map(|e| e.into_token()) {
        match token.kind() {
            SyntaxKind::IDENT
            | SyntaxKind::UNRESTRICTED_NAME
            | SyntaxKind::DOLLAR
            | SyntaxKind::STAR
            | SyntaxKind::STAR_STAR => at += 1,
            SyntaxKind::DOT => steps.push(at),
            _ => {}
        }
    }
    steps.push(at);
    steps
}

/// Where the `.`s fall in a reference operand, as depths into what
/// [`operand_segments`] read off it.
///
/// `merge::TakePicture_snapshots.merge` is a chain of two -- the feature
/// `merge::TakePicture_snapshots`, then `merge` within it -- while
/// `a::b::c` is one name. Counted a segment at a time the chain gains a
/// step the notation never wrote, and
/// `validateFeatureChainingFeatureConformance` then asks whether the
/// second is featured within a first that is only half a name.
pub(crate) fn operand_chain_steps(operand: &SyntaxNode) -> Vec<usize> {
    let mut steps = Vec::new();
    let mut at = 0;
    for token in operand_name_tokens(operand) {
        match token.kind() {
            SyntaxKind::IDENT | SyntaxKind::UNRESTRICTED_NAME | SyntaxKind::DOLLAR => at += 1,
            SyntaxKind::DOT => steps.push(at),
            _ => {}
        }
    }
    steps.push(at);
    steps
}

/// The tokens of a connector operand that spell the name it writes.
///
/// An end may carry a multiplicity of its own -- `connector ps :
/// ProductSelection ([0..*] myCart, ...)` -- and what is written inside
/// the brackets is a bound and not a step of the name. Counted as one,
/// the `*` of `[0..*]` made the steps say two where the name had one
/// segment, and reading a prefix of that name went off the end of it.
pub(crate) fn operand_name_tokens(
    operand: &SyntaxNode,
) -> impl Iterator<Item = sysml_syntax::SyntaxToken> {
    operand
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|token| {
            !token
                .parent_ancestors()
                .any(|up| up.kind() == SyntaxKind::MULTIPLICITY)
        })
}

/// The function a trigger invokes, which its kind alone says.
///
/// "Return one of the Functions TriggerWhen, TriggerAt or TriggerAfter,
/// from the Kernel Semantic Library Triggers package, depending on
/// whether the kind of this TriggerInvocationExpression is when, at or
/// after, respectively."
pub(crate) fn triggered_function(kind: &str) -> Option<&'static str> {
    match kind {
        "when" => Some("Triggers::TriggerWhen"),
        "at" => Some("Triggers::TriggerAt"),
        "after" => Some("Triggers::TriggerAfter"),
        _ => None,
    }
}

/// The specification's own operator table, both columns: which Kernel
/// Function Library function each symbol invokes, and whether that
/// function can be evaluated at model level.
///
/// The library carries the second nowhere.
/// `Function::isModelLevelEvaluable` is derived, the metamodel states no
/// derivation, and no library function writes it -- so read off the model
/// it is false of every function, and every constraint asking whether an
/// expression can be evaluated says no. Tables 5 and 7 say it plainly, and
/// only three cannot: `all` is a type extent, and `~` and `[` are
/// undefined.
///
/// The pilot implementation agrees symbol for symbol without stating the
/// column: its registry holds exactly the thirty-six the tables mark
/// "Yes". The library writes the names in quotes, and the model holds what
/// they answer to; `^` and `**` are one function written two ways.
pub(crate) const OPERATORS: [(&str, &str, bool); 39] = [
    ("all", "BaseFunctions::all", false),
    ("istype", "BaseFunctions::istype", true),
    ("hastype", "BaseFunctions::hastype", true),
    ("@", "BaseFunctions::@", true),
    ("@@", "BaseFunctions::@@", true),
    ("as", "BaseFunctions::as", true),
    ("meta", "BaseFunctions::meta", true),
    ("==", "BaseFunctions::==", true),
    ("!=", "BaseFunctions::!=", true),
    ("===", "BaseFunctions::===", true),
    ("!==", "BaseFunctions::!==", true),
    ("[", "BaseFunctions::[", false),
    ("#", "BaseFunctions::#", true),
    (",", "BaseFunctions::,", true),
    (".", "ControlFunctions::.", true),
    ("if", "ControlFunctions::if", true),
    ("??", "ControlFunctions::??", true),
    ("and", "ControlFunctions::and", true),
    ("or", "ControlFunctions::or", true),
    ("implies", "ControlFunctions::implies", true),
    ("collect", "ControlFunctions::collect", true),
    ("select", "ControlFunctions::select", true),
    ("xor", "DataFunctions::xor", true),
    ("not", "DataFunctions::not", true),
    ("~", "DataFunctions::~", false),
    ("|", "DataFunctions::|", true),
    ("&", "DataFunctions::&", true),
    ("<", "DataFunctions::<", true),
    (">", "DataFunctions::>", true),
    ("<=", "DataFunctions::<=", true),
    (">=", "DataFunctions::>=", true),
    ("+", "DataFunctions::+", true),
    ("-", "DataFunctions::-", true),
    ("*", "DataFunctions::*", true),
    ("/", "DataFunctions::/", true),
    ("%", "DataFunctions::%", true),
    ("^", "DataFunctions::^", true),
    ("**", "DataFunctions::^", true),
    ("..", "DataFunctions::..", true),
];

/// The library function an operator symbol invokes.
pub(crate) fn invoked_function(operator: &str) -> Option<&'static str> {
    OPERATORS
        .iter()
        .find(|(symbol, ..)| *symbol == operator)
        .map(|(_, named, _)| *named)
}

/// Whether the library function of that name can be evaluated at model
/// level, where the specification's table says so.
pub(crate) fn evaluable_at_model_level(qualified: &str) -> Option<bool> {
    OPERATORS
        .iter()
        .find(|(_, named, _)| *named == qualified)
        .map(|(.., evaluable)| *evaluable)
}

/// The name an invocation writes after an arrow.
///
/// `xs->minimize{ ... }` invokes `minimize` and hands it what stands in
/// front of the arrow, so the name is a token of the expression rather
/// than a node under it.
pub(crate) fn invoked_through_arrow(node: &SyntaxNode) -> Option<String> {
    if node.kind() != SyntaxKind::ARROW_EXPR {
        return None;
    }
    node.children_with_tokens()
        .filter_map(|it| it.into_token())
        .find(|token| {
            matches!(
                token.kind(),
                SyntaxKind::IDENT | SyntaxKind::UNRESTRICTED_NAME
            )
        })
        .map(|token| token.text().to_string())
}

/// The name an invocation writes in front of its arguments.
///
/// `new Foo(1)` writes `new` in front of the name, and what it
/// constructs is the name rather than the keyword.
pub(crate) fn invoked_by_name(node: &SyntaxNode) -> Option<SyntaxNode> {
    let callee = node.children().next()?;
    match callee.kind() {
        SyntaxKind::UNARY_EXPR => callee.children().next(),
        _ => Some(callee),
    }
}

/// The range of the last identifier in a reference operand -- what a
/// rename of the thing it names rewrites, as opposed to the whole `a.b`.
pub(crate) fn last_name_range(operand: &SyntaxNode) -> TextRange {
    operand
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| matches!(t.kind(), SyntaxKind::IDENT | SyntaxKind::UNRESTRICTED_NAME))
        .last()
        .map(|t| t.text_range())
        .unwrap_or_else(|| operand.text_range())
}

/// Is there a `from` in this statement that `operand` stands before?
/// That name is the dependency's own, not one of its clients.
pub(crate) fn before_from(node: &SyntaxNode, operand: &SyntaxNode) -> bool {
    node.children_with_tokens()
        .filter(|part| part.kind() == SyntaxKind::FROM_KW)
        .any(|from| operand.text_range().end() <= from.text_range().start())
}

/// The metadata definition an `@name`/`#name` annotation names: the
/// qualified name sitting directly under the annotation node.
pub(crate) fn metadata_target(node: &SyntaxNode) -> Option<Target> {
    if node.kind() != SyntaxKind::METADATA_ANNOTATION {
        return None;
    }
    let qname = node
        .children()
        .find(|c| c.kind() == SyntaxKind::QUALIFIED_NAME)?;
    Some(Target {
        segments: name_segments(&qname),
        range: qname.text_range(),
        name_range: last_name_range(&qname),
        at: segment_ranges(&qname),
        chain: chain_steps(&qname),
    })
}

pub(crate) fn relationship_parts(node: &SyntaxNode) -> Vec<(SyntaxKind, Vec<Target>)> {
    // `connector a ::> a.x to b;` writes no `from`, so `a ::> a.x` is
    // the end it runs from -- `ConnectorEnd : Feature = ...
    // ( declaredName = NAME REFERENCES )? OwnedReferenceSubsetting` --
    // and what the end refers to is not something the connector itself
    // refers to.
    let end_refers = names_an_end(node);
    node.children()
        .filter_map(|part| match part.kind() {
            SyntaxKind::REFERENCES if end_refers => None,
            SyntaxKind::TYPING
            | SyntaxKind::SUBSETTING
            | SyntaxKind::REDEFINITION
            | SyntaxKind::REFERENCES => {
                // `end cart : ShoppingCart crosses selectedProduct.inCart`
                // is written in the same shape as `subsets`, and the
                // standard makes a relationship of its own of it: a
                // cross subsetting says which feature of the other end
                // this one is reached across.
                let crosses = part
                    .children_with_tokens()
                    .filter_map(|it| it.into_token())
                    .map(|it| it.kind())
                    .find(|it| !it.is_trivia())
                    == Some(SyntaxKind::CROSSES_KW);
                match crosses {
                    true => Some((SyntaxKind::CROSSES_KW, part)),
                    false => Some((part.kind(), part)),
                }
            }
            // KerML writes `unions T`, `chains a.b`, `disjoint from T`
            // and their kin as the one shape, told apart by the keyword
            // leading it. Five of them relate a type or a feature to
            // another; the rest say something else and are read, where
            // they are read at all, elsewhere.
            SyntaxKind::RELATION => {
                let lead = part
                    .children_with_tokens()
                    .filter_map(|it| it.into_token())
                    .map(|it| it.kind())
                    .find(|it| !it.is_trivia())?;
                // `feature g ~ B::f;` is `feature g conjugates B::f;`
                // spelled the other way the grammar allows
                let lead = match lead {
                    SyntaxKind::TILDE => SyntaxKind::CONJUGATES_KW,
                    other => other,
                };
                matches!(
                    lead,
                    SyntaxKind::UNIONS_KW
                        | SyntaxKind::INTERSECTS_KW
                        | SyntaxKind::DIFFERENCES_KW
                        | SyntaxKind::CHAINS_KW
                        | SyntaxKind::CONJUGATES_KW
                        | SyntaxKind::FEATURED_KW
                        | SyntaxKind::DISJOINT_KW
                )
                .then_some((lead, part))
            }
            _ => None,
        })
        .map(|(kind, part)| {
            let targets = part
                .children()
                .filter(|c| c.kind() == SyntaxKind::TYPE_REF)
                .filter_map(|type_ref| {
                    let qname = type_ref
                        .children()
                        .find(|c| c.kind() == SyntaxKind::QUALIFIED_NAME)?;
                    let mut segments = name_segments(&qname);
                    // `port p : ~P` types the port by the conjugate of
                    // `P`, which the port definition owns under that
                    // name. Naming it is a step further down the same
                    // path, so the walk that finds `P` finds it.
                    let conjugated = type_ref
                        .children_with_tokens()
                        .filter_map(|it| it.into_token())
                        .any(|it| it.kind() == SyntaxKind::TILDE);
                    if conjugated {
                        let last = segments.last()?.clone();
                        segments.push(format!("~{last}"));
                    }
                    Some(Target {
                        chain: chain_steps(&qname),
                        segments,
                        range: match conjugated {
                            true => type_ref.text_range(),
                            false => qname.text_range(),
                        },
                        name_range: last_name_range(&qname),
                        at: segment_ranges(&qname),
                    })
                })
                .collect();
            (kind, targets)
        })
        .collect()
}

/// For a usage introduced by `perform`/`exhibit`/`event`/`include` with a
/// direct reference operand (`perform a.b;`), the segments of that operand.
pub(crate) fn adapter_target_segments(node: &SyntaxNode) -> Option<Vec<String>> {
    adapter_target(node).map(|operand| operand_segments(&operand))
}

/// The operand a `perform`/`exhibit`/`assert`/... usage adapts, when it
/// names one rather than declaring it.
pub(crate) fn adapter_target(node: &SyntaxNode) -> Option<SyntaxNode> {
    if node.kind() != SyntaxKind::USAGE {
        return None;
    }
    let leads_with_adapter = node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| !t.kind().is_trivia())
        .is_some_and(|t| {
            matches!(
                t.kind(),
                SyntaxKind::PERFORM_KW
                    | SyntaxKind::EXHIBIT_KW
                    | SyntaxKind::EVENT_KW
                    | SyntaxKind::INCLUDE_KW
                    | SyntaxKind::SATISFY_KW
                    | SyntaxKind::ASSERT_KW
                    | SyntaxKind::ASSUME_KW
                    | SyntaxKind::REQUIRE_KW
                    | SyntaxKind::VERIFY_KW
                    | SyntaxKind::FRAME_KW
                    | SyntaxKind::RENDER_KW
                    | SyntaxKind::NOT_KW
            )
        });
    if !leads_with_adapter {
        return None;
    }
    let operand = node
        .children()
        .find(|c| matches!(c.kind(), SyntaxKind::NAME_REF | SyntaxKind::PATH_EXPR))?;
    (!operand_segments(&operand).is_empty()).then_some(operand)
}

/// All identifier segments within a reference operand (`a.b`, `A::B.c`).
/// The names written in an expression, each as far as it can be
/// followed.
///
/// A dotted name is taken whole -- `a.b.c` looks `a` up in scope and
/// then each step among the members of the last one's type -- so a
/// `PATH_EXPR` counts only when what it walks from is itself a name.
/// `f(x).b` gives `f` and `x` and stops: nothing in the model says what
/// `f` returns, so there is no namespace for `b` to be a member of.
pub(crate) fn name_chains(node: &SyntaxNode, out: &mut Vec<SyntaxNode>) {
    match node.kind() {
        // `list->select { in i; i > 2 }` declares `i` inside a body that
        // is not built into the model, so the names there have a scope
        // nothing here can see. Reporting them would be a false alarm.
        SyntaxKind::BODY_EXPR => return,
        SyntaxKind::NAME_REF => {
            out.push(node.clone());
            return;
        }
        SyntaxKind::PATH_EXPR if is_name_chain(node) => {
            out.push(node.clone());
            return;
        }
        _ => {}
    }
    for child in node.children() {
        // `f(b = 1)` names a parameter of `f`, not anything in scope here
        if node.kind() == SyntaxKind::ARG_LIST && followed_by_eq(node, &child) {
            continue;
        }
        name_chains(&child, out);
    }
}

/// Whether `=` is the next thing after `child` -- the `b` of `f(b = 1)`.
pub(crate) fn followed_by_eq(parent: &SyntaxNode, child: &SyntaxNode) -> bool {
    let mut after = false;
    for element in parent.children_with_tokens() {
        if element.kind().is_trivia() {
            continue;
        }
        if after {
            return element.kind() == SyntaxKind::EQ;
        }
        after = element.as_node() == Some(child);
    }
    false
}

pub(crate) fn operand_segments(operand: &SyntaxNode) -> Vec<String> {
    operand_name_tokens(operand)
        .filter(|t| {
            matches!(
                t.kind(),
                SyntaxKind::IDENT | SyntaxKind::UNRESTRICTED_NAME | SyntaxKind::DOLLAR
            )
        })
        // a name written from the root means the same thing in an
        // expression as anywhere else, and the root is spelled as the
        // segment a declared name cannot be
        .map(|t| match t.kind() {
            SyntaxKind::DOLLAR => String::new(),
            _ => sysml_syntax::unquote(t.text()),
        })
        .collect()
}

/// The metaclass a relationship's target has to be, where the abstract
/// syntax narrows it.
///
/// `Subsetting::subsettedFeature`, `Redefinition::redefinedFeature` and
/// `ReferenceSubsetting::referencedFeature` are each declared `Feature`,
/// so a name landing on anything else has not resolved -- which tells
/// `feature aa subsets non;` apart from a model that means it, when `non`
/// is a classifier declared next door.
///
/// Only what a feature relates is narrowed. What a definition specializes
/// is a `Classifier` for a `Subclassification` and a `Type` for a plain
/// `Specialization`, and this parser hands both the same node.
pub(crate) fn expected_kind(part: SyntaxKind, is_definition: bool) -> Option<ElementKind> {
    match part {
        SyntaxKind::SUBSETTING | SyntaxKind::CROSSES_KW if is_definition => None,
        SyntaxKind::SUBSETTING
        | SyntaxKind::CROSSES_KW
        | SyntaxKind::REDEFINITION
        | SyntaxKind::REFERENCES => Some(ElementKind::Feature),
        _ => None,
    }
}

/// Whether a declaration writing its own name may be answered with
/// itself.
///
/// `part p4 :> p4;` says the feature is the one its type already
/// declares, and the language reads it that way even where the type
/// declares no such thing. Nothing else can mean that: `part v : v;`
/// would make a feature its own type and `part def C :> C;` a
/// definition its own supertype -- loops that say nothing, and that
/// every reader of the model would have to know to stop at.
pub(crate) fn may_name_itself(part: SyntaxKind, is_definition: bool) -> bool {
    match part {
        // `end cart : ShoppingCart crosses cart::product_account.inCart`
        // -- what an end crosses to is reached through the ends of the
        // association, this one included, so the path may start with
        // the very name being declared
        SyntaxKind::SUBSETTING | SyntaxKind::CROSSES_KW => !is_definition,
        SyntaxKind::REDEFINITION | SyntaxKind::REFERENCES => true,
        _ => false,
    }
}

/// What a relationship written as a statement of its own says, for the
/// kinds that write both ends as plain names: the keyword the first of
/// them follows, and the properties the standard keeps the two on.
///
/// `disjoining d disjoint A from B;`, `conjugation c conjugate A ~ B;`
/// and their kin write their ends in shapes of their own and are not
/// read here.
pub(crate) fn relation_ends(kind: ElementKind) -> Option<(SyntaxKind, &'static str, &'static str)> {
    let ends = match kind {
        ElementKind::Specialization => (SyntaxKind::SUBTYPE_KW, "specific", "general"),
        ElementKind::Subclassification => (
            SyntaxKind::SUBCLASSIFIER_KW,
            "subclassifier",
            "superclassifier",
        ),
        ElementKind::Subsetting => (
            SyntaxKind::SUBSET_KW,
            "subsettingFeature",
            "subsettedFeature",
        ),
        ElementKind::Redefinition => (
            SyntaxKind::REDEFINITION_KW,
            "redefiningFeature",
            "redefinedFeature",
        ),
        ElementKind::FeatureTyping => (SyntaxKind::TYPING_KW, "typedFeature", "type"),
        ElementKind::Disjoining => (SyntaxKind::DISJOINT_KW, "typeDisjoined", "disjoiningType"),
        _ => return None,
    };
    Some(ends)
}

/// The reference written directly after `keyword`, if the next thing is one.
pub(crate) fn operand_after(node: &SyntaxNode, keyword: SyntaxKind) -> Option<SyntaxNode> {
    let mut seen = false;
    for element in node.children_with_tokens() {
        match element.as_token() {
            Some(token) if token.kind().is_trivia() => {}
            Some(token) => {
                if seen {
                    return None;
                }
                seen = token.kind() == keyword;
            }
            None => {
                let child = element.into_node().expect("checked for a token above");
                if seen {
                    return matches!(child.kind(), SyntaxKind::NAME_REF | SyntaxKind::PATH_EXPR)
                        .then_some(child);
                }
            }
        }
    }
    None
}

/// Whether a statement writes a keyword of its own, rather than one
/// nested in something it declares.
pub(crate) fn has_leading(node: &SyntaxNode, keyword: SyntaxKind) -> bool {
    node.children_with_tokens()
        .filter_map(|part| part.into_token())
        .any(|token| token.kind() == keyword)
}

/// The operands naming a connector's or transition's ends.
///
/// A connector relates every reference it holds. A transition writes an
/// optional name of its own first (`transition off_to_on first off then
/// on`), so only the references introduced by `first`/`then` are ends.
///
/// One statement can be two elements, and then the answer depends on
/// which of them is asking -- so it is asked of the element's metaclass
/// rather than of the syntax alone.
pub(crate) fn end_operands(node: &SyntaxNode, of: ElementKind) -> Vec<SyntaxNode> {
    let is_reference = |kind| matches!(kind, SyntaxKind::NAME_REF | SyntaxKind::PATH_EXPR);
    let introduces_end = match node.kind() {
        // A connector statement relates every reference it holds --
        // `bind a.p = b.p;` among them, which writes its second end as
        // a value clause rather than as another operand.
        SyntaxKind::CONNECTOR_STMT => {
            return node
                .children()
                .flat_map(|child| match child.kind() {
                    kind if is_reference(kind) => vec![child],
                    SyntaxKind::VALUE => child
                        .children()
                        .filter(|c| is_reference(c.kind()))
                        .collect(),
                    // `connect ( causeA, causeB, effectC, effectD )`
                    // relates the whole list, and each of them may
                    // count what it relates in front of its name
                    SyntaxKind::PAREN_EXPR | SyntaxKind::PARAM_LIST => {
                        child.children().filter(listed_end).collect()
                    }
                    _ => Vec::new(),
                })
                .collect();
        }
        // `then message m of T from a to b;` is the flow and the
        // succession into it, and each has ends of its own: the flow
        // runs from `a` to `b`, and the step runs into the flow from
        // whatever was written above it. Among the elements a control
        // statement builds, a flow is the only connector that is not
        // itself a succession.
        SyntaxKind::CONTROL_STMT
            if of.is_a(ElementKind::ConnectorAsUsage)
                && !of.is_a(ElementKind::SuccessionAsUsage) =>
        {
            &[SyntaxKind::FROM_KW, SyntaxKind::TO_KW][..]
        }
        // `transition t first a ... then b` writes a name of its own
        // first, and `else A3;` writes where a guard that did not hold
        // goes: `DefaultTargetSuccession : TransitionUsage = 'else'
        // TransitionSuccessionMember`.
        SyntaxKind::CONTROL_STMT => &[
            SyntaxKind::FIRST_KW,
            SyntaxKind::THEN_KW,
            SyntaxKind::ELSE_KW,
        ][..],
        // a binding writes its two ends around an `=` rather than after
        // a keyword each
        _ if node
            .children_with_tokens()
            .filter_map(|it| it.into_token())
            .any(|it| it.kind() == SyntaxKind::BINDING_KW) =>
        {
            return binding_operands(node)
        }
        // `connection c : L connect a to b;` and `flow f of T from a to b;`
        // declare a name and a type before the ends arrive, and
        // `succession a then b;` writes its ends around the keyword.
        // `allocation a : L allocate x to y;` introduces its first end
        // the same way `connect` does.
        _ => &[
            SyntaxKind::CONNECT_KW,
            SyntaxKind::ALLOCATE_KW,
            SyntaxKind::TO_KW,
            SyntaxKind::FROM_KW,
            SyntaxKind::FIRST_KW,
            SyntaxKind::THEN_KW,
        ][..],
    };
    // `a then b` and `interface a.p to b.p;` say where they start
    // before the keyword, in the place a statement writing `first`,
    // `from` or `connect` puts a name and a type instead. Only the very
    // front of the statement is that place: a clause such as `accept rs
    // : T` takes it for itself, and what such a clause declares is a
    // name of its own rather than an end.
    let says_where_first = |kind| {
        matches!(
            kind,
            SyntaxKind::FIRST_KW | SyntaxKind::FROM_KW | SyntaxKind::CONNECT_KW
        )
    };
    let leading_source = introduces_end
        .iter()
        .any(|kind| matches!(kind, SyntaxKind::THEN_KW | SyntaxKind::TO_KW))
        && !node
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| says_where_first(t.kind()));
    // `connector eng to tanks.main1;` and `connector a ::> a.x to b;`
    // write no `from`, and `BinaryConnectorDeclaration : Connector = (
    // FeatureDeclaration? 'from' | isSufficient ?= 'all' 'from'? )?
    // ConnectorEndMember 'to' ConnectorEndMember` allows a declaration
    // only in front of one. So what stands between the keyword and the
    // `to` is the end the connector runs from, named or not, and the
    // connector has no name of its own. What the end refers to wins
    // over the name it was given, which is why the last one before the
    // `to` is the answer.
    let front_of_a_connector = names_an_end(node)
        .then(|| {
            let mut found = None;
            for element in node.children_with_tokens() {
                if element
                    .as_token()
                    .is_some_and(|token| token.kind() == SyntaxKind::TO_KW)
                {
                    break;
                }
                if let Some(child) = element.into_node() {
                    if is_reference(child.kind())
                        || matches!(child.kind(), SyntaxKind::NAME | SyntaxKind::REFERENCES)
                    {
                        found = Some(child);
                    }
                }
            }
            found
        })
        .flatten();
    let mut front = front_of_a_connector.or_else(|| {
        node.children_with_tokens()
            .take_while(|element| {
                element.as_token().is_none_or(|token| {
                    token.kind().is_trivia()
                        || token.kind().is_modifier_kw()
                        || token.kind().is_visibility_kw()
                        || token.kind().is_def_kind_kw()
                        || matches!(
                            token.kind(),
                            SyntaxKind::SUCCESSION_KW | SyntaxKind::TRANSITION_KW
                        )
                })
            })
            .filter_map(|element| element.into_node())
            .find(|child| is_reference(child.kind()))
    });
    let mut out = Vec::new();
    let mut after_keyword = false;
    for element in node.children_with_tokens() {
        match element.as_token() {
            Some(token) if token.kind().is_trivia() => {}
            // only a reference written directly after one of those keywords
            // is an end. Any other keyword in between starts a declaration --
            // `then accept sig after ...`, `flow of Fuel ...` -- whose name
            // is not something to resolve.
            Some(token) => {
                if leading_source && matches!(token.kind(), SyntaxKind::THEN_KW | SyntaxKind::TO_KW)
                {
                    out.extend(front.take());
                }
                after_keyword = introduces_end.contains(&token.kind());
            }
            None => {
                let child = element.into_node().expect("element is a node");
                // `connect [1] myCart to [1] products` and `first [1]
                // paint then [1] dry` count the end before naming it,
                // and the count is not what the keyword introduced
                if child.kind() == SyntaxKind::MULTIPLICITY {
                    continue;
                }
                if after_keyword && is_reference(child.kind()) {
                    out.push(child);
                } else if matches!(
                    child.kind(),
                    SyntaxKind::PAREN_EXPR | SyntaxKind::PARAM_LIST
                ) && (after_keyword || of.is_a(ElementKind::Connector))
                {
                    // `connect (d1, d2, d3)` relates the whole list, and
                    // the parentheses hold it rather than the statement.
                    // KerML writes the list with no keyword at all --
                    // `NaryConnectorDeclaration : Connector =
                    // FeatureDeclaration? '(' ConnectorEndMember ','
                    // ConnectorEndMember ( ',' ConnectorEndMember )*
                    // ')'` -- so a connector's parentheses hold its ends
                    // wherever they stand.
                    out.extend(child.children().filter(listed_end));
                }
                after_keyword = false;
            }
        }
    }
    // `transition first a accept s do action D then b;` -- `do` takes
    // the rest of the statement with it, so the target parses inside the
    // action the effect declares. It is the transition's target either
    // way, and read only from the statement's own children the
    // transition relates one thing.
    if out.len() < 2 && of.is_a(ElementKind::TransitionUsage) {
        let carried = node
            .children()
            .filter(|child| child.kind() == SyntaxKind::USAGE)
            .find_map(|child| operand_after(&child, SyntaxKind::THEN_KW));
        out.extend(carried);
    }
    out
}

/// Segments of each `#keyword` prefix on a definition/usage node.
pub(crate) fn prefix_metadata_segments(node: &SyntaxNode) -> Vec<Vec<String>> {
    node.children()
        .filter(|c| c.kind() == SyntaxKind::PREFIX_METADATA)
        .filter_map(|prefix| {
            let qname = prefix
                .children()
                .find(|c| c.kind() == SyntaxKind::QUALIFIED_NAME)?;
            let segments = name_segments(&qname);
            (!segments.is_empty()).then_some(segments)
        })
        .collect()
}

/// Name segments of a `QUALIFIED_NAME` node (quotes stripped; `$` and
/// wildcards kept as segments).
/// Where each segment of a qualified name is written, in order.
pub(crate) fn segment_ranges(qname: &SyntaxNode) -> Vec<TextRange> {
    qname
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| {
            matches!(
                t.kind(),
                SyntaxKind::IDENT
                    | SyntaxKind::UNRESTRICTED_NAME
                    | SyntaxKind::DOLLAR
                    | SyntaxKind::STAR
                    | SyntaxKind::STAR_STAR
            )
        })
        .map(|t| t.text_range())
        .collect()
}

/// Where each identifier of an operand is written, in order.
pub(crate) fn operand_ranges(operand: &SyntaxNode) -> Vec<TextRange> {
    operand
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        // one range per segment, the root marker included: what each
        // step of the name landed on is paired off against these
        .filter(|t| {
            matches!(
                t.kind(),
                SyntaxKind::IDENT | SyntaxKind::UNRESTRICTED_NAME | SyntaxKind::DOLLAR
            )
        })
        .map(|t| t.text_range())
        .collect()
}

pub(crate) fn name_segments(qname: &SyntaxNode) -> Vec<String> {
    qname
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| {
            matches!(
                t.kind(),
                SyntaxKind::IDENT
                    | SyntaxKind::UNRESTRICTED_NAME
                    | SyntaxKind::DOLLAR
                    | SyntaxKind::STAR
                    | SyntaxKind::STAR_STAR
            )
        })
        .map(|t| {
            // `$` is the root, and `'$'` is a package someone named `$`.
            // Both unquote to the same three characters, so the root is
            // spelled as a segment a declared name cannot be: an empty
            // one. A NAME token always has text.
            if t.kind() == SyntaxKind::DOLLAR {
                return String::new();
            }
            sysml_syntax::unquote(t.text())
        })
        .collect()
}

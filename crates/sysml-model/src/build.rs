//! Structural conversion from the syntax tree to model elements.
//!
//! Builds the ownership tree with element kinds and declared names for the
//! structural constructs (packages, definitions, usages, imports,
//! annotations). Relationships (typings, specializations, redefinitions) are
//! reified with resolved targets by `sysml-semantics`; expression trees are
//! not represented as elements.

use sysml_syntax::{Parse, SyntaxKind, SyntaxNode};

use crate::{ElementId, ElementKind, Model, Role, Value, Vis};

/// Result of building one file into a model: the file's root elements and a
/// map from each created element back to the syntax node it came from.
pub struct Built {
    pub roots: Vec<ElementId>,
    pub source: Vec<(ElementId, SyntaxNode)>,
}

/// Build a [`Model`] from a parsed source file. Returns the model and the
/// root elements (top-level members of the file).
pub fn build_model(parse: &Parse) -> (Model, Vec<ElementId>) {
    let mut model = Model::new();
    let built = build_into(&mut model, parse);
    (model, built.roots)
}

/// Build one parsed file into an existing model (multi-file workspaces).
pub fn build_into(model: &mut Model, parse: &Parse) -> Built {
    let mut built = Built {
        roots: Vec::new(),
        source: Vec::new(),
    };
    for child in parse.syntax().children() {
        build_node(model, &child, None, &mut built);
    }
    built
}

fn build_node(model: &mut Model, node: &SyntaxNode, owner: Option<ElementId>, built: &mut Built) {
    use SyntaxKind::*;
    let kind = match node.kind() {
        PACKAGE => Some(package_kind(node)),
        DEFINITION => Some(definition_kind(node)),
        USAGE => Some(usage_kind(node)),
        CONNECTOR_STMT => connector_kind(node),
        CONTROL_STMT => control_kind(node),
        IMPORT | EXPOSE => Some(import_kind(node)),
        // `dependency use from A to B;` -- a relationship in its own
        // right, with clients on one side of `to` and suppliers on the
        // other, and nothing at all in the model until now
        DEPENDENCY => Some(ElementKind::Dependency),
        // `flow f of Fuel from a to b` -- what the flow carries, which the
        // standard owns from the flow as a feature of its own
        PAYLOAD => Some(ElementKind::PayloadFeature),
        // an alias is a Membership whose memberElement is resolved later
        ALIAS => Some(ElementKind::Membership),
        DOCUMENTATION => Some(ElementKind::Documentation),
        COMMENT_ELEM => Some(ElementKind::Comment),
        REP => Some(ElementKind::TextualRepresentation),
        METADATA_ANNOTATION => Some(ElementKind::MetadataUsage),
        // `mass * speed` ending a calculation body -- the result
        // expression, kept as the text the author wrote the way a guard
        // is. `if c { ... }` structured control is not a result.
        EXPR_STMT if node.children().all(|child| child.kind() != BODY) => {
            Some(ElementKind::Expression)
        }
        _ => None,
    };

    let Some(kind) = kind else {
        // an `entry action a;` control statement wraps a declaration
        // without becoming an element of its own; what it wraps still
        // belongs to the owner
        if node.kind() == CONTROL_STMT {
            for child in node.children() {
                if matches!(child.kind(), DEFINITION | USAGE) {
                    build_node(model, &child, owner, built);
                }
            }
        }
        // other statements, filters, expressions: no structural element of
        // their own — their references are handled during name resolution
        return;
    };

    let id = model.create(kind);
    built.source.push((id, node.clone()));
    match owner {
        Some(owner) => model.add_owned(owner, id),
        None => built.roots.push(id),
    }

    if let Some(name) = declared_name(node).or_else(|| statement_declared_name(node)) {
        model.set(id, "declaredName", Value::String(name));
    }
    if let Some(visibility) = member_visibility(node) {
        model.set_member_visibility(id, visibility);
    }
    if let Some(role) = member_role(node) {
        model.set_member_role(id, role);
    }
    if let Some(direction) = declared_direction(node) {
        if kind.feature("direction").is_some() {
            model.set(id, "direction", Value::EnumLit(direction));
        }
    }
    if let Some(short) = declared_short_name(node) {
        model.set(id, "declaredShortName", Value::String(short));
    }
    // The flags the notation writes as a keyword, and the standard
    // keeps as a property. Every one of these is a fact the source
    // stated: dropping it does not leave the model silent, it leaves it
    // saying `false` -- `variation part def` interchanged as one that
    // is not a variation.
    for (keyword, flag) in [
        (ABSTRACT_KW, "isAbstract"),
        // `Message : FlowUsage = OccurrenceUsagePrefix 'message' ...
        // { isAbstract = true }` -- the specification has no metaclass of
        // its own for a message, and marks it this way instead. It is what
        // tells `message m from a to b` from `flow f from a to b`.
        (MESSAGE_KW, "isAbstract"),
        (CONSTANT_KW, "isConstant"),
        (CONST_KW, "isConstant"),
        (DERIVED_KW, "isDerived"),
        (INDIVIDUAL_KW, "isIndividual"),
        (ORDERED_KW, "isOrdered"),
        (PARALLEL_KW, "isParallel"),
        (PORTION_KW, "isPortion"),
        (STANDARD_KW, "isStandard"),
        (VAR_KW, "isVariable"),
        (VARIATION_KW, "isVariation"),
    ] {
        if has_token(node, keyword) && kind.feature(flag).is_some() {
            model.set(id, flag, Value::Bool(true));
        }
    }
    // `PortionUsage : OccurrenceUsage = ... portionKind = PortionKind ...
    // { isPortion = true }` -- `snapshot s : O;` says both which portion
    // it is and that it is one, and neither was arriving.
    for (keyword, portion) in [(SNAPSHOT_KW, "snapshot"), (TIMESLICE_KW, "timeslice")] {
        if has_token(node, keyword) && kind.feature("portionKind").is_some() {
            model.set(id, "portionKind", Value::EnumLit(portion));
            model.set(id, "isPortion", Value::Bool(true));
        }
    }
    // `nonunique` is the only one of these that turns a flag off: the
    // standard's default is that a feature's values are unique, and a
    // model saying they are not must not arrive saying they are.
    if has_token(node, NONUNIQUE_KW) && kind.feature("isUnique").is_some() {
        model.set(id, "isUnique", Value::Bool(false));
    }
    // `not satisfy r by p;` asserts that it does not, which is the
    // opposite of what the drawing and the generated stub would say of
    // it otherwise. `assert not` negates in the same way.
    if has_token(node, NOT_KW) && kind.feature("isNegated").is_some() {
        model.set(id, "isNegated", Value::Bool(true));
    }
    // `end #original r1 : Req1;` -- what a connector relates, as opposed to
    // an ordinary feature it happens to own
    if has_token(node, END_KW) && kind.feature("isEnd").is_some() {
        model.set(id, "isEnd", Value::Bool(true));
    }
    // `part driver : Driver;` is something the owner is made of and
    // `ref part driver : Driver;` one it only refers to. The standard
    // keeps that on `isComposite`, with `isReference = not isComposite`
    // (KerML), and the graphical notation draws the two with the same
    // diamond -- filled for a composite feature membership, hollow for
    // a noncomposite one.
    if kind.feature("isComposite").is_some() {
        model.set(
            id,
            "isComposite",
            Value::Bool(is_composite(node, kind, owner, model)),
        );
    }
    if kind.is_a(ElementKind::Comment) {
        if let Some(body) = comment_body(node) {
            model.set(id, "body", Value::String(body));
        }
    }
    if kind == ElementKind::TextualRepresentation {
        if let Some(lang) = string_token(node) {
            model.set(id, "language", Value::String(lang));
        }
    }
    if kind == ElementKind::Expression && node.kind() == EXPR_STMT {
        model.set_member_role(id, Role::Result);
        if let Some(written) = node.children().next() {
            represent_textually(model, id, written.text().to_string().trim());
        }
    }
    if kind == ElementKind::TransitionUsage {
        if let Some(trigger) = reify_accept_payload(model, node, id) {
            model.set(id, "triggerAction", Value::RefList(vec![trigger]));
        }
        reify_guard(model, node, id);
        reify_effect(model, node, id);
    }
    if kind == ElementKind::AcceptActionUsage {
        reify_accept_payload(model, node, id);
    }
    if kind.feature("multiplicity").is_some() {
        reify_multiplicity(model, node, id);
    }
    // every feature, not only the usages SysML layers on them: the
    // standard puts a `FeatureValue` on `Feature`, and KerML writes
    // `feature x = 5;` as readily as SysML writes `attribute x = 5;`
    if kind.is_a(ElementKind::Feature) {
        reify_feature_value(model, node, id);
    }

    // recurse into the element's body, parameter list and nested
    // declarations (`end x [1..*] feature y : T;`)
    for child in node.children() {
        match child.kind() {
            BODY | PARAM_LIST => {
                for member in child.children() {
                    build_node(model, &member, Some(id), built);
                }
            }
            PAYLOAD => build_node(model, &child, Some(id), built),
            // Two shapes wrap the declaration the author wrote in an
            // element of their own: `then action b;` (a succession) and
            // `in event occurrence ieo;` (an anonymous direction/adapter
            // wrapper). In both the name belongs to the enclosing scope,
            // not one level in.
            //
            // A connector end really does own what it nests, though --
            // `end [1] feature transferTarget references target;` is an
            // anonymous end whose feature is its own member -- so `end`
            // keeps the nesting.
            DEFINITION | USAGE => {
                let wrapper = node.kind() == CONTROL_STMT
                    || (!has_token(node, END_KW)
                        && declared_name(node).is_none()
                        && statement_declared_name(node).is_none());
                let parent = if wrapper { owner } else { Some(id) };
                build_node(model, &child, parent, built);
            }
            _ => {}
        }
    }
}

/// Reify a `[4]` or `[0..*]` clause as the `MultiplicityRange` the standard
/// stores: an owned range whose bounds are literal expressions, referenced
/// from the element's `multiplicity`.
///
/// A single bound is recorded as the range's `bound`, two as `lowerBound`
/// and `upperBound` -- the same shape the written text has, with the KerML
/// reading (`[n]` means exactly n, `[*]` means zero or more) left to the
/// consumer.
fn reify_multiplicity(model: &mut Model, node: &SyntaxNode, owner: ElementId) {
    use SyntaxKind::*;
    let Some(clause) = node.children().find(|child| child.kind() == MULTIPLICITY) else {
        return;
    };
    let range = model.create(ElementKind::MultiplicityRange);
    model.add_owned(owner, range);
    model.set(owner, "multiplicity", Value::Ref(range));

    // How many bounds there are is what the brackets say, not how many
    // of them we could read. Dropping one changes what the others mean:
    // `[1..18446744073709551615]` would arrive as a lone `1` and be read
    // as exactly one, and so would `[-1]`. A bound with no shape here is
    // kept as the text it was written as, which every consumer already
    // treats as a bound it does not know.
    let mut segments: Vec<Vec<sysml_syntax::SyntaxElement>> = vec![Vec::new()];
    for part in clause.children_with_tokens() {
        match part.kind() {
            L_BRACKET | R_BRACKET | WHITESPACE | LINE_NOTE | BLOCK_NOTE => {}
            DOT_DOT => segments.push(Vec::new()),
            _ => segments.last_mut().expect("there is always one").push(part),
        }
    }
    let bounds: Vec<ElementId> = if segments.iter().all(Vec::is_empty) {
        Vec::new() // `[]`, which says nothing
    } else {
        segments
            .iter()
            .map(|segment| bound_expression(model, range, segment))
            .collect()
    };
    match bounds.as_slice() {
        [only] => {
            model.set(range, "bound", Value::Ref(*only));
        }
        [lower, upper] => {
            model.set(range, "lowerBound", Value::Ref(*lower));
            model.set(range, "upperBound", Value::Ref(*upper));
        }
        _ => {}
    }
}

/// One bound of a multiplicity range, as the element it denotes. A
/// bound of one plain token is the literal it spells; anything else --
/// `count + 1`, a number too large for the model's integers, a minus
/// sign where a natural belongs, or nothing at all -- is an expression
/// kept as the text it was written as.
fn bound_expression(
    model: &mut Model,
    range: ElementId,
    segment: &[sysml_syntax::SyntaxElement],
) -> ElementId {
    use SyntaxKind::*;
    let written: String = segment.iter().map(|part| part.to_string()).collect();
    let single = match segment {
        [only] => only.as_token().map(|token| token.kind()),
        _ => None,
    };
    let (kind, value) = match single {
        Some(DECIMAL) => match written.parse() {
            Ok(int) => (ElementKind::LiteralInteger, Value::Int(int)),
            Err(_) => (
                ElementKind::FeatureReferenceExpression,
                Value::String(written.clone()),
            ),
        },
        Some(STAR) => (ElementKind::LiteralInfinity, Value::Bool(true)),
        // `[count]` -- a named bound is an expression, kept as text the
        // way a transition guard is
        _ => (
            ElementKind::FeatureReferenceExpression,
            Value::String(written.clone()),
        ),
    };
    let bound = model.create(kind);
    model.add_owned(range, bound);
    match kind {
        ElementKind::LiteralInteger => {
            model.set(bound, "value", value);
        }
        ElementKind::FeatureReferenceExpression => {
            if let Value::String(text) = value {
                represent_textually(model, bound, &text);
            }
        }
        // `LiteralInfinity` has no value of its own: being one says it all
        _ => {}
    }
    bound
}

/// Reify an `= 1200.0`, `default = x` or `:= "boot"` clause as the
/// `FeatureValue` membership the standard stores: it owns the value
/// expression and says whether the value is a default or an initial one.
fn reify_feature_value(model: &mut Model, node: &SyntaxNode, owner: ElementId) {
    use SyntaxKind::*;
    let Some(clause) = node.children().find(|child| child.kind() == VALUE) else {
        return;
    };
    let membership = model.create(ElementKind::FeatureValue);
    model.add_owned(owner, membership);
    model.set(membership, "featureWithValue", Value::Ref(owner));
    if has_token(&clause, DEFAULT_KW) {
        model.set(membership, "isDefault", Value::Bool(true));
    }
    if has_token(&clause, COLON_EQ) {
        model.set(membership, "isInitial", Value::Bool(true));
    }

    let Some(written) = clause
        .children()
        .find(|child| !matches!(child.kind(), BODY))
    else {
        return;
    };
    let expression = value_expression(model, membership, &written);
    model.set(membership, "value", Value::Ref(expression));
}

/// The expression a feature value holds. A literal becomes the matching
/// literal element; anything else is an `Expression` kept as the text the
/// author wrote, the way a transition guard is.
fn value_expression(model: &mut Model, membership: ElementId, written: &SyntaxNode) -> ElementId {
    if let Some((kind, value)) = literal_value(written) {
        let literal = model.create(kind);
        model.add_owned(membership, literal);
        model.set(literal, "value", value);
        return literal;
    }
    // `= ledPinNumber` refers to a feature rather than computing
    // anything, and the standard has an expression kind for exactly
    // that. Name resolution fills in the `referent`, which is how a
    // reader of the model can follow the name to what it stands for
    // without resolving it again.
    let kind = if sysml_syntax::is_name_chain(written) {
        ElementKind::FeatureReferenceExpression
    } else {
        ElementKind::Expression
    };
    let expression = model.create(kind);
    model.add_owned(membership, expression);
    represent_textually(model, expression, written.text().to_string().trim());
    expression
}

/// The literal a value clause holds, when it holds one this model reifies.
/// `-2` arrives as a unary minus around the literal and is folded here.
fn literal_value(written: &SyntaxNode) -> Option<(ElementKind, Value)> {
    use SyntaxKind::*;
    if written.kind() == UNARY_EXPR {
        let mut parts = written.children_with_tokens().filter(|part| {
            !part
                .as_token()
                .is_some_and(|token| token.kind().is_trivia())
        });
        let minus = parts.next()?.into_token()?.kind() == MINUS;
        let inner = parts.next()?.into_node()?;
        if !minus || parts.next().is_some() {
            return None;
        }
        return match literal_value(&inner)? {
            (ElementKind::LiteralInteger, Value::Int(int)) => {
                Some((ElementKind::LiteralInteger, Value::Int(-int)))
            }
            (ElementKind::LiteralRational, Value::Real(real)) => {
                Some((ElementKind::LiteralRational, Value::Real(-real)))
            }
            _ => None,
        };
    }
    if written.kind() != LITERAL {
        return None;
    }
    let token = written
        .children_with_tokens()
        .find_map(sysml_syntax::SyntaxElement::into_token)?;
    match token.kind() {
        DECIMAL => token
            .text()
            .parse()
            .ok()
            .map(|int| (ElementKind::LiteralInteger, Value::Int(int))),
        REAL => token
            .text()
            .parse()
            .ok()
            .map(|real| (ElementKind::LiteralRational, Value::Real(real))),
        TRUE_KW => Some((ElementKind::LiteralBoolean, Value::Bool(true))),
        FALSE_KW => Some((ElementKind::LiteralBoolean, Value::Bool(false))),
        STRING => Some((
            ElementKind::LiteralString,
            Value::String(token.text().trim_matches('"').to_string()),
        )),
        _ => None,
    }
}

/// Attach the text an element was written as, the way a transition guard
/// keeps its condition: a `TextualRepresentation` in the `sysml` language.
fn represent_textually(model: &mut Model, element: ElementId, text: &str) {
    let written = model.create(ElementKind::TextualRepresentation);
    model.add_owned(element, written);
    model.set(written, "language", Value::String("sysml".to_string()));
    model.set(written, "body", Value::String(text.to_string()));
    model.set(written, "representedElement", Value::Ref(element));
}

/// Reify what an `accept x : T via p` clause waits for.
///
/// The parser leaves the clause flat, so the payload has nothing standing
/// for it. Its name is what the rest of the model refers to --
/// `subscribing.sub`, `trigger1.ignitionCmd` -- and for a transition
/// `sysml-semantics` attaches the typing written after it.
fn reify_accept_payload(
    model: &mut Model,
    node: &SyntaxNode,
    owner: ElementId,
) -> Option<ElementId> {
    let mut after_accept = false;
    let mut name = None;
    for element in node.children_with_tokens() {
        match element.as_token() {
            Some(token) if token.kind().is_trivia() => {}
            Some(token) if token.kind() == SyntaxKind::ACCEPT_KW => after_accept = true,
            // any other keyword closes the slot the payload name sits in
            Some(_) if after_accept => break,
            Some(_) => {}
            None => {
                let child = element.into_node().expect("checked for a token above");
                if after_accept && child.kind() == SyntaxKind::NAME_REF {
                    name = child.first_token().map(|t| unquote(t.text()));
                    break;
                }
            }
        }
    }
    // no `accept` clause, nothing to stand for
    let name = name?;
    let payload = model.create(ElementKind::AcceptActionUsage);
    model.add_owned(owner, payload);
    model.set(payload, "declaredName", Value::String(name));
    Some(payload)
}

/// Reify the condition a transition is guarded by.
///
/// The condition parses as an expression tree, which this model does not
/// represent as elements, so what is kept is its source text -- recorded the
/// way SysML records any element in a concrete syntax, as a textual
/// representation of the guard.
fn reify_guard(model: &mut Model, node: &SyntaxNode, transition: ElementId) {
    let Some(condition) = node
        .children()
        .find(|child| child.kind() == SyntaxKind::COND_EXPR)
    else {
        return;
    };
    let text = condition.text().to_string();
    let guard = model.create(ElementKind::Expression);
    model.add_owned(transition, guard);
    model.set(transition, "guardExpression", Value::RefList(vec![guard]));

    let written = model.create(ElementKind::TextualRepresentation);
    model.add_owned(guard, written);
    model.set(written, "language", Value::String("sysml".to_string()));
    model.set(
        written,
        "body",
        Value::String(text.strip_prefix("if").unwrap_or(&text).trim().to_string()),
    );
    model.set(written, "representedElement", Value::Ref(guard));
}

/// Reify the action a transition performs on its way across.
///
/// `transition t first a do send x to b then b;` writes the effect inline
/// and the parser leaves it as flat tokens, so nothing would otherwise
/// stand for it. The library declares that action as `TransitionAction::
/// effect`, which the inline one redefines, so it is reified under that
/// name -- `t.effect` then refers to what the transition actually does.
fn reify_effect(model: &mut Model, node: &SyntaxNode, transition: ElementId) {
    let mut after_do = false;
    let mut sends = false;
    for token in tokens(node) {
        match token {
            SyntaxKind::THEN_KW => break,
            SyntaxKind::DO_KW => after_do = true,
            SyntaxKind::SEND_KW if after_do => sends = true,
            _ => {}
        }
    }
    if !after_do {
        return;
    }
    let kind = if sends {
        ElementKind::SendActionUsage
    } else {
        ElementKind::ActionUsage
    };
    let effect = model.create(kind);
    model.add_owned(transition, effect);
    model.set(effect, "declaredName", Value::String("effect".to_string()));
    model.set(transition, "effectAction", Value::RefList(vec![effect]));
}

/// The connector a `CONNECTOR_STMT` reifies, keyed on its leading keyword.
///
/// `flow` and `message` are parsed as usages and already become elements
/// that way. The remaining connector statements the parser folds into this
/// node -- `first x then y` among them -- keep their pre-existing treatment
/// as plain statements with no element of their own.
fn connector_kind(node: &SyntaxNode) -> Option<ElementKind> {
    tokens(node).find_map(|token| match token {
        SyntaxKind::CONNECT_KW => Some(ElementKind::ConnectionUsage),
        SyntaxKind::BIND_KW => Some(ElementKind::BindingConnectorAsUsage),
        SyntaxKind::ALLOCATE_KW => Some(ElementKind::AllocationUsage),
        _ => None,
    })
}

/// The `CONTROL_STMT` forms that carry structure of their own: the two that
/// relate a source to a target (`transition [name] first x then y` and a
/// bare `first x then y`), and the named control nodes a succession can
/// point at (`merge continue;`, `join join1;`).
///
/// The node also covers `entry`/`exit`/`do`, loops and `then y` on its own,
/// which keep their pre-existing treatment as plain statements.
fn control_kind(node: &SyntaxNode) -> Option<ElementKind> {
    // `then merge continue;` is written as one statement but declares the
    // node; the declaration is what the rest of the flow refers to, so it
    // wins over the succession the leading `then` would otherwise make.
    let declaration = tokens(node).find_map(control_node_kind);
    declaration.or_else(|| {
        tokens(node).find_map(|token| match token {
            SyntaxKind::TRANSITION_KW => Some(ElementKind::TransitionUsage),
            // `terminate c1;` -- unlike `merge m`, the name of a terminate
            // node comes before the keyword, so it is not one of the
            // declaring keywords a name is looked for after
            SyntaxKind::TERMINATE_KW => Some(ElementKind::TerminateActionUsage),
            SyntaxKind::FIRST_KW | SyntaxKind::THEN_KW => Some(ElementKind::SuccessionAsUsage),
            _ => None,
        })
    })
}

/// The element a keyword declares when it appears inside a control
/// statement, if it declares one.
///
/// `then action b;` nests a `USAGE` the builder already descends into, but
/// `then merge continue;` and `then message m2 of T;` are parsed flat, so
/// the declaration only survives if this statement becomes it.
fn control_node_kind(token: SyntaxKind) -> Option<ElementKind> {
    match token {
        SyntaxKind::MERGE_KW => Some(ElementKind::MergeNode),
        SyntaxKind::DECIDE_KW => Some(ElementKind::DecisionNode),
        SyntaxKind::FORK_KW => Some(ElementKind::ForkNode),
        SyntaxKind::JOIN_KW => Some(ElementKind::JoinNode),
        SyntaxKind::MESSAGE_KW => Some(ElementKind::FlowUsage),
        _ => None,
    }
}

/// Some statements write their own name as a plain reference rather than
/// the `NAME` node a declaration carries: `merge continue;`, `transition
/// off_to_on first off then on`, `action engineStarted accept engineStart`,
/// `action stop terminate;`.
///
/// The name is the reference before the keyword that introduces the
/// statement's operands, so a bare `first x then y` stays unnamed. A usage
/// only takes a name this way when such a keyword is present, leaving
/// `perform pp.gt;` to the effective name resolution gives it.
fn statement_declared_name(node: &SyntaxNode) -> Option<String> {
    let introduces_operands = |kind| {
        matches!(
            kind,
            SyntaxKind::FIRST_KW
                | SyntaxKind::THEN_KW
                | SyntaxKind::ACCEPT_KW
                | SyntaxKind::SEND_KW
                | SyntaxKind::TERMINATE_KW
                | SyntaxKind::ASSIGN_KW
        )
    };
    // `dependency Use from A to B;` names itself before `from`, and
    // `Dependency = 'dependency' ( Identification? 'from' )? ...` says a
    // name is only there when `from` is: `dependency Z to A;` starts
    // with a client.
    if node.kind() == SyntaxKind::DEPENDENCY {
        if !has_token(node, SyntaxKind::FROM_KW) {
            return None;
        }
        return node
            .children_with_tokens()
            .take_while(|part| part.kind() != SyntaxKind::FROM_KW)
            .filter_map(|part| part.into_node())
            .find(|child| child.kind() == SyntaxKind::NAME_REF)
            .and_then(|name| Some(unquote(name.first_token()?.text())));
    }
    match node.kind() {
        SyntaxKind::CONTROL_STMT => {}
        SyntaxKind::USAGE if tokens(node).any(introduces_operands) => {}
        _ => return None,
    }
    // `then merge continue;` names the node it declares, so the leading
    // `then` does not end the search -- the declaration keyword restarts it
    let declares = tokens(node).any(|t| control_node_kind(t).is_some());
    // `a then b` takes an operand on each side of the keyword, unlike
    // `first`, `accept` and the rest, which take only what follows. So
    // the reference at the front of a bare `then` is where the flow
    // comes from, and reading it as a name leaves the statement saying
    // nothing about what it joins. A `first` puts the source after
    // itself and leaves the front for a name again.
    let bare_then =
        !declares && has_token(node, SyntaxKind::THEN_KW) && !has_token(node, SyntaxKind::FIRST_KW);
    let mut reached_declaration = !declares;
    for element in node.children_with_tokens() {
        if let Some(token) = element.as_token() {
            if control_node_kind(token.kind()).is_some() {
                reached_declaration = true;
                continue;
            }
            // past that keyword every reference is an operand, not a name
            if reached_declaration && introduces_operands(token.kind()) {
                return None;
            }
            continue;
        }
        let child = element.into_node().expect("checked for a token above");
        if reached_declaration && child.kind() == SyntaxKind::NAME_REF {
            if bare_then {
                return None;
            }
            return Some(unquote(child.first_token()?.text()));
        }
    }
    None
}

fn package_kind(node: &SyntaxNode) -> ElementKind {
    use SyntaxKind::*;
    if has_token(node, LIBRARY_KW) {
        ElementKind::LibraryPackage
    } else if has_token(node, NAMESPACE_KW) {
        ElementKind::Namespace
    } else {
        ElementKind::Package
    }
}

fn import_kind(node: &SyntaxNode) -> ElementKind {
    use SyntaxKind::*;
    let wildcard = node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .any(|t| matches!(t.kind(), STAR | STAR_STAR));
    if wildcard {
        ElementKind::NamespaceImport
    } else {
        ElementKind::MembershipImport
    }
}

fn definition_kind(node: &SyntaxNode) -> ElementKind {
    use SyntaxKind::*;
    let kws = kind_keywords(node);
    let name = match kws.first() {
        Some(PART_KW) => "PartDefinition",
        Some(ATTRIBUTE_KW) => "AttributeDefinition",
        Some(PORT_KW) => "PortDefinition",
        Some(ITEM_KW) => "ItemDefinition",
        Some(ACTION_KW) => "ActionDefinition",
        Some(CALC_KW) => "CalculationDefinition",
        Some(STATE_KW) => "StateDefinition",
        Some(CONSTRAINT_KW) => "ConstraintDefinition",
        Some(REQUIREMENT_KW) => "RequirementDefinition",
        Some(CONNECTION_KW) => "ConnectionDefinition",
        Some(INTERFACE_KW) => "InterfaceDefinition",
        Some(ALLOCATION_KW) => "AllocationDefinition",
        Some(ENUM_KW) => "EnumerationDefinition",
        Some(OCCURRENCE_KW) => "OccurrenceDefinition",
        Some(VIEW_KW) => "ViewDefinition",
        Some(VIEWPOINT_KW) => "ViewpointDefinition",
        Some(RENDERING_KW) => "RenderingDefinition",
        Some(METADATA_KW) => "MetadataDefinition",
        Some(CONCERN_KW) => "ConcernDefinition",
        Some(CASE_KW) => "CaseDefinition",
        Some(USE_KW) => "UseCaseDefinition",
        Some(ANALYSIS_KW) => "AnalysisCaseDefinition",
        Some(VERIFICATION_KW) => "VerificationCaseDefinition",
        Some(FLOW_KW) | Some(SUCCESSION_KW) => "FlowDefinition",
        // KerML classifiers
        Some(TYPE_KW) => "Type",
        Some(CLASSIFIER_KW) => "Classifier",
        Some(CLASS_KW) => "Class",
        Some(DATATYPE_KW) => "DataType",
        Some(STRUCT_KW) => "Structure",
        Some(ASSOC_KW) => {
            if kws.contains(&STRUCT_KW) {
                "AssociationStructure"
            } else {
                "Association"
            }
        }
        Some(BEHAVIOR_KW) => "Behavior",
        Some(FUNCTION_KW) => "Function",
        Some(PREDICATE_KW) => "Predicate",
        Some(INTERACTION_KW) => "Interaction",
        Some(METACLASS_KW) => "Metaclass",
        // What is left is `#service def X`, which takes its kind from the
        // user-defined keyword rather than naming one: a SysML definition
        // of unspecified kind, not a bare KerML classifier. Every other
        // definition the parser builds carries one of the keywords above.
        _ => "Definition",
    };
    kind_or(name, ElementKind::Classifier)
}

fn usage_kind(node: &SyntaxNode) -> ElementKind {
    use SyntaxKind::*;
    // adapter keywords take precedence: `perform action a` is a
    // PerformActionUsage, not an ActionUsage
    for token in tokens(node) {
        let candidate = match token {
            PERFORM_KW => Some("PerformActionUsage"),
            EXHIBIT_KW => Some("ExhibitStateUsage"),
            EVENT_KW => Some("EventOccurrenceUsage"),
            INCLUDE_KW => Some("IncludeUseCaseUsage"),
            SATISFY_KW => Some("SatisfyRequirementUsage"),
            ASSERT_KW => Some("AssertConstraintUsage"),
            MESSAGE_KW => Some("FlowUsage"),
            // `action stop terminate;` -- `TerminateNode :
            // TerminateActionUsage = ... 'terminate' ...`, so the keyword
            // fixes the metaclass however the action was introduced
            TERMINATE_KW => Some("TerminateActionUsage"),
            // `action publish send ... via p` carries the library's payload
            // parameters (sentMessage, acceptedMessage)
            SEND_KW => Some("SendActionUsage"),
            ACCEPT_KW => Some("AcceptActionUsage"),
            // roles that fix the metaclass without a kind keyword of
            // their own: an actor or stakeholder is a part, an objective
            // a requirement
            ACTOR_KW | STAKEHOLDER_KW => Some("PartUsage"),
            // `PortionUsage : OccurrenceUsage = ... portionKind =
            // PortionKind ...` -- the portion keyword stands where a kind
            // keyword would, and without it `snapshot s : O;` arrives as
            // the bare reference a usage with no keyword at all would
            // `IndividualUsage : OccurrenceUsage = ... isIndividual ?=
            // 'individual' ...` the same way
            SNAPSHOT_KW | TIMESLICE_KW | INDIVIDUAL_KW => Some("OccurrenceUsage"),
            OBJECTIVE_KW => Some("RequirementUsage"),
            _ => None,
        };
        if let Some(name) = candidate {
            return kind_or(name, ElementKind::Usage);
        }
    }
    let kws = kind_keywords(node);
    let name = match kws.first() {
        Some(PART_KW) => "PartUsage",
        Some(ATTRIBUTE_KW) => "AttributeUsage",
        Some(PORT_KW) => "PortUsage",
        Some(ITEM_KW) => "ItemUsage",
        Some(ACTION_KW) => "ActionUsage",
        Some(CALC_KW) => "CalculationUsage",
        Some(STATE_KW) => "StateUsage",
        Some(CONSTRAINT_KW) => "ConstraintUsage",
        Some(REQUIREMENT_KW) => "RequirementUsage",
        Some(CONNECTION_KW) => "ConnectionUsage",
        Some(INTERFACE_KW) => "InterfaceUsage",
        Some(ALLOCATION_KW) => "AllocationUsage",
        Some(ENUM_KW) => "EnumerationUsage",
        Some(OCCURRENCE_KW) => "OccurrenceUsage",
        Some(VIEW_KW) => "ViewUsage",
        Some(VIEWPOINT_KW) => "ViewpointUsage",
        Some(RENDERING_KW) => "RenderingUsage",
        Some(METADATA_KW) => "MetadataUsage",
        Some(CONCERN_KW) => "ConcernUsage",
        Some(CASE_KW) => "CaseUsage",
        Some(USE_KW) => "UseCaseUsage",
        Some(ANALYSIS_KW) => "AnalysisCaseUsage",
        Some(VERIFICATION_KW) => "VerificationCaseUsage",
        Some(FLOW_KW) => "FlowUsage",
        // `succession flow x from a to b` is a SuccessionFlowUsage; a
        // bare `succession a then b` is a SuccessionAsUsage. The keyword
        // that follows is what tells them apart.
        Some(SUCCESSION_KW) if kws.get(1) == Some(&FLOW_KW) => "SuccessionFlowUsage",
        Some(SUCCESSION_KW) => "SuccessionAsUsage",
        // KerML features
        Some(FEATURE_KW) => "Feature",
        Some(STEP_KW) => "Step",
        Some(EXPR_KW) => "Expression",
        Some(BOOL_KW) => "BooleanExpression",
        Some(INV_KW) => "Invariant",
        Some(CONNECTOR_KW) => "Connector",
        Some(BINDING_KW) => "BindingConnector",
        Some(MULTIPLICITY_KW) => "Multiplicity",
        // A usage that names no kind is a reference: `ref x;` spells it
        // out, and `subject s;` or a bare `x : T;` mean the same thing.
        _ => "ReferenceUsage",
    };
    kind_or(name, ElementKind::Usage)
}

fn kind_or(name: &str, fallback: ElementKind) -> ElementKind {
    ElementKind::from_name(name).unwrap_or(fallback)
}

fn tokens(node: &SyntaxNode) -> impl Iterator<Item = SyntaxKind> + '_ {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .map(|t| t.kind())
}

/// The visibility written before a member, when the default was overridden.
fn member_visibility(node: &SyntaxNode) -> Option<Vis> {
    use SyntaxKind::*;
    with_wrapper(node).find_map(|scope| {
        tokens(&scope).find_map(|token| match token {
            PRIVATE_KW => Some(Vis::Private),
            PROTECTED_KW => Some(Vis::Protected),
            PUBLIC_KW => Some(Vis::Public),
            _ => None,
        })
    })
}

/// The syntactic role a member was declared in, when it has one: what
/// makes `subject veh : Vehicle;` a subject rather than a plain feature.
fn member_role(node: &SyntaxNode) -> Option<Role> {
    use SyntaxKind::*;
    // `entry action a;` puts its keyword before the usage, not inside it
    let mut before = node.prev_sibling_or_token();
    while let Some(part) = before {
        match &part {
            sysml_syntax::SyntaxElement::Token(token) if token.kind().is_trivia() => {
                before = part.prev_sibling_or_token();
            }
            sysml_syntax::SyntaxElement::Token(token) => {
                let role = match token.kind() {
                    ENTRY_KW => Some(Role::Entry),
                    DO_KW => Some(Role::Do),
                    EXIT_KW => Some(Role::Exit),
                    _ => None,
                };
                if role.is_some() {
                    return role;
                }
                break;
            }
            _ => break,
        }
    }
    with_wrapper(node).find_map(|scope| {
        tokens(&scope).find_map(|token| match token {
            SUBJECT_KW => Some(Role::Subject),
            ACTOR_KW => Some(Role::Actor),
            STAKEHOLDER_KW => Some(Role::Stakeholder),
            OBJECTIVE_KW => Some(Role::Objective),
            VARIANT_KW => Some(Role::Variant),
            RETURN_KW => Some(Role::Return),
            // a requirement's constraints and concerns; a state's
            // subactions never reach here -- their keyword sits before
            // the usage and is caught above
            ASSUME_KW => Some(Role::Assume),
            REQUIRE_KW => Some(Role::Require),
            FRAME_KW => Some(Role::Frame),
            VERIFY_KW => Some(Role::Verify),
            _ => None,
        })
    })
}

/// Whether a feature is part of what its owner is, rather than
/// something the owner only refers to.
///
/// The rules are the standard's own. `ref` says reference outright
/// (`BasicUsagePrefix : ( isReference ?= 'ref' )?`), and KerML makes a
/// reference of anything with a direction, anything that is a connector
/// end, and anything with no featuring type: `direction <> null or
/// isEnd or featuringType->isEmpty() implies isReference`. SysML adds
/// that a port owns nothing composite but its nested ports:
/// `ownedUsage->reject(oclIsKindOf(PortUsage))->forAll(not
/// isComposite)`. Everything else a type owns is composite.
fn is_composite(
    node: &SyntaxNode,
    kind: ElementKind,
    owner: Option<ElementId>,
    model: &Model,
) -> bool {
    if has_token(node, SyntaxKind::REF_KW)
        || kind == ElementKind::ReferenceUsage
        || has_token(node, SyntaxKind::END_KW)
        || declared_direction(node).is_some()
    {
        return false;
    }
    let Some(owner) = owner else {
        return false;
    };
    let owning = model.kind(owner);
    if !owning.is_a(ElementKind::Type) {
        return false;
    }
    if owning.is_a(ElementKind::PortDefinition) || owning.is_a(ElementKind::PortUsage) {
        return kind.is_a(ElementKind::PortUsage);
    }
    true
}

/// The direction a feature was declared with (`in`, `out`, `inout`).
fn declared_direction(node: &SyntaxNode) -> Option<&'static str> {
    use SyntaxKind::*;
    with_wrapper(node).find_map(|scope| {
        tokens(&scope).find_map(|token| match token {
            INOUT_KW => Some("inout"),
            IN_KW => Some("in"),
            OUT_KW => Some("out"),
            _ => None,
        })
    })
}

/// The node and, when the node was hoisted out of an anonymous wrapper
/// (`variant part optA;` parses as a wrapper around `part optA`), the
/// wrapper too -- the keywords that describe the member sit on it.
fn with_wrapper(node: &SyntaxNode) -> impl Iterator<Item = SyntaxNode> {
    use SyntaxKind::*;
    let wrapper = node.parent().filter(|parent| {
        matches!(parent.kind(), DEFINITION | USAGE)
            && declared_name(parent).is_none()
            && statement_declared_name(parent).is_none()
    });
    std::iter::once(node.clone()).chain(wrapper)
}

fn kind_keywords(node: &SyntaxNode) -> Vec<SyntaxKind> {
    tokens(node).filter(|k| k.is_def_kind_kw()).collect()
}

fn has_token(node: &SyntaxNode, kind: SyntaxKind) -> bool {
    tokens(node).any(|k| k == kind)
}

fn declared_name(node: &SyntaxNode) -> Option<String> {
    let name = node
        .children()
        .find(|c| c.kind() == SyntaxKind::NAME)?
        .first_token()?;
    Some(unquote(name.text()))
}

fn declared_short_name(node: &SyntaxNode) -> Option<String> {
    let short = node
        .children()
        .find(|c| c.kind() == SyntaxKind::SHORT_NAME)?;
    let text: String = short
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| {
            !matches!(
                t.kind(),
                SyntaxKind::LT | SyntaxKind::GT | SyntaxKind::WHITESPACE
            )
        })
        .map(|t| unquote(t.text()))
        .collect();
    (!text.is_empty()).then_some(text)
}

fn comment_body(node: &SyntaxNode) -> Option<String> {
    let token = node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == SyntaxKind::COMMENT_BODY)?;
    let text = token.text();
    let text = text
        .strip_prefix("/*")
        .and_then(|t| t.strip_suffix("*/"))
        .unwrap_or(text);
    Some(text.trim().to_string())
}

fn string_token(node: &SyntaxNode) -> Option<String> {
    let token = node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == SyntaxKind::STRING)?;
    let text = token.text();
    Some(
        text.strip_prefix('"')
            .and_then(|t| t.strip_suffix('"'))
            .unwrap_or(text)
            .to_string(),
    )
}

fn unquote(text: &str) -> String {
    text.strip_prefix('\'')
        .and_then(|t| t.strip_suffix('\''))
        .unwrap_or(text)
        .to_string()
}

#[cfg(test)]
mod tests {
    /// `in event occurrence x;` parses as an anonymous wrapper around the
    /// usage carrying the name, unlike `event occurrence x;`. The name has
    /// to end up where it was written either way.
    /// The guard is an expression tree, which is not made of elements, so
    /// its source text is kept as a textual representation instead.
    /// `ref x;` names no kind of its own; `ref part x;` does.
    #[test]
    fn a_named_bound_is_kept_as_the_expression_it_denotes() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "part def V {\n\tattribute count;\n\tpart wheels : Wheel[count];\n}\n",
        ));
        let wheels = model.owned(roots[0])[1];
        let range = reference(&model, wheels, "multiplicity").unwrap();
        let bound = reference(&model, range, "bound").unwrap();
        // one bound was written, so the pair of them was not
        assert_eq!(reference(&model, range, "lowerBound"), None);
        assert_eq!(model.kind(bound), ElementKind::FeatureReferenceExpression);
        let written = model.owned(bound)[0];
        assert_eq!(model.kind(written), ElementKind::TextualRepresentation);
        assert_eq!(
            model.get(written, "body").and_then(Value::as_str),
            Some("count")
        );
    }

    /// How many bounds a multiplicity has is what the brackets say. A
    /// bound this builder cannot read must still take up its place: drop
    /// it and `[1..18446744073709551615]` arrives as a lone `1`, which
    /// every consumer reads as exactly one -- a wrong answer with
    /// nothing to show that anything was lost.
    #[test]
    fn a_bound_it_cannot_read_still_takes_up_its_place() {
        let cases = [
            // written, lower, upper, single
            ("[1..10]", Some("1"), Some("10"), None),
            ("[4]", None, None, Some("4")),
            // too large for the model's integers, and a sign where a
            // natural belongs: neither is a number, and neither leaves
            // the other one alone in the brackets
            ("[1..18446744073709551615]", Some("1"), Some(""), None),
            ("[-1]", None, None, Some("")),
            ("[..3]", Some(""), Some("3"), None),
            ("[]", None, None, None),
        ];
        for (written, lower, upper, single) in cases {
            let (model, roots) = build_model(&sysml_syntax::parse(&format!(
                "part def V {{\n\tattribute xs : Real{written};\n}}\n"
            )));
            let xs = model.owned(roots[0])[0];
            let range = reference(&model, xs, "multiplicity").expect(written);
            let spelled = |name: &str| -> Option<String> {
                let bound = reference(&model, range, name)?;
                Some(match model.get(bound, "value") {
                    Some(Value::Int(int)) => int.to_string(),
                    // what it could not read is kept as written, and an
                    // empty answer here means "a bound, but not a number"
                    _ => String::new(),
                })
            };
            assert_eq!(spelled("lowerBound").as_deref(), lower, "{written}");
            assert_eq!(spelled("upperBound").as_deref(), upper, "{written}");
            assert_eq!(spelled("bound").as_deref(), single, "{written}");
        }
    }

    #[test]
    fn a_trailing_expression_becomes_the_result_member() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "calc def Momentum {\n\tin mass : Real;\n\tin speed : Real;\n\tmass * speed\n}\n",
        ));
        let result = *model.owned(roots[0]).last().unwrap();
        assert_eq!(model.kind(result), ElementKind::Expression);
        assert_eq!(model.member_role(result), Some(Role::Result));
        let written = model.owned(result)[0];
        assert_eq!(model.kind(written), ElementKind::TextualRepresentation);
        assert_eq!(
            model.get(written, "body").and_then(Value::as_str),
            Some("mass * speed")
        );

        // structured control (`if c { ... }`) is a statement, not a result
        let (model, roots) = build_model(&sysml_syntax::parse(
            "action def Act {\n\tif ready { action a; }\n}\n",
        ));
        assert!(model
            .owned(roots[0])
            .iter()
            .all(|&child| model.kind(child) != ElementKind::Expression));
    }

    #[test]
    fn the_keywords_the_notation_writes_are_kept() {
        // Every one of these is a fact the source stated, and the
        // standard keeps each on a property of its own. Dropping one
        // does not leave the model silent about it -- the interchange
        // writes the default, so a `variation part def` goes out as one
        // that is not a variation.
        let (model, _) = build_model(&sysml_syntax::parse(
            "package P {\n\
             \tvariation part def Choice;\n\
             \tindividual part def Serial1;\n\
             \tabstract part def Shape;\n\
             \tpart def Car { derived attribute d; constant attribute k = 1; }\n\
             \trequirement def R;\n\
             \tpart def Rig { part c : Car; not satisfy R by c; }\n\
             }\n",
        ));
        let flag = |name: &str, property: &str| {
            let id = model
                .ids()
                .find(|&id| model.name(id) == Some(name))
                .unwrap_or_else(|| panic!("`{name}` is declared"));
            model.get(id, property).cloned()
        };
        for (name, property) in [
            ("Choice", "isVariation"),
            ("Serial1", "isIndividual"),
            ("Shape", "isAbstract"),
            ("d", "isDerived"),
            ("k", "isConstant"),
        ] {
            assert_eq!(
                flag(name, property),
                Some(Value::Bool(true)),
                "`{name}` lost `{property}`"
            );
        }
        // `not satisfy` asserts the opposite of what it names
        let negated = model
            .ids()
            .find(|&id| model.kind(id) == ElementKind::SatisfyRequirementUsage)
            .expect("the assertion is built");
        assert_eq!(model.get(negated, "isNegated"), Some(&Value::Bool(true)));
    }

    #[test]
    fn a_value_that_is_not_a_literal_is_kept_as_text() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "part def V {\n\tattribute a = 2 + b;\n\tattribute c = false;\n}\n",
        ));
        let a = model.owned(roots[0])[0];
        let membership = model.owned(a)[0];
        assert_eq!(model.kind(membership), ElementKind::FeatureValue);
        let expression = reference(&model, membership, "value").unwrap();
        assert_eq!(model.kind(expression), ElementKind::Expression);
        let written = model.owned(expression)[0];
        assert_eq!(
            model.get(written, "body").and_then(Value::as_str),
            Some("2 + b")
        );

        // `false` is the boolean literal, not text
        let c = model.owned(roots[0])[1];
        let literal = reference(&model, model.owned(c)[0], "value").unwrap();
        assert_eq!(model.kind(literal), ElementKind::LiteralBoolean);
        assert_eq!(model.get(literal, "value"), Some(&Value::Bool(false)));

        // a literal no reification exists for stays an expression as text
        let (model, roots) = build_model(&sysml_syntax::parse(
            "part def W {\n\tattribute n = null;\n}\n",
        ));
        let n = model.owned(roots[0])[0];
        let kept = reference(&model, model.owned(n)[0], "value").unwrap();
        assert_eq!(model.kind(kept), ElementKind::Expression);
    }

    #[test]
    fn a_negative_default_is_still_a_literal() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "part def M {\n\tattribute shift = -2;\n\tattribute drop = -1.5;\n\tattribute odd = -true;\n}\n",
        ));
        let value_of = |at: usize| {
            let attribute = model.owned(roots[0])[at];
            let literal = reference(&model, model.owned(attribute)[0], "value").unwrap();
            (model.kind(literal), model.get(literal, "value").cloned())
        };
        assert_eq!(
            value_of(0),
            (ElementKind::LiteralInteger, Some(Value::Int(-2)))
        );
        assert_eq!(
            value_of(1),
            (ElementKind::LiteralRational, Some(Value::Real(-1.5)))
        );
        // a minus on something that is not a number stays an expression
        assert_eq!(value_of(2).0, ElementKind::Expression);
    }

    /// The element a property points at, when it points at one.
    fn reference(model: &Model, of: ElementId, property: &str) -> Option<ElementId> {
        match model.get(of, property) {
            Some(Value::Ref(target)) => Some(*target),
            _ => None,
        }
    }

    #[test]
    fn a_usage_without_a_kind_keyword_is_a_reference() {
        // `ref b;`, `subject s;` and a bare `c;` all name no kind of their
        // own; `ref part a;` does, so it keeps it
        let (model, roots) = build_model(&sysml_syntax::parse(
            "requirement def R {\n\tref part a;\n\tref b;\n\tsubject s;\n\tc;\n}\n",
        ));
        let kinds: Vec<ElementKind> = model
            .owned(roots[0])
            .iter()
            .map(|&id| model.kind(id))
            .collect();
        assert_eq!(
            kinds,
            [
                ElementKind::PartUsage,
                ElementKind::ReferenceUsage,
                ElementKind::ReferenceUsage,
                ElementKind::ReferenceUsage,
            ]
        );
    }

    #[test]
    fn an_accept_usage_declares_the_payload_it_waits_for() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "action def A {\n\taction trigger1 accept cmd : Cmd;\n}\n",
        ));
        let trigger = model.owned(roots[0])[0];
        assert_eq!(model.kind(trigger), ElementKind::AcceptActionUsage);
        assert_eq!(model.name(trigger), Some("trigger1"));
        // `trigger1.cmd` has to find something
        let payload = model.owned(trigger)[0];
        assert_eq!(model.name(payload), Some("cmd"));
    }

    #[test]
    fn a_transition_guard_is_kept_as_written() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "state def S {\n\
             \tstate a;\n\
             \tstate b;\n\
             \ttransition first a if 1 == 1 then b;\n\
             }\n",
        ));
        let transition = model
            .owned(roots[0])
            .iter()
            .copied()
            .find(|&id| model.kind(id) == ElementKind::TransitionUsage)
            .unwrap();
        let guard = model.owned(transition)[0];
        assert_eq!(model.kind(guard), ElementKind::Expression);

        let written = model.owned(guard)[0];
        assert_eq!(model.kind(written), ElementKind::TextualRepresentation);
        assert_eq!(
            model.get(written, "body").and_then(Value::as_str),
            Some("1 == 1")
        );
        assert_eq!(
            model.get(written, "language").and_then(Value::as_str),
            Some("sysml")
        );
    }

    #[test]
    fn a_direction_wrapper_does_not_swallow_the_name_it_wraps() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "part def M {\n\tevent occurrence eo;\n\tin event occurrence ieo;\n}\n",
        ));
        let names: Vec<&str> = model
            .owned(roots[0])
            .iter()
            .filter_map(|&id| model.name(id))
            .collect();
        assert_eq!(names, ["eo", "ieo"]);
    }

    /// A connector end is an anonymous wrapper too, but the feature it
    /// nests really is its own member.
    #[test]
    fn a_connector_end_keeps_the_feature_it_nests() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "connection def C {\n\tend [1] feature src references source;\n}\n",
        ));
        assert!(model
            .owned(roots[0])
            .iter()
            .all(|&id| model.name(id).is_none()));
        let end = model.owned(roots[0])[0];
        let nested: Vec<&str> = model
            .owned(end)
            .iter()
            .filter_map(|&id| model.name(id))
            .collect();
        assert_eq!(nested, ["src"]);
    }

    #[test]
    fn statements_name_themselves_with_a_plain_reference() {
        // `action b accept x;` and `action stop terminate;` write their
        // names as references, not NAME nodes, while a bare `fork;` names
        // nothing at all
        let (model, roots) = build_model(&sysml_syntax::parse(
            "action def A {\n\
             \tmerge m;\n\
             \tfork;\n\
             \taction b accept x;\n\
             \taction stop terminate;\n\
             \tfirst m then b;\n\
             }\n",
        ));
        let names: Vec<&str> = model
            .owned(roots[0])
            .iter()
            .filter_map(|&id| model.name(id))
            .collect();
        assert_eq!(names, ["m", "b", "stop"]);
    }

    use super::*;

    #[test]
    fn builds_a_small_model() {
        let text = "package VehicleModel {\n  import ScalarValues::*;\n  doc /* The vehicle model. */\n  abstract part def Vehicle {\n    attribute mass : Real = 1200.0;\n    part wheels : Wheel[4];\n  }\n  part myCar : Vehicle;\n}\n";
        let parse = sysml_syntax::parse(text);
        assert!(parse.ok());
        let (model, roots) = build_model(&parse);

        assert_eq!(roots.len(), 1);
        let pkg = roots[0];
        assert_eq!(model.kind(pkg), ElementKind::Package);
        assert_eq!(model.name(pkg), Some("VehicleModel"));

        let owned = model.owned(pkg);
        assert_eq!(owned.len(), 4);
        assert_eq!(model.kind(owned[0]), ElementKind::NamespaceImport);
        assert_eq!(model.kind(owned[1]), ElementKind::Documentation);
        assert_eq!(
            model.get(owned[1], "body").and_then(Value::as_str),
            Some("The vehicle model.")
        );

        let vehicle = owned[2];
        assert_eq!(model.kind(vehicle), ElementKind::PartDefinition);
        assert_eq!(model.name(vehicle), Some("Vehicle"));
        assert_eq!(model.get(vehicle, "isAbstract"), Some(&Value::Bool(true)));
        let members = model.owned(vehicle);
        assert_eq!(model.kind(members[0]), ElementKind::AttributeUsage);
        assert_eq!(model.name(members[0]), Some("mass"));
        assert_eq!(model.kind(members[1]), ElementKind::PartUsage);

        assert_eq!(model.kind(owned[3]), ElementKind::PartUsage);
        assert_eq!(model.name(owned[3]), Some("myCar"));
    }

    #[test]
    fn namespaces_and_kindless_definitions() {
        let parse =
            sysml_syntax::parse_dialect("namespace N { type T; }", sysml_syntax::Dialect::KerML);
        let (model, roots) = build_model(&parse);
        assert_eq!(model.kind(roots[0]), ElementKind::Namespace);

        // `#keyword def X` takes its kind from the keyword's metadata, so
        // the declaration names none -- but `def` still makes it a SysML
        // definition, which is what a definition diagram draws
        let parse = sysml_syntax::parse("#service def X;");
        let (model, roots) = build_model(&parse);
        assert_eq!(model.kind(roots[0]), ElementKind::Definition);

        // a KerML declaration with no recognised keyword stays a classifier
        let parse = sysml_syntax::parse_dialect("classifier C;", sysml_syntax::Dialect::KerML);
        let (model, roots) = build_model(&parse);
        assert_eq!(model.kind(roots[0]), ElementKind::Classifier);

        // KerML `multiplicity` declarations map to Multiplicity
        let parse = sysml_syntax::parse_dialect("multiplicity m;", sysml_syntax::Dialect::KerML);
        let (model, roots) = build_model(&parse);
        assert_eq!(model.kind(roots[0]), ElementKind::Multiplicity);
    }

    #[test]
    fn kerml_classifiers() {
        let text = "package K { classifier A; datatype D; assoc struct S; feature f; }";
        let parse = sysml_syntax::parse_dialect(text, sysml_syntax::Dialect::KerML);
        assert!(parse.ok());
        let (model, roots) = build_model(&parse);
        let kinds: Vec<_> = model
            .owned(roots[0])
            .iter()
            .map(|id| model.kind(*id))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ElementKind::Classifier,
                ElementKind::DataType,
                ElementKind::AssociationStructure,
                ElementKind::Feature,
            ]
        );
    }
}

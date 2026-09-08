//! Structural conversion from the syntax tree to model elements.
//!
//! Builds the ownership tree with element kinds and declared names for the
//! structural constructs (packages, definitions, usages, imports,
//! annotations). Relationships (typings, specializations, redefinitions) are
//! reified with resolved targets by `sysml-semantics`; expression trees are
//! not represented as elements.

use sysml_syntax::{unquote, Parse, SyntaxKind, SyntaxNode};

use crate::{ElementId, ElementKind, Model, Role, Value, Vis};

/// Result of building one file into a model: the file's root elements and a
/// map from each created element back to the syntax node it came from.
pub struct Built {
    pub roots: Vec<ElementId>,
    pub source: Vec<(ElementId, SyntaxNode)>,
    /// Which notation the file was written in. The two share a syntax
    /// tree but not a set of metaclasses, so a node that names no kind
    /// becomes a different thing in each.
    dialect: sysml_syntax::Dialect,
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
        dialect: parse.dialect(),
    };
    for child in parse.syntax().children() {
        build_node(model, &child, None, &mut built);
    }
    built
}

/// Build one syntax node into the model, returning the element it became
/// where it became one.
fn build_node(
    model: &mut Model,
    node: &SyntaxNode,
    owner: Option<ElementId>,
    built: &mut Built,
) -> Option<ElementId> {
    use SyntaxKind::*;
    // `variant part optA;` parses as an anonymous usage carrying only the
    // prefix keywords, wrapped around the usage that carries the name.
    // The wrapper is nothing on its own: making an element of it puts a
    // nameless twin beside every variant and every directed occurrence.
    // What it says belongs to the declaration inside it, which reads it
    // back through `with_wrapper` and `usage_kind`.
    if is_prefix_wrapper(node) {
        for child in node.children() {
            if matches!(child.kind(), DEFINITION | USAGE) {
                build_node(model, &child, owner, built);
            }
        }
        return None;
    }
    let kind = match node.kind() {
        PACKAGE => Some(package_kind(node)),
        DEFINITION => Some(definition_kind(node)),
        USAGE => Some(usage_kind(node, owner, model, built.dialect)),
        // `specialization s subtype A :> B;` and its kin relate two types
        // written elsewhere. They are relationships in their own right,
        // with names of their own, and nothing stood for them at all.
        RELATION_STMT => relation_kind(node),
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
        // `ElementFilterMember : ElementFilterMembership = MemberPrefix
        // 'filter' ownedRelatedElement += OwnedExpression ';'` -- a
        // package that filters its members said so, and the model was
        // arriving with no sign of it
        FILTER => Some(ElementKind::ElementFilterMembership),
        DOCUMENTATION => Some(ElementKind::Documentation),
        COMMENT_ELEM => Some(ElementKind::Comment),
        REP => Some(ElementKind::TextualRepresentation),
        METADATA_ANNOTATION => Some(ElementKind::MetadataUsage),
        // `#Safety part def Boiler;` -- `PrefixMetadataUsage :
        // MetadataUsage = ownedRelationship += OwnedFeatureTyping`, so
        // the prefix is a usage of its own and not a spelling of the
        // element it stands before
        PREFIX_METADATA => Some(ElementKind::MetadataUsage),
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
        return None;
    };

    // `EnumerationUsageMember : VariantMembership = MemberPrefix
    // ownedRelatedElement += EnumeratedValue` -- every value written in an
    // enumeration body is one of its variants, whether or not it repeats
    // the `enum` keyword.
    let enumerated = kind.is_a(ElementKind::EnumerationUsage)
        && owner.is_some_and(|owner| model.kind(owner).is_a(ElementKind::EnumerationDefinition));
    // `then merge continue;` and `then send new S() to b;` are one
    // statement that the abstract syntax makes two elements of: the node
    // or action the statement declares, and the succession its leading
    // `then` writes into it. `control_kind` answers with the declaration,
    // because that is what the rest of the flow refers to by name -- so
    // the succession is built here beside it, ahead of it in the body,
    // where what it continues from is the step written above.
    //
    // A leading `first` writes none: `first x;` names which step comes
    // first and nothing flows into it.
    if let Some(owner) = owner {
        if kind != ElementKind::SuccessionAsUsage && matches!(tokens(node).next(), Some(THEN_KW)) {
            let flow = model.create(ElementKind::SuccessionAsUsage);
            model.add_owned(owner, flow);
            built.source.push((flow, node.clone()));
        }
    }
    // `end owningEntities[1..*] feature owner : LegalEntity;` declares
    // the end `owner`, not the end `owningEntities`: `EndFeaturePrefix
    // ( ownedRelationship += OwnedCrossFeatureMember )?
    // FeatureDeclaration` puts the cross feature between the `end` and
    // the declaration, and the standard says where it lands -- "owned
    // cross features are in the namespace of the owning association
    // ends, so their names are qualified by the name of the association
    // ends, e.g. `LegalAssetOwnership::owner::owningEntities`". So this
    // element is the cross feature, and where it goes is not known
    // until the end it belongs to has been built.
    let crossed = crossing_declaration(node);
    let id = model.create(kind);
    built.source.push((id, node.clone()));
    if crossed.is_none() {
        match owner {
            Some(owner) => model.add_owned(owner, id),
            None => built.roots.push(id),
        }
    }

    if let Some(name) = declared_name(node).or_else(|| statement_declared_name(node)) {
        model.set(id, "declaredName", Value::String(name));
    }
    if let Some(visibility) = member_visibility(node)
        // `validateExposeVisibility` -- "an Expose always has protected
        // visibility", whether or not the source wrote one
        .or_else(|| kind.is_a(ElementKind::Expose).then_some(Vis::Protected))
    {
        model.set_member_visibility(id, visibility);
    }
    if let Some(role) = member_role(node).or_else(|| enumerated.then_some(Role::Variant)) {
        model.set_member_role(id, role);
        // A membership that is a parameter membership fixes the
        // direction of what it owns, and the notation writes it
        // nowhere: `subject s;` is what a requirement takes in.
        if let Some(direction) = crate::parameter_direction(role) {
            if kind.feature("direction").is_some() {
                model.set(id, "direction", Value::EnumLit(direction));
            }
        }
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
    for (keyword, flag) in KEYWORD_FLAGS {
        if scope_has(node, keyword) && kind.feature(flag).is_some() {
            model.set(id, flag, Value::Bool(true));
        }
    }
    // Flags the specification states outright rather than leaving to a
    // keyword. A model that carries one of a pair without the other is
    // one the specification's own constraints reject, and the source
    // said both:
    //
    // - `validateEnumerationDefinitionIsVariation` -- an enumeration
    //   definition is a variation, written `enum def` or not
    // - `validateDefinitionVariationIsAbstract` and its `Usage`
    //   counterpart -- a variation is abstract
    // - `validateFeatureConstantIsVariable` -- a constant feature is a
    //   variable one whose value cannot change
    if kind.is_a(ElementKind::EnumerationDefinition) && kind.feature("isVariation").is_some() {
        model.set(id, "isVariation", Value::Bool(true));
    }
    for (stated, implied) in [("isVariation", "isAbstract"), ("isConstant", "isVariable")] {
        if model.get(id, stated) == Some(&Value::Bool(true)) && kind.feature(implied).is_some() {
            model.set(id, implied, Value::Bool(true));
        }
    }
    // `PortionUsage : OccurrenceUsage = ... portionKind = PortionKind ...
    // { isPortion = true }` -- `snapshot s : O;` says both which portion
    // it is and that it is one, and neither was arriving.
    for (keyword, portion) in [(SNAPSHOT_KW, "snapshot"), (TIMESLICE_KW, "timeslice")] {
        if scope_has(node, keyword) && kind.feature("portionKind").is_some() {
            model.set(id, "portionKind", Value::EnumLit(portion));
            model.set(id, "isPortion", Value::Bool(true));
        }
    }
    // `validateConnectionDefinitionIsSufficient` -- "a
    // ConnectionDefinition is always sufficient": everything that
    // connects the way it says is one of its connections, and the
    // specification states that rather than leaving it to a keyword.
    if kind.is_a(ElementKind::ConnectionDefinition) && kind.feature("isSufficient").is_some() {
        model.set(id, "isSufficient", Value::Bool(true));
    }
    // `nonunique` is the only one of these that turns a flag off: the
    // standard's default is that a feature's values are unique, and a
    // model saying they are not must not arrive saying they are.
    if scope_has(node, NONUNIQUE_KW) && kind.feature("isUnique").is_some() {
        model.set(id, "isUnique", Value::Bool(false));
    }
    // `not satisfy r by p;` asserts that it does not, which is the
    // opposite of what the drawing and the generated stub would say of
    // it otherwise. `assert not` negates in the same way.
    if scope_has(node, NOT_KW) && kind.feature("isNegated").is_some() {
        model.set(id, "isNegated", Value::Bool(true));
    }
    // `end #original r1 : Req1;` -- what a connector relates, as opposed to
    // an ordinary feature it happens to own. A cross feature is written
    // after the same keyword and is not itself an end: what the `end`
    // says is an end is the declaration that follows it.
    if has_token(node, END_KW) && crossed.is_none() && kind.feature("isEnd").is_some() {
        model.set(id, "isEnd", Value::Bool(true));
    }
    // `part driver : Driver;` is something the owner is made of and
    // `ref part driver : Driver;` one it only refers to. The standard
    // keeps that on `isComposite`, with `isReference = not isComposite`
    // (KerML), and the graphical notation draws the two with the same
    // diamond -- filled for a composite feature membership, hollow for
    // a noncomposite one.
    if kind.feature("isComposite").is_some() {
        // `validateUsageIsReferential` -- "a Usage that is directed, an
        // end feature or has no featuringTypes must be referential".
        // `is_composite` reads the direction the source wrote; a
        // `subject` or a `return` is directed by the membership that
        // owns it instead, and is a parameter for the same reason.
        let directed = model.get(id, "direction").is_some();
        // Only a `FeatureMembership` features what it owns, and only
        // what is featured can be part of it. `variant action a1;` is
        // owned through a `VariantMembership`, so the variation is not
        // made of it -- `validateUsageIsReferential` says as much from
        // the other side, since a usage with no featuring type must be
        // referential.
        let composite = owner.is_some_and(|owner| {
            crate::membership_kind(model, id).is_a(ElementKind::FeatureMembership)
                && !directed
                && is_composite(node, kind, model.kind(owner))
        });
        model.set(id, "isComposite", Value::Bool(composite));
    }
    // `port def P` defines two things. The standard has a
    // `PortDefinition` own exactly one `ConjugatedPortDefinition`,
    // named after it and related to it by a `PortConjugation`; the
    // notation writes `port p : ~P` to type a port by that one, so it
    // has to be an element of its own with that name rather than a
    // spelling of `P`.
    if kind == ElementKind::PortDefinition {
        if let Some(name) = model.name(id).map(str::to_string) {
            let conjugate = model.create(ElementKind::ConjugatedPortDefinition);
            model.add_owned(id, conjugate);
            model.set(conjugate, "declaredName", Value::String(format!("~{name}")));
            let conjugation = model.create(ElementKind::PortConjugation);
            model.add_owned(conjugate, conjugation);
            model.set(conjugation, "conjugatedType", Value::Ref(conjugate));
            model.set(conjugation, "originalType", Value::Ref(id));
            // the same pair under the names a port conjugation states
            // them by, which is what the constraints about one read
            model.set(conjugation, "originalPortDefinition", Value::Ref(id));
        }
    }
    if kind.is_a(ElementKind::Comment) {
        if let Some(body) = comment_body(node) {
            model.set(id, "body", Value::String(body));
        }
        // `comment C locale "en-GB" /* ... */` says which language its
        // prose is written in, and `Comment::locale` is where the
        // standard keeps that.
        if let Some(locale) = string_token(node) {
            model.set(id, "locale", Value::String(locale));
        }
    }
    // `import all P::*` brings in what is private as well, and `import
    // P::**` everything nested under what it names. Neither was
    // arriving, so both went out as the plain import they are not.
    if kind.is_a(ElementKind::Import) {
        // An import that does not say `all` brings in only what is
        // public, and saying nothing is not the same as not knowing.
        // `expose` says it either way: `validateExposeIsImportAll` --
        // "an Expose is always an import of everything" -- and a view
        // that showed only the public members of what it is pointed at
        // would leave the rest out of the drawing.
        let everything = has_token(node, ALL_KW) || kind.is_a(ElementKind::Expose);
        model.set(id, "isImportAll", Value::Bool(everything));
        if node
            .descendants_with_tokens()
            .filter_map(|part| part.into_token())
            .any(|token| token.kind() == STAR_STAR)
        {
            model.set(id, "isRecursive", Value::Bool(true));
        }
    }
    if kind == ElementKind::TextualRepresentation {
        if let Some(lang) = string_token(node) {
            model.set(id, "language", Value::String(lang));
        }
        // `textual-representation-node` shows the language *and* the
        // text: a `rep` that says which language it is in and not what
        // it says is a representation of nothing.
        if let Some(body) = comment_body(node) {
            model.set(id, "body", Value::String(body));
        }
    }
    // `expose P::Thing;` says what it exposes and `filter @Safety;` what
    // it filters by, and a view listing neither says only that it exposes
    // and filters something. Both are kept as the text the author wrote,
    // which is what the compartment shows.
    if kind.is_a(ElementKind::Expose) {
        if let Some(qname) = node
            .children()
            .find(|child| child.kind() == SyntaxKind::QUALIFIED_NAME)
        {
            represent_textually(model, id, qname.text().to_string().trim());
        }
    }
    if kind == ElementKind::ElementFilterMembership {
        if let Some(written) = node.children().find(|child| child.kind() != BODY) {
            let condition = model.create(ElementKind::Expression);
            model.add_owned(id, condition);
            represent_textually(model, condition, written.text().to_string().trim());
            // `filter @Safety;` filters by what the expression comes to,
            // and the membership names it outright: without that the
            // model owns an expression and says nothing about what it is
            model.set(id, "condition", Value::Ref(condition));
        }
    }
    if kind == ElementKind::Expression && node.kind() == EXPR_STMT {
        model.set_member_role(id, Role::Result);
        if let Some(written) = node.children().next() {
            represent_textually(model, id, written.text().to_string().trim());
        }
    }
    if kind == ElementKind::TransitionUsage {
        // `TriggerActionMember : TransitionFeatureMembership = ... kind
        // = 'trigger' ownedRelatedElement += TriggerAction` and
        // `TriggerAction : AcceptActionUsage = AcceptParameterPart`:
        // what a transition waits for is an accept action of its own,
        // and the payload written after the keyword is that action's
        // first parameter rather than the trigger itself.
        //
        // `validateTransitionUsageParameters` -- "a TransitionUsage must
        // have at least one owned input parameter and, if it has a
        // triggerAction, it must have at least two". The first is the
        // occurrence it transitions from; the second is what the
        // trigger accepted, which `checkTransitionUsagePayloadSpecialization`
        // has subset the trigger's own payload parameter. The library
        // names that one from the transition -- `bind payload =
        // aState.aTransition.apayload;` -- and the standard says how:
        // its naming feature is the trigger's payload parameter.
        let occurrence = model.create(ElementKind::ReferenceUsage);
        model.add_owned(id, occurrence);
        model.set(occurrence, "direction", Value::EnumLit("in"));
        if has_token(node, ACCEPT_KW) {
            let accepted = model.create(ElementKind::ReferenceUsage);
            model.add_owned(id, accepted);
            model.set(accepted, "direction", Value::EnumLit("in"));
            let trigger = model.create(ElementKind::AcceptActionUsage);
            model.add_owned(id, trigger);
            let payload = reify_accept_payload(model, node, trigger);
            reify_action_arguments(
                model,
                node,
                trigger,
                ElementKind::AcceptActionUsage,
                payload,
            );
            model.set(id, "triggerAction", Value::RefList(vec![trigger]));
            if let Some(payload) = payload {
                let subsetting = model.create(ElementKind::Subsetting);
                model.add_owned(accepted, subsetting);
                model.set(subsetting, "subsettingFeature", Value::Ref(accepted));
                model.set(subsetting, "subsettedFeature", Value::Ref(payload));
            }
        }
        reify_guard(model, node, id);
        reify_effect(model, node, id);
        // A transition is not a connector: what it relates it relates
        // through a `Succession` of its own, which is what
        // `validateTransitionUsageSuccession` asks for. Name resolution
        // fills in the two ends, which are the transition's own.
        let succession = model.create(ElementKind::SuccessionAsUsage);
        model.add_owned(id, succession);
    }
    let payload = (kind == ElementKind::AcceptActionUsage)
        .then(|| reify_accept_payload(model, node, id))
        .flatten();
    reify_action_arguments(model, node, id, kind, payload);
    // `IfNode : IfActionUsage = ... 'if' ownedRelationship +=
    // ExpressionParameterMember ...` and the two loops the same way: the
    // condition is what the node is about, and it was being read and
    // dropped.
    // `ForLoopNode : ForLoopActionUsage = 'for' LoopVariableMember 'in'
    // ExpressionParameterMember ...` -- the variable is written first and
    // is the first feature the loop owns, which is what
    // `validateForLoopActionUsageLoopVariable` asks of it.
    if kind == ElementKind::ForLoopActionUsage {
        reify_loop_variable(model, node, id);
    }
    if structured_node(kind) {
        reify_condition(model, node, id);
    }
    if kind.feature("multiplicity").is_some() {
        reify_multiplicity(model, node, id);
    }
    // every feature, not only the usages SysML layers on them: the
    // standard puts a `FeatureValue` on `Feature`, and KerML writes
    // `feature x = 5;` as readily as SysML writes `attribute x = 5;`
    //
    // An assignment is the exception: `assign x := 1;` gives the value to
    // `x`, and reading it as the assignment's own value says the action
    // itself is one.
    if kind.is_a(ElementKind::Feature) && !kind.is_a(ElementKind::AssignmentActionUsage) {
        reify_feature_value(model, node, id);
    }

    // recurse into the element's body, parameter list and nested
    // declarations (`end x [1..*] feature y : T;`)
    for child in node.children() {
        match child.kind() {
            BODY | PARAM_LIST => {
                // `IfNode = 'if' ExpressionParameterMember
                // ActionBodyParameterMember ( 'else' ... )?`, and
                // `ActionBodyParameter : ActionUsage = ... '{'
                // ActionBodyItem* '}'`. What a structured control node
                // writes in braces is one parameter handed to it, not
                // members of the node itself: `inputParameters()->size()
                // = 2` counts a `for` loop's sequence and its body,
                // however many statements the body is written with.
                let under = body_parameter(model, id, &child).unwrap_or(id);
                for member in child.children() {
                    // `loop { ... } until c;` is one node written as two
                    // statements: `WhileLoopNode : WhileLoopActionUsage =
                    // ... ( 'until' ExpressionParameterMember ';' )?`.
                    // What it asks belongs to the loop before it, and a
                    // second loop standing for it says the flow repeats
                    // twice over. That loop is the last thing built
                    // here: `then action aLoop while c { ... }` writes
                    // the succession as the statement and leaves the
                    // loop beside it.
                    if let Some(repeats) =
                        loop_before(model, under).filter(|_| closes_a_loop(&member))
                    {
                        reify_condition(model, &member, repeats);
                        continue;
                    }
                    let made = build_node(model, &member, Some(under), built);
                    // `connect ( cause1 ::> causer1, cause2 ::> causer2 )`
                    // writes each end in the list, named and referring
                    if child.kind() == PARAM_LIST
                        && kind.is_a(ElementKind::Connector)
                        && member.children().any(|it| it.kind() == REFERENCES)
                    {
                        if let Some(end) = made {
                            takes_the_end_role(model, end);
                        }
                    }
                }
            }
            PAYLOAD | PREFIX_METADATA => {
                build_node(model, &child, Some(id), built);
            }
            // `then action b;` writes the declaration inside the
            // succession it starts, but what it declares belongs to the
            // enclosing scope, not one level in. An anonymous prefix
            // wrapper does the same and never gets this far: it is not
            // an element, so its children were hoisted on the way in.
            DEFINITION | USAGE => {
                // The declaration after a cross feature is the end the
                // source declared: it stands where this element would
                // have, and owns the cross feature written before it.
                if crossed.as_ref().is_some_and(|it| *it == child) {
                    if let Some(end) = build_node(model, &child, owner, built) {
                        takes_the_cross_feature(model, end, id);
                    }
                    continue;
                }
                // `loop action charging { ... }` gives the body a name:
                // `ActionBodyParameter : ActionUsage = ( 'action'
                // UsageDeclaration? )? '{' ActionBodyItem* '}'`, and
                // `charging.monitor` is read through it. What a
                // structured node is handed belongs to it however it
                // was written; a step of the flow does not.
                let handed = handed_body(model.kind(id), &child);
                let parent = if handed || node.kind() != CONTROL_STMT {
                    Some(id)
                } else {
                    owner
                };
                let made = build_node(model, &child, parent, built);
                // `connect [1] lugNutPort ::> wheel.lugNutPort to ...`
                // writes the end itself: `ConnectorEnd : Feature = (
                // OwnedCrossMultiplicityMember )? ( declaredName = NAME
                // REFERENCES )? OwnedReferenceSubsetting`. The
                // declaration is the end the connector relates through,
                // and read as a member of it the connector related
                // nothing at all.
                if kind.is_a(ElementKind::Connector)
                    && child.children().any(|it| it.kind() == REFERENCES)
                {
                    if let Some(end) = made {
                        takes_the_end_role(model, end);
                    }
                }
                if let Some(parameter) = made.filter(|_| handed) {
                    model.set(parameter, "direction", Value::EnumLit("in"));
                    model.set(parameter, "isComposite", Value::Bool(false));
                }
            }
            _ => {}
        }
    }
    Some(id)
}

/// The loop an `until` written next closes, where the statement before
/// it left one.
fn loop_before(model: &Model, under: ElementId) -> Option<ElementId> {
    model
        .owned(under)
        .last()
        .copied()
        .filter(|&it| model.kind(it) == ElementKind::WhileLoopActionUsage)
}

/// Whether a statement is the `until` clause that closes the loop written
/// before it, rather than a loop of its own.
fn closes_a_loop(node: &SyntaxNode) -> bool {
    node.kind() == SyntaxKind::CONTROL_STMT && tokens(node).next() == Some(SyntaxKind::UNTIL_KW)
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
    // `part x [0..*]` writes the clause as a node of its own. A control
    // statement writes the same thing where an index would go -- `then
    // timeslice ownership[0..*]` -- and a two-ended range arrives whole
    // there rather than as bounds either side of a `..`, so it is opened
    // up to be read the one way.
    let written: Vec<sysml_syntax::SyntaxElement> =
        match node.children().find(|child| child.kind() == MULTIPLICITY) {
            Some(clause) => clause.children_with_tokens().collect(),
            None if node.kind() == CONTROL_STMT => {
                let Some(indexed) = node.children().find(|child| child.kind() == INDEX_EXPR) else {
                    return;
                };
                indexed
                    .children_with_tokens()
                    .skip_while(|part| part.kind() != L_BRACKET)
                    .flat_map(|part| match part.as_node() {
                        Some(range) if range.kind() == BINARY_EXPR => {
                            range.children_with_tokens().collect::<Vec<_>>()
                        }
                        _ => vec![part],
                    })
                    .collect()
            }
            None => return,
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
    for part in written {
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
            refers_through(model, bound, kind);
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
        // `validateFeatureValueIsInitial` -- "a FeatureValue that is
        // initial has a feature whose value can change". `:=` gives a
        // starting value rather than the value, which is what makes it
        // one.
        if model.kind(owner).feature("isVariable").is_some() {
            model.set(owner, "isVariable", Value::Bool(true));
        }
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
    refers_through(model, expression, kind);
    represent_textually(model, expression, written.text().to_string().trim());
    expression
}

/// Stand a `Membership` on a feature reference expression for what it
/// will turn out to refer to.
///
/// `deriveFeatureReferenceExpressionReferent` takes the *first* owned
/// membership that is not a parameter's, so the one holding the referent
/// has to come before whatever else the expression owns -- the text it
/// was written as, among other things. Name resolution fills in what it
/// relates once the name has been looked up; until then it relates
/// nothing, which is what an unresolved name amounts to.
fn refers_through(model: &mut Model, expression: ElementId, kind: ElementKind) {
    if kind == ElementKind::FeatureReferenceExpression {
        let membership = model.create(ElementKind::Membership);
        model.add_owned(expression, membership);
    }
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
            Value::String(sysml_syntax::unquote_string(token.text())),
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
///
/// `PayloadParameter : ReferenceUsage` and
/// `deriveAcceptActionUsagePayloadParameter` -- "the payloadParameter of
/// an AcceptActionUsage is its first parameter" -- so what waits is a
/// parameter of the node and not an action of its own.
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
    let payload = model.create(ElementKind::ReferenceUsage);
    model.add_owned(owner, payload);
    model.set(payload, "declaredName", Value::String(name));
    Some(payload)
}

/// Reify what a `send` or an `accept` acts on.
///
/// `SendNode : SendActionUsage = 'send' ArgumentMember ( 'via'
/// ArgumentMember )? ( 'to' ArgumentMember )?` -- each clause writes an
/// argument, which the standard keeps as an input parameter of the
/// action holding what was written as its value and reads back by
/// position: `senderArgument = argument(2)`, `receiverArgument =
/// argument(3)`. The parameters stand there whether or not the source
/// wrote a clause for each, which is what `validateSendActionParameters`
/// and `validateAcceptActionUsageParameters` say outright.
///
/// Without them the port a message goes out of is nowhere in the model,
/// so `via displayPort` named nothing and a name that stands for nothing
/// went unreported.
fn reify_action_arguments(
    model: &mut Model,
    node: &SyntaxNode,
    action: ElementId,
    kind: ElementKind,
    payload: Option<ElementId>,
) {
    // in the order the standard declares the parameters, against the
    // keyword the notation writes each after. The payload is the first
    // of them and is written after the keyword naming the action itself,
    // so no keyword introduces it.
    let slots: &[Option<SyntaxKind>] = match kind {
        ElementKind::SendActionUsage => &[None, Some(SyntaxKind::VIA_KW), Some(SyntaxKind::TO_KW)],
        ElementKind::AcceptActionUsage => &[None, Some(SyntaxKind::VIA_KW)],
        _ => return,
    };
    for (at, slot) in slots.iter().enumerate() {
        // the payload the `accept` clause named is that first parameter
        // rather than one standing beside it
        let parameter = match (at, payload) {
            (0, Some(payload)) => payload,
            _ => {
                let made = model.create(ElementKind::ReferenceUsage);
                model.add_owned(action, made);
                made
            }
        };
        model.set(parameter, "direction", Value::EnumLit("in"));
        let Some(written) = slot.and_then(|keyword| operand_after(node, keyword)) else {
            continue;
        };
        let membership = model.create(ElementKind::FeatureValue);
        model.add_owned(parameter, membership);
        model.set(membership, "featureWithValue", Value::Ref(parameter));
        let expression = value_expression(model, membership, &written);
        model.set(membership, "value", Value::Ref(expression));
    }
}

/// The reference a keyword introduces, where the statement writes one
/// directly after it.
fn operand_after(node: &SyntaxNode, keyword: SyntaxKind) -> Option<SyntaxNode> {
    let mut after = false;
    for element in node.children_with_tokens() {
        match element.as_token() {
            Some(token) if token.kind().is_trivia() => {}
            // any other keyword closes the slot the reference sits in
            Some(token) => after = token.kind() == keyword,
            None => {
                let child = element.into_node().expect("checked for a token above");
                if after {
                    return matches!(child.kind(), SyntaxKind::NAME_REF | SyntaxKind::PATH_EXPR)
                        .then_some(child);
                }
            }
        }
    }
    None
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

/// Keep what an `if` or a loop asks, as the text the author wrote it as.
///
/// `if hot > 100 then cool;` and `while hot > 0 { ... }` are about their
/// condition; a node holding only the body says a flow branches or
/// repeats without saying on what.
fn reify_condition(model: &mut Model, node: &SyntaxNode, id: ElementId) {
    use SyntaxKind::*;
    // `for i in xs` asks for `xs`: what comes before `in` is the variable
    // it binds, which `reify_loop_variable` declares. Kept whole, the
    // loop arrives asking for `i in xs`, which is a comparison.
    let mut asked = String::new();
    // What a node asks begins after the keyword that says which node
    // it is, and never before: `then while c { ... }` writes the flow
    // it continues first, and `action aLoop while c { ... }` its own
    // name. A `for` asks for what follows `in`.
    // `then if c { ... }` writes what it asks as the conditional
    // expression a guard is written as, all of it in one node.
    if let Some(written) = node.children().find(|it| it.kind() == COND_EXPR) {
        let text = written.text().to_string();
        return keep_condition(model, id, text.strip_prefix("if").unwrap_or(&text).trim());
    }
    let mut started = false;
    for part in node.children_with_tokens() {
        if !started {
            started = matches!(part.kind(), IN_KW | WHILE_KW | UNTIL_KW | LOOP_KW | IF_KW);
            continue;
        }
        match part.kind() {
            // what follows the condition is the body, and a declaration
            // is the body it is handed rather than part of what it asks
            // -- `loop action charging { ... }` asks nothing
            THEN_KW | ELSE_KW | L_BRACE | SEMICOLON | BODY | DEFINITION | USAGE => break,
            kind if kind.is_trivia() => continue,
            _ => {}
        }
        let written = match part {
            sysml_syntax::SyntaxElement::Node(node) => node.text().to_string(),
            sysml_syntax::SyntaxElement::Token(token) => token.text().to_string(),
        };
        if !asked.is_empty() {
            asked.push(' ');
        }
        asked.push_str(written.trim());
    }
    let asked = asked.trim();
    if asked.is_empty() {
        // `WhileLoopNode = ... ( 'while' ExpressionParameterMember |
        // 'loop' EmptyParameterMember ) ...`: a bare `loop` asks
        // nothing and is handed an empty parameter all the same --
        // `EmptyUsage : ReferenceUsage = {}` -- which is what keeps it
        // the two the constraint counts.
        if has_token(node, LOOP_KW) {
            let empty = model.create(ElementKind::ReferenceUsage);
            model.add_owned(id, empty);
            model.set(empty, "direction", Value::EnumLit("in"));
        }
        return;
    }
    keep_condition(model, id, asked);
}

/// Keep what a structured control node asks, as the text it was written
/// as.
///
/// `IfNode = 'if' ExpressionParameterMember ...` and its loop kin: what
/// the node asks is handed to it, and `inputParameters()` is what the
/// constraints about a structured control node count.
fn keep_condition(model: &mut Model, id: ElementId, asked: &str) {
    let condition = model.create(ElementKind::Expression);
    model.add_owned(id, condition);
    // `ExpressionParameterMember : ParameterMembership` -- what the
    // node asks is handed to it as a parameter, not held as a result.
    // A `ResultExpressionMembership` is owned by a function or an
    // expression, which a loop node is neither, and
    // `validateResultExpressionMembershipOwningType` says so.
    model.set(condition, "direction", Value::EnumLit("in"));
    represent_textually(model, condition, asked);
}

/// Declare the variable a `for` loop iterates with.
///
/// `ForVariableDeclarationMember : FeatureMembership = ownedRelatedElement
/// += ForVariableDeclaration` -- the loop owns it, and the body refers to
/// it by name, so a loop without it leaves `in power = vehiclePower;`
/// naming nothing.
fn reify_loop_variable(model: &mut Model, node: &SyntaxNode, id: ElementId) {
    use SyntaxKind::*;
    let mut named = None;
    for part in node.children_with_tokens() {
        match part.kind() {
            IN_KW => break,
            NAME | NAME_REF | QUALIFIED_NAME => {
                named = part.into_node().and_then(|child| {
                    child
                        .descendants_with_tokens()
                        .filter_map(|e| e.into_token())
                        .find(|t| matches!(t.kind(), IDENT | UNRESTRICTED_NAME))
                        .map(|t| unquote(t.text()))
                });
            }
            _ => {}
        }
    }
    let Some(named) = named else { return };
    let variable = model.create(ElementKind::ReferenceUsage);
    model.add_owned(id, variable);
    model.set(variable, "declaredName", Value::String(named));
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
    // `do send 1 to p` is a send action like any other, and
    // `validateSendActionParameters` counts the three parameters it is
    // handed whether the statement wrote a clause for each or not.
    reify_action_arguments(model, node, effect, kind, None);
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
    if let Some(declaration) = tokens(node).find_map(control_node_kind) {
        return Some(declaration);
    }
    // `accept Go then s2;` in a state body is a transition out of the
    // state it is written in: `TargetTransitionUsage : TransitionUsage =
    // ... TriggerActionMember ... 'then' TransitionSuccessionMember`.
    // Read as the succession its `then` would otherwise make, the trigger
    // has nothing standing for it and the transition nothing to be found
    // by.
    //
    // `if x then a;` after a decision node is the same shape:
    // `GuardedTargetSuccession : TransitionUsage = GuardExpressionMember
    // 'then' TransitionSuccessionMember`. An `IfNode` writes its
    // branches in braces and no `then` at all, so the keyword is what
    // tells a branch of the flow from a structured node.
    if is_target_transition(node) {
        return Some(ElementKind::TransitionUsage);
    }
    // `then if monitor.charge < 100 { ... }` declares the branch as much
    // as `then while c { ... }` declares the loop. The parser reads the
    // `if c` after a leading keyword as the conditional expression a
    // transition guard is written as, and what tells the two apart is
    // the braces: an `IfNode` writes its branches in them, and a guard
    // is followed by the `then` it guards.
    if node.children().any(|it| it.kind() == SyntaxKind::COND_EXPR)
        && node
            .children()
            .any(|it| it.kind() == SyntaxKind::BODY && has_token(&it, SyntaxKind::L_BRACE))
    {
        return Some(ElementKind::IfActionUsage);
    }
    // `then send new S() via p;` declares the action as much as `then
    // merge continue;` declares the node, and for the same reason the
    // declaration wins: read as the succession alone, the action the
    // source wrote is in the model nowhere at all. What the statement
    // continues from is then the step before it, which is how a
    // succession with one end written reads anyway.
    // and only where the action is the statement's own: `transition t1
    // first a do send 1 to p then b` writes one as the effect it carries
    // across, and the transition is what the statement declares.
    let continues = (has_token(node, SyntaxKind::THEN_KW) || has_token(node, SyntaxKind::FIRST_KW))
        && !has_token(node, SyntaxKind::TRANSITION_KW)
        && !has_token(node, SyntaxKind::DO_KW);
    if continues {
        if let Some(declared) = tokens(node).find_map(|token| match token {
            SyntaxKind::SEND_KW => Some(ElementKind::SendActionUsage),
            SyntaxKind::ACCEPT_KW => Some(ElementKind::AcceptActionUsage),
            SyntaxKind::ASSIGN_KW => Some(ElementKind::AssignmentActionUsage),
            SyntaxKind::TERMINATE_KW => Some(ElementKind::TerminateActionUsage),
            // `then while c { ... }` and `then for i in xs { ... }`
            // declare the loop as much as `then merge continue;`
            // declares the node. Read in the order they are written the
            // leading `then` answers first, and the loop the source
            // wrote is in the model nowhere at all -- with the body it
            // was handed.
            SyntaxKind::WHILE_KW | SyntaxKind::UNTIL_KW | SyntaxKind::LOOP_KW => {
                Some(ElementKind::WhileLoopActionUsage)
            }
            SyntaxKind::FOR_KW => Some(ElementKind::ForLoopActionUsage),
            // `then event server.publish_request[1];` declares the
            // occurrence the flow runs into, and `then perform a;` and
            // `then include u;` the same. Read as the succession alone,
            // the step the source wrote is in the model nowhere and the
            // succession runs into nothing.
            SyntaxKind::EVENT_KW => Some(ElementKind::EventOccurrenceUsage),
            SyntaxKind::PERFORM_KW => Some(ElementKind::PerformActionUsage),
            SyntaxKind::INCLUDE_KW => Some(ElementKind::IncludeUseCaseUsage),
            _ => None,
        }) {
            return Some(declared);
        }
    }
    let node_kind = tokens(node).find_map(|token| match token {
        SyntaxKind::TRANSITION_KW => Some(ElementKind::TransitionUsage),
        // `while x > 0 { ... }` and `for t in xs { ... }` are action
        // usages of their own (`WhileLoopActionUsage`,
        // `ForLoopActionUsage`). Without them the statement built
        // nothing, and what the loop body declared went with it.
        // `WhileLoopNode : WhileLoopActionUsage = ... ( 'while'
        // ExpressionParameterMember | 'loop' EmptyParameterMember ) ...`
        // -- a bare `loop { ... }` is the same node, asking nothing
        SyntaxKind::WHILE_KW | SyntaxKind::UNTIL_KW | SyntaxKind::LOOP_KW => {
            Some(ElementKind::WhileLoopActionUsage)
        }
        SyntaxKind::FOR_KW => Some(ElementKind::ForLoopActionUsage),
        SyntaxKind::IF_KW => Some(ElementKind::IfActionUsage),
        // `terminate c1;` -- unlike `merge m`, the name of a terminate
        // node comes before the keyword, so it is not one of the
        // declaring keywords a name is looked for after
        SyntaxKind::TERMINATE_KW => Some(ElementKind::TerminateActionUsage),
        // `send x via p;`, `accept sig : Sig;` and `assign x := 1;`
        // are action nodes of their own -- `SendNode :
        // SendActionUsage`, `AcceptNode : AcceptActionUsage`,
        // `AssignmentNode : AssignmentActionUsage` -- and each of them
        // was building nothing at all
        SyntaxKind::SEND_KW => Some(ElementKind::SendActionUsage),
        SyntaxKind::ACCEPT_KW => Some(ElementKind::AcceptActionUsage),
        SyntaxKind::ASSIGN_KW => Some(ElementKind::AssignmentActionUsage),
        // `first x;` on its own is `InitialNodeMember : FeatureMembership
        // = MemberPrefix 'first' memberFeature = [QualifiedName]`: it
        // names which step comes first and writes no flow at all. The
        // flow is what a `then` writes -- `TargetSuccession :
        // SuccessionAsUsage = SourceEndMember 'then' ConnectorEndMember`
        // -- so `first a; then b;` is one succession and not two. Read
        // as a succession as well, the `first` took the flow its `then`
        // writes and left that one relating its own declaration to
        // itself.
        SyntaxKind::FIRST_KW if !has_token(node, SyntaxKind::THEN_KW) => {
            // A `Membership` rather than the `FeatureMembership` the
            // grammar names: this one refers to a member declared
            // elsewhere, the way an alias does, and the metaclasses
            // that fold into ownership are the ones an interchange
            // synthesizes rather than writes.
            Some(ElementKind::Membership)
        }
        SyntaxKind::FIRST_KW | SyntaxKind::THEN_KW => Some(ElementKind::SuccessionAsUsage),
        _ => None,
    });
    node_kind.or_else(|| {
        // `entry performSelfTest { ... }` and `do providePower;` are the
        // action itself, named as the one it performs
        // (`StatePerformActionUsage : PerformActionUsage`) rather than as
        // a wrapper around a declaration. Without this the subaction of
        // every state written that way was dropped, body and all.
        let subaction = matches!(
            tokens(node).next(),
            Some(SyntaxKind::ENTRY_KW | SyntaxKind::DO_KW | SyntaxKind::EXIT_KW)
        );
        let nests = node
            .children()
            .any(|child| matches!(child.kind(), SyntaxKind::DEFINITION | SyntaxKind::USAGE));
        let performs = node
            .children()
            .any(|child| child.kind() == SyntaxKind::NAME_REF);
        if subaction && !nests {
            // `entry;` on its own still declares an action -- an empty
            // one -- and `entry; then off;` is a succession out of it.
            // Building nothing left the succession relating one thing,
            // which `validateConnectorRelatedFeatures` says a concrete
            // connector cannot do.
            return Some(if performs {
                ElementKind::PerformActionUsage
            } else {
                ElementKind::ActionUsage
            });
        }
        None
    })
}

/// Whether a control statement is a transition rather than the
/// succession its `then` reads as.
///
/// `TargetTransitionUsage` puts a trigger, a guard or both before the
/// `then`; a bare `then b` and a `first a then b` are successions. A
/// guard alone is a transition too -- `GuardedTargetSuccession :
/// TransitionUsage = GuardExpressionMember 'then'
/// TransitionSuccessionMember` -- and what tells `if hot then cool;`
/// from the `if` node of a structured body is the `then`, which a node
/// writing its branches in braces does not have.
fn is_target_transition(node: &SyntaxNode) -> bool {
    // `else A3;` writes the branch a guard did not take, and writes no
    // `then` at all: `DefaultTargetSuccession : TransitionUsage =
    // 'else' TransitionSuccessionMember`. The `else` of a structured
    // `if c { ... } else { ... }` is a token of that one statement, so
    // a statement of its own that begins with the keyword is this.
    if matches!(tokens(node).next(), Some(SyntaxKind::ELSE_KW)) {
        return true;
    }
    tokens(node)
        .take_while(|token| *token != SyntaxKind::THEN_KW)
        .any(|token| matches!(token, SyntaxKind::ACCEPT_KW | SyntaxKind::IF_KW))
        && has_token(node, SyntaxKind::THEN_KW)
}

/// The relationship a `RELATION_STMT` reifies, keyed on the keyword that
/// says which two things it relates.
///
/// `specialization s subtype A :> B;` writes as a statement of its own
/// what `classifier A :> B` writes as a clause, and KerML gives each form
/// the same metaclass. The leading `specialization`, `disjoining`,
/// `conjugation` and `inverting` only introduce a name for it, so the
/// keyword after them is what says which relationship it is.
fn relation_kind(node: &SyntaxNode) -> Option<ElementKind> {
    relation_tokens(node).find_map(|token| match token {
        SyntaxKind::SUBTYPE_KW => Some(ElementKind::Specialization),
        SyntaxKind::SUBCLASSIFIER_KW => Some(ElementKind::Subclassification),
        SyntaxKind::SUBSET_KW => Some(ElementKind::Subsetting),
        SyntaxKind::REDEFINITION_KW => Some(ElementKind::Redefinition),
        SyntaxKind::TYPING_KW => Some(ElementKind::FeatureTyping),
        SyntaxKind::CONJUGATE_KW => Some(ElementKind::Conjugation),
        SyntaxKind::DISJOINT_KW => Some(ElementKind::Disjoining),
        SyntaxKind::INVERSE_KW => Some(ElementKind::FeatureInverting),
        SyntaxKind::FEATURING_KW => Some(ElementKind::TypeFeaturing),
        _ => None,
    })
}

/// The keywords a relationship statement writes outside its body.
///
/// `disjoining d disjoint A from B;` puts `disjoint` and the first of the
/// two types it relates in a clause of their own, so the keyword that
/// says which relationship this is sits one level in.
fn relation_tokens(node: &SyntaxNode) -> impl Iterator<Item = SyntaxKind> {
    node.children_with_tokens()
        .flat_map(|part| match part {
            sysml_syntax::SyntaxElement::Token(token) => vec![token.kind()],
            sysml_syntax::SyntaxElement::Node(child) if child.kind() != SyntaxKind::BODY => {
                tokens(&child).collect()
            }
            sysml_syntax::SyntaxElement::Node(_) => Vec::new(),
        })
        .collect::<Vec<_>>()
        .into_iter()
}

/// The flags the notation writes as a keyword, and the property the
/// standard keeps each on.
///
/// `Message : FlowUsage = OccurrenceUsagePrefix 'message' ... {
/// isAbstract = true }` -- the specification has no metaclass of its own
/// for a message and marks it this way instead, which is what tells
/// `message m from a to b` from `flow f from a to b`.
const KEYWORD_FLAGS: [(SyntaxKind, &str); 12] = [
    (SyntaxKind::ABSTRACT_KW, "isAbstract"),
    (SyntaxKind::MESSAGE_KW, "isAbstract"),
    (SyntaxKind::CONSTANT_KW, "isConstant"),
    (SyntaxKind::CONST_KW, "isConstant"),
    (SyntaxKind::DERIVED_KW, "isDerived"),
    (SyntaxKind::INDIVIDUAL_KW, "isIndividual"),
    (SyntaxKind::ORDERED_KW, "isOrdered"),
    (SyntaxKind::PARALLEL_KW, "isParallel"),
    (SyntaxKind::PORTION_KW, "isPortion"),
    (SyntaxKind::STANDARD_KW, "isStandard"),
    (SyntaxKind::VAR_KW, "isVariable"),
    (SyntaxKind::VARIATION_KW, "isVariation"),
];

/// Every flag this builder reads off the source for any metaclass that
/// declares it.
///
/// The distinction this draws is what lets a reader tell one kind of
/// silence from another. An element of a metaclass that declares one of
/// these and carries no value for it is one whose source said nothing,
/// so what the specification declares as the property's default is the
/// answer. Any other property missing is this builder not building it,
/// which says nothing about the model at all.
pub const BUILT_FLAGS: [&str; 13] = [
    "isAbstract",
    "isConstant",
    "isDerived",
    "isEnd",
    "isIndividual",
    "isNegated",
    "isOrdered",
    "isParallel",
    "isPortion",
    "isStandard",
    "isUnique",
    "isVariable",
    "isVariation",
];

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
        // `then timeslice ownership[0..*] ordered { ... }` is written
        // flat the same way, and read as the succession alone it took
        // the portion's body with it: what the body declared came out
        // owned by the step rather than by the portion, which is what
        // `validateOccurrenceUsagePortionKind` -- a portion is owned by
        // an occurrence -- says of the result.
        SyntaxKind::SNAPSHOT_KW | SyntaxKind::TIMESLICE_KW => Some(ElementKind::OccurrenceUsage),
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
    // the keywords a relationship statement writes between its name and
    // the two things it relates
    let relates = |kind| {
        matches!(
            kind,
            SyntaxKind::SUBTYPE_KW
                | SyntaxKind::SUBCLASSIFIER_KW
                | SyntaxKind::SUBSET_KW
                | SyntaxKind::REDEFINITION_KW
                | SyntaxKind::TYPING_KW
                | SyntaxKind::CONJUGATE_KW
                | SyntaxKind::DISJOINT_KW
                | SyntaxKind::INVERSE_KW
                | SyntaxKind::OF_KW
                | SyntaxKind::FROM_KW
        )
    };
    let introduces_operands = |kind| {
        matches!(
            kind,
            SyntaxKind::FIRST_KW
                | SyntaxKind::THEN_KW
                | SyntaxKind::ACCEPT_KW
                | SyntaxKind::SEND_KW
                | SyntaxKind::TERMINATE_KW
                | SyntaxKind::ASSIGN_KW
                // `do providePower;` performs the action it names; the
                // name is what it is about, not what it is called
                | SyntaxKind::ENTRY_KW
                | SyntaxKind::DO_KW
                | SyntaxKind::EXIT_KW
        )
    };
    // `specialization s subtype A :> B;` and `featuring feat of f by A;`
    // name themselves before the clause that says what they relate;
    // `disjoint A from B;` leaves the place empty and stays unnamed.
    if node.kind() == SyntaxKind::RELATION_STMT {
        return node
            .children_with_tokens()
            .take_while(|part| !relates(part.kind()) && part.kind() != SyntaxKind::RELATION)
            .filter_map(|part| part.into_node())
            .find(|child| child.kind() == SyntaxKind::NAME_REF)
            .and_then(|name| Some(unquote(name.first_token()?.text())));
    }
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
    // `for t in 1..3` and `if hot > 100 then cool` lead with the loop
    // variable and the condition; taking either for a name would leave
    // the statement saying nothing about what it asks
    if tokens(node).any(|token| {
        matches!(
            token,
            SyntaxKind::FOR_KW | SyntaxKind::IF_KW | SyntaxKind::WHILE_KW | SyntaxKind::UNTIL_KW
        )
    }) {
        return None;
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
        // `timeslice ownership[0..*]` writes the multiplicity where an
        // index would go, so the name it declares is inside that
        let named = match child.kind() {
            SyntaxKind::NAME_REF => Some(child),
            SyntaxKind::INDEX_EXPR => child
                .children()
                .find(|part| part.kind() == SyntaxKind::NAME_REF),
            _ => None,
        };
        if let Some(named) = named.filter(|_| reached_declaration) {
            if bare_then {
                return None;
            }
            return Some(unquote(named.first_token()?.text()));
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
    // `expose` is not an import: a view exposes what it shows, and the
    // standard has metaclasses of its own for it
    // (`exposes-compartment-element = MembershipExpose | NamespaceExpose`).
    match (node.kind() == EXPOSE, wildcard) {
        (true, true) => ElementKind::NamespaceExpose,
        (true, false) => ElementKind::MembershipExpose,
        (false, true) => ElementKind::NamespaceImport,
        (false, false) => ElementKind::MembershipImport,
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
        // `IndividualDefinition : OccurrenceDefinition =
        // BasicDefinitionPrefix? isIndividual ?= 'individual'
        // DefinitionExtensionKeyword* 'def' Definition` -- `individual
        // def IO1;` names no kind and is an occurrence definition all
        // the same, which is what `isIndividual` is declared on. Read as
        // a definition of unspecified kind it was not individual at
        // all, and the usages typed by it had no individual definition
        // to have.
        _ if has_token(node, INDIVIDUAL_KW) => "OccurrenceDefinition",
        // What is left is `#service def X`, which takes its kind from the
        // user-defined keyword rather than naming one: a SysML definition
        // of unspecified kind, not a bare KerML classifier. Every other
        // definition the parser builds carries one of the keywords above.
        _ => "Definition",
    };
    kind_or(name, ElementKind::Classifier)
}

fn usage_kind(
    node: &SyntaxNode,
    owner: Option<ElementId>,
    model: &Model,
    dialect: sysml_syntax::Dialect,
) -> ElementKind {
    use SyntaxKind::*;
    let kws = kind_keywords(node);
    let scope: Vec<SyntaxKind> = scope_tokens(node).chain(leading_keywords(node)).collect();
    // `assert not satisfy R by p;` asserts a satisfaction, which the
    // grammar makes a `SatisfyRequirementUsage`; `assert` says how
    // strongly it is claimed, not what kind of usage it is. Reading the
    // keywords in the order they were written answers with whichever came
    // first, and `assert` always comes first.
    if scope.contains(&SATISFY_KW) {
        return kind_or("SatisfyRequirementUsage", ElementKind::Usage);
    }
    // adapter keywords take precedence: `perform action a` is a
    // PerformActionUsage, not an ActionUsage
    for token in scope {
        let candidate = match token {
            PERFORM_KW => Some("PerformActionUsage"),
            EXHIBIT_KW => Some("ExhibitStateUsage"),
            EVENT_KW => Some("EventOccurrenceUsage"),
            INCLUDE_KW => Some("IncludeUseCaseUsage"),
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
            // `action a1 assign x := 2;` assigns to `x`; it is not an
            // action that happens to have a value of its own
            // (`AssignmentNode : AssignmentActionUsage`)
            ASSIGN_KW => Some("AssignmentActionUsage"),
            // roles that fix the metaclass without a kind keyword of
            // their own: an actor or stakeholder is a part, an objective
            // a requirement
            ACTOR_KW | STAKEHOLDER_KW => Some("PartUsage"),
            // `render asTree : Tree;` is a rendering usage; without the
            // keyword it arrives as the bare reference a usage with no
            // kind keyword would
            RENDER_KW => Some("RenderingUsage"),
            // `PortionUsage : OccurrenceUsage = ... portionKind =
            // PortionKind ...` -- the portion keyword stands where a kind
            // keyword would, and without it `snapshot s : O;` arrives as
            // the bare reference a usage with no keyword at all would
            // `IndividualUsage : OccurrenceUsage = ... isIndividual ?=
            // 'individual' ...` the same way. Only where there is no kind
            // keyword: `individual part x : X;` is a part that is one
            // individual, and it says so before it says `part`.
            SNAPSHOT_KW | TIMESLICE_KW | INDIVIDUAL_KW if kws.is_empty() => Some("OccurrenceUsage"),
            OBJECTIVE_KW => Some("RequirementUsage"),
            // `RequirementConstraintUsage : ConstraintUsage`,
            // `FramedConcernUsage : ConcernUsage` and
            // `RequirementVerificationUsage : RequirementUsage` -- each
            // may be written as a bare reference to what it names, and
            // read as one it arrived as the plain reference a usage
            // with no keyword would. A reference is not composite, and
            // `validateRequirementConstraintMembershipIsComposite` says
            // what a requirement frames or assumes must be.
            ASSUME_KW | REQUIRE_KW => Some("ConstraintUsage"),
            FRAME_KW => Some("ConcernUsage"),
            VERIFY_KW => Some("RequirementUsage"),
            _ => None,
        };
        if let Some(name) = candidate {
            return kind_or(name, ElementKind::Usage);
        }
    }
    // `action aLoop while c { ... }` names the loop it declares:
    // `WhileLoopNode : WhileLoopActionUsage = ActionNodePrefix ( 'while'
    // ... ) ...`, where the `action` before it introduces a name rather
    // than a plain action. Read as one, the loop is in the model
    // nowhere and the `until` after it stands for a second.
    if let Some(loops) = tokens(node).find_map(|token| match token {
        WHILE_KW | UNTIL_KW | LOOP_KW => Some(ElementKind::WhileLoopActionUsage),
        FOR_KW => Some(ElementKind::ForLoopActionUsage),
        _ => None,
    }) {
        return loops;
    }
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
        // `EnumeratedValue : EnumerationUsage = 'enum'? Usage` -- a value
        // written in an enumeration body names no kind, and reading it as
        // a plain reference leaves the enumeration with no values at all.
        _ if owner
            .is_some_and(|owner| model.kind(owner).is_a(ElementKind::EnumerationDefinition)) =>
        {
            "EnumerationUsage"
        }
        // KerML lets a feature leave out the `feature` keyword where a
        // modifier stands in front of it -- `composite tanks : Tank;`,
        // `in test : Boolean;`, `end guardedLink [0..1]` -- and it is a
        // plain feature. `ReferenceUsage` is SysML's, and reading one
        // into a KerML file makes every such declaration referential
        // whatever it says.
        _ if dialect == sysml_syntax::Dialect::KerML => "Feature",
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
    // `do providePower;` is the subaction itself rather than a wrapper
    // around one, so its keyword is on the statement. It only says which
    // subaction it is when it leads: the `do` in `transition ... do send
    // x then b` says what that transition does on the way across.
    if node.kind() == CONTROL_STMT {
        match tokens(node).next() {
            Some(ENTRY_KW) => return Some(Role::Entry),
            Some(DO_KW) => return Some(Role::Do),
            Some(EXIT_KW) => return Some(Role::Exit),
            _ => {}
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
            // `ViewRenderingMembership` -- how a view is drawn is a
            // member of it, and the standard has a membership of its own
            // for saying so
            RENDER_KW => Some(Role::Render),
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
fn is_composite(node: &SyntaxNode, kind: ElementKind, owning: ElementKind) -> bool {
    if scope_has(node, SyntaxKind::REF_KW)
        || kind == ElementKind::ReferenceUsage
        || has_token(node, SyntaxKind::END_KW)
        || declared_direction(node).is_some()
    {
        return false;
    }
    // `validateAttributeUsageIsReference: isReference` -- "An
    // AttributeUsage is always referential", and the same of an
    // `EventOccurrenceUsage`. A value is not something its owner is made
    // of, whether or not the source wrote `ref`.
    if kind.is_a(ElementKind::AttributeUsage) || kind.is_a(ElementKind::EventOccurrenceUsage) {
        return false;
    }
    // The two notations default the other way about. KerML writes
    // `composite feature ...` where it means one, and the metamodel
    // declares `isComposite = false` for everything else; SysML's `part
    // wheel;` is what its owner is made of unless it says `ref`. So a
    // feature declared in KerML is composite only where it says so,
    // and a usage is composite unless it says otherwise.
    if !kind.is_a(ElementKind::Usage) {
        return has_token(node, SyntaxKind::COMPOSITE_KW);
    }
    // `validateAttributeDefinitionFeatures` and its usage twin --
    // "the features of an attribute are all referential". A data value
    // has no parts, so nothing declared inside one is something it is
    // made of, whatever kind that thing is: `attribute def
    // SampledFunction { assert constraint { ... } }` asserts something
    // about a sampled function rather than adding a part to it.
    if owning.is_a(ElementKind::DataType) || owning.is_a(ElementKind::AttributeUsage) {
        return false;
    }
    if owning.is_a(ElementKind::PortDefinition) || owning.is_a(ElementKind::PortUsage) {
        return kind.is_a(ElementKind::PortUsage);
    }
    // `validatePortUsageIsReference` -- a port owned by anything that is
    // not itself a port is referential. `part def P { port p; }` says
    // where a P connects, not what one is made of.
    if kind.is_a(ElementKind::PortUsage) {
        return false;
    }
    true
}

/// Stand the declaration after a cross feature up as the end, and give
/// it the cross feature written in front of it.
///
/// `validateFeatureEndNotDerivedAbstractCompositeOrPortion` -- an end is
/// what a link relates, not something its association is made of, and
/// `end inCart[0..1] item cart : Cart;` would otherwise leave `cart`
/// composite for having been written as a usage.
fn takes_the_cross_feature(model: &mut Model, end: ElementId, cross: ElementId) {
    takes_the_end_role(model, end);
    model.add_owned(end, cross);
}

/// Say that a declaration is one of the ends its connector relates.
///
/// `validateFeatureEndNotDerivedAbstractCompositeOrPortion` -- an end is
/// what a link relates, not something its connector is made of.
fn takes_the_end_role(model: &mut Model, end: ElementId) {
    if model.kind(end).feature("isEnd").is_some() {
        model.set(end, "isEnd", Value::Bool(true));
    }
    if model.kind(end).feature("isComposite").is_some() {
        model.set(end, "isComposite", Value::Bool(false));
    }
}

/// The end declaration a cross feature was written in front of.
///
/// `end owningEntities[1..*] feature owner : LegalEntity;` -- what
/// stands between the `end` and the declaration after it is the cross
/// feature (`EndFeaturePrefix ( ownedRelationship +=
/// OwnedCrossFeatureMember )? FeatureDeclaration`), and the declaration
/// is the end itself. Written without one, `end feature owner :
/// LegalEntity;` puts the keyword in this node and declares no nested
/// element at all.
fn crossing_declaration(node: &SyntaxNode) -> Option<SyntaxNode> {
    if node.kind() != SyntaxKind::USAGE || !has_token(node, SyntaxKind::END_KW) {
        return None;
    }
    node.children()
        .find(|it| matches!(it.kind(), SyntaxKind::DEFINITION | SyntaxKind::USAGE))
}

/// Whether a node writes its flow in braces rather than in the
/// statement itself.
fn structured_node(kind: ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::IfActionUsage
            | ElementKind::WhileLoopActionUsage
            | ElementKind::ForLoopActionUsage
    )
}

/// Whether a declaration written inside a control statement is the body
/// the node is handed rather than a step beside it.
///
/// `ActionBodyParameter : ActionUsage = ( 'action' UsageDeclaration? )?
/// '{' ActionBodyItem* '}'` -- the braces are what make it a body, so a
/// declaration ending in `;` is a step of the flow whatever it is
/// written after.
fn handed_body(kind: ElementKind, child: &SyntaxNode) -> bool {
    structured_node(kind)
        && child
            .children()
            .any(|it| it.kind() == SyntaxKind::BODY && has_token(&it, SyntaxKind::L_BRACE))
}

/// The one parameter a structured control node's braces stand for.
///
/// `if c { a; b; }` hands the node a single body, not two members of
/// its own, and the same of a loop's. A body written as `;` rather than
/// in braces stands for nothing and is not one.
fn body_parameter(model: &mut Model, id: ElementId, body: &SyntaxNode) -> Option<ElementId> {
    if !structured_node(model.kind(id)) || !has_token(body, SyntaxKind::L_BRACE) {
        return None;
    }
    let parameter = model.create(ElementKind::ActionUsage);
    model.add_owned(id, parameter);
    // Handed to the node rather than part of it:
    // `ActionBodyParameterMember : ParameterMembership`, and
    // `validateUsageIsReferential` says what is handed to a behaviour is
    // not something it is made of.
    model.set(parameter, "direction", Value::EnumLit("in"));
    model.set(parameter, "isComposite", Value::Bool(false));
    Some(parameter)
}

/// The direction a feature was declared with (`in`, `out`, `inout`).
fn declared_direction(node: &SyntaxNode) -> Option<&'static str> {
    use SyntaxKind::*;
    // `for i in xs { ... }` writes `in` to say what it iterates over, not
    // which way anything is passed. A control statement declares no
    // feature of its own, so it has no direction to read.
    if node.kind() == CONTROL_STMT {
        return None;
    }
    with_wrapper(node).find_map(|scope| {
        tokens(&scope).find_map(|token| match token {
            INOUT_KW => Some("inout"),
            IN_KW => Some("in"),
            OUT_KW => Some("out"),
            _ => None,
        })
    })
}

/// The keywords a statement wrote immediately before the declaration it
/// wraps.
///
/// `then event occurrence b;` leaves `event` on the succession and
/// `occurrence b` one level in, so the declaration cannot see the keyword
/// that says what it is. Only the run directly before it counts: in
/// `accept x then send y;` what precedes `y` is `send`, not `accept`.
fn leading_keywords(node: &SyntaxNode) -> impl Iterator<Item = SyntaxKind> {
    let mut leading = Vec::new();
    if let Some(parent) = node
        .parent()
        .filter(|p| p.kind() == SyntaxKind::CONTROL_STMT)
    {
        for part in parent.children_with_tokens() {
            if part.as_node() == Some(node) {
                break;
            }
            match part {
                sysml_syntax::SyntaxElement::Node(_) => leading.clear(),
                sysml_syntax::SyntaxElement::Token(token) if !token.kind().is_trivia() => {
                    leading.push(token.kind())
                }
                _ => {}
            }
        }
    }
    leading.into_iter()
}

/// Whether this node is an anonymous prefix wrapper: a declaration that
/// carries only the keywords written before another declaration, which is
/// its one child.
///
/// `variant part optA;` and `in event occurrence ieo;` parse this way. The
/// wrapper names nothing and stands for nothing; what it says describes
/// the declaration it wraps.
///
/// A connector end really does own what it nests, though -- `end [1]
/// feature transferTarget references target;` is an anonymous end whose
/// feature is its own member -- so `end` is never a wrapper.
fn is_prefix_wrapper(node: &SyntaxNode) -> bool {
    use SyntaxKind::*;
    matches!(node.kind(), DEFINITION | USAGE)
        && !has_token(node, END_KW)
        && declared_name(node).is_none()
        && statement_declared_name(node).is_none()
        && node
            .children()
            .any(|child| matches!(child.kind(), DEFINITION | USAGE))
}

/// The node and, when the node was hoisted out of an anonymous wrapper
/// (`variant part optA;` parses as a wrapper around `part optA`), the
/// wrapper too -- the keywords that describe the member sit on it.
fn with_wrapper(node: &SyntaxNode) -> impl Iterator<Item = SyntaxNode> {
    let wrapper = node.parent().filter(is_prefix_wrapper);
    std::iter::once(node.clone()).chain(wrapper)
}

/// Every keyword that describes this declaration, wherever it was
/// written: on the declaration itself or on the wrapper around it.
fn scope_tokens(node: &SyntaxNode) -> impl Iterator<Item = SyntaxKind> {
    with_wrapper(node)
        .flat_map(|scope| tokens(&scope).collect::<Vec<_>>())
        .collect::<Vec<_>>()
        .into_iter()
}

fn scope_has(node: &SyntaxNode, kind: SyntaxKind) -> bool {
    scope_tokens(node).any(|token| token == kind)
}

fn kind_keywords(node: &SyntaxNode) -> Vec<SyntaxKind> {
    tokens(node).filter(|k| k.is_def_kind_kw()).collect()
}

fn has_token(node: &SyntaxNode, kind: SyntaxKind) -> bool {
    tokens(node).any(|k| k == kind)
}

fn declared_name(node: &SyntaxNode) -> Option<String> {
    // `connector eng to tanks.main1;` names no connector.
    // `BinaryConnectorDeclaration : Connector = ( FeatureDeclaration?
    // 'from' | isSufficient ?= 'all' 'from'? )? ConnectorEndMember 'to'
    // ConnectorEndMember` writes a declaration only in front of a
    // `from`, so what follows the keyword without one is the end the
    // connector runs from. Read as a name, `Vehicle1` came to have two
    // members called `eng`, and the connector related one thing.
    if names_an_end(node) {
        return None;
    }
    let name = node
        .children()
        .find(|c| c.kind() == SyntaxKind::NAME)?
        .first_token()?;
    Some(unquote(name.text()))
}

/// Whether the name after `connector` or `binding` is an end rather
/// than a name of the element's own.
///
/// Both write their declaration only in front of the keyword that
/// introduces the first end -- `from` for a connector, `bind` or `of`
/// for a binding -- so `connector eng to tanks.main1;` and `binding a =
/// b;` name nothing. The n-ary connector form writes its ends in
/// parentheses and may be named without a `from`, and a binding may
/// write a declaration and no ends at all (`binding bi { ... }`), so
/// the `to` and the `=` are what tell those apart.
fn names_an_end(node: &SyntaxNode) -> bool {
    has_token(node, SyntaxKind::CONNECTOR_KW)
        && has_token(node, SyntaxKind::TO_KW)
        && !has_token(node, SyntaxKind::FROM_KW)
        || has_token(node, SyntaxKind::BINDING_KW)
            && !has_token(node, SyntaxKind::BIND_KW)
            && !has_token(node, SyntaxKind::OF_KW)
            && node.children().any(|it| it.kind() == SyntaxKind::VALUE)
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
    Some(sysml_syntax::unquote_string(token.text()))
}

#[cfg(test)]
mod tests {
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
        // the membership it refers through stands first, because the
        // standard takes the first one an expression owns; the text it
        // was written as follows
        assert_eq!(model.kind(model.owned(bound)[0]), ElementKind::Membership);
        let written = model.owned(bound)[1];
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

    /// `then action { ... }` names no action, but it is one: the flow
    /// runs into it and its body is the action's, not the enclosing
    /// statement's. After `do` the same shape means the opposite --
    /// there the body belongs to the statement that performs it.
    #[test]
    fn a_nameless_action_a_flow_runs_into_is_an_action_of_its_own() {
        let owner_of = |model: &Model, roots: &[ElementId], name: &str| {
            let elem = model
                .ids()
                .find(|&id| model.name(id) == Some(name))
                .unwrap_or_else(|| panic!("`{name}` is declared: {roots:?}"));
            model
                .ids()
                .find(|&id| model.owned(id).contains(&elem))
                .expect("something owns it")
        };

        let (model, roots) = build_model(&sysml_syntax::parse(
            "attribute def E;\n\
             action def A {\n\
             \tloop {\n\
             \t\taccept e : E;\n\
             \t\tthen action {\n\t\t\taction g;\n\t\t}\n\
             \t}\n}\n",
        ));
        let holder = owner_of(&model, &roots, "g");
        assert_eq!(model.kind(holder), ElementKind::ActionUsage);
        assert_eq!(model.name(holder), None);

        let (model, roots) = build_model(&sysml_syntax::parse(
            "attribute def E;\n\
             action def A {\n\
             \taccept e : E do action {\n\t\taction g;\n\t}\n}\n",
        ));
        let holder = owner_of(&model, &roots, "g");
        assert_eq!(model.kind(holder), ElementKind::AcceptActionUsage);
    }

    /// One statement, two elements: `then merge m;` writes the node the
    /// rest of the flow names *and* the succession into it, and the
    /// abstract syntax has both. Built as the node alone, the model says
    /// the flow arrives there from nowhere.
    #[test]
    fn a_statement_that_declares_a_step_still_writes_the_succession_into_it() {
        for (source, declared) in [
            (
                "action def A {\n\taction x;\n\tthen merge m;\n}\n",
                ElementKind::MergeNode,
            ),
            (
                "attribute def S;\naction def A {\n\taction x;\n\tthen accept s : S;\n}\n",
                ElementKind::AcceptActionUsage,
            ),
        ] {
            let (model, roots) = build_model(&sysml_syntax::parse(source));
            let action = roots
                .iter()
                .copied()
                .find(|&root| model.name(root) == Some("A"))
                .expect("`A` is declared");
            let inside: Vec<ElementKind> = model
                .owned(action)
                .iter()
                .map(|&it| model.kind(it))
                .collect();
            let at = inside
                .iter()
                .position(|&kind| kind == declared)
                .unwrap_or_else(|| panic!("{declared:?} is built: {inside:?}"));
            // ahead of the declaration, where what it continues from is
            // the step written above
            assert_eq!(inside[at - 1], ElementKind::SuccessionAsUsage, "{inside:?}");
        }

        // but a statement that is only the succession stays the one
        // element it has always been
        let (model, roots) = build_model(&sysml_syntax::parse(
            "action def A {\n\taction x;\n\tthen y;\n\taction y;\n}\n",
        ));
        let action = roots
            .iter()
            .copied()
            .find(|&root| model.name(root) == Some("A"))
            .expect("`A` is declared");
        let inside: Vec<ElementKind> = model
            .owned(action)
            .iter()
            .map(|&it| model.kind(it))
            .collect();
        assert_eq!(
            inside
                .iter()
                .filter(|&&kind| kind == ElementKind::SuccessionAsUsage)
                .count(),
            1,
            "{inside:?}"
        );
    }

    /// `then send new S() via p;` declares the action as much as `then
    /// merge continue;` declares the node. Read as the succession its
    /// leading keyword would otherwise make, the action the source wrote
    /// `port def P` defines two things. The standard has a
    /// `PortDefinition` own exactly one `ConjugatedPortDefinition`,
    /// named after it and related to it by a `PortConjugation` -- and
    /// the notation writes `port p : ~P` to type a port by that one,
    /// so it has to be an element of its own with that name.
    #[test]
    fn a_port_definition_defines_its_conjugate_as_well() {
        let (model, roots) = build_model(&sysml_syntax::parse("part def V {\n\tport def P;\n}\n"));
        let port = model.owned(roots[0])[0];
        assert_eq!(model.kind(port), ElementKind::PortDefinition);
        let owned = model.owned(port);
        assert_eq!(owned.len(), 1, "the conjugate and nothing else");
        let conjugate = owned[0];
        assert_eq!(model.kind(conjugate), ElementKind::ConjugatedPortDefinition);
        assert_eq!(model.name(conjugate), Some("~P"));
        // and the conjugation that says what it is the conjugate of
        let conjugation = model.owned(conjugate)[0];
        assert_eq!(model.kind(conjugation), ElementKind::PortConjugation);
        assert_eq!(
            model.get(conjugation, "conjugatedType"),
            Some(&Value::Ref(conjugate))
        );
        assert_eq!(
            model.get(conjugation, "originalType"),
            Some(&Value::Ref(port))
        );
        // the conjugate has no conjugate of its own, which is what
        // `validatePortDefinitionConjugatedPortDefinition` asks
        assert!(model
            .owned(conjugate)
            .iter()
            .all(|&it| model.kind(it) != ElementKind::ConjugatedPortDefinition));
    }

    /// `IfNode = 'if' ExpressionParameterMember
    /// ActionBodyParameterMember ( 'else' ... )?`, and
    /// `ActionBodyParameter : ActionUsage = ... '{' ActionBodyItem* '}'`.
    /// What a structured control node writes in braces is one parameter
    /// handed to it, not members of the node itself:
    /// `validateForLoopActionUsageParameters` counts a loop's sequence
    /// and its body as two, however many statements the body holds.
    ///
    /// A bare `loop` asks nothing and is handed an empty parameter all
    /// the same -- `'loop' EmptyParameterMember`, where `EmptyUsage :
    /// ReferenceUsage = {}`.
    #[test]
    fn a_control_node_is_handed_one_body_however_much_it_holds() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "action def A {\n\
             \tattribute i;\n\
             \tif i > 0 { action b; action c; } else { action d; }\n\
             \tloop { action e; }\n\
             }\n",
        ));
        let shape = |of: ElementId| -> Vec<(ElementKind, usize)> {
            model
                .owned(of)
                .iter()
                .map(|&it| (model.kind(it), model.owned(it).len()))
                .collect()
        };
        let members = model.owned(roots[0]);
        // the two branches are one parameter each, whatever they hold
        let branch = ElementKind::ActionUsage;
        assert_eq!(
            shape(members[1]),
            [(ElementKind::Expression, 1), (branch, 2), (branch, 1),]
        );
        // and a bare loop is handed an empty parameter beside its body
        assert_eq!(
            shape(members[2]),
            [(ElementKind::ReferenceUsage, 0), (branch, 1)]
        );
    }

    /// A loop written after a `then`, and one given a name, are still
    /// the loop they declare.
    ///
    /// `then while c { ... }` writes the flow it continues *and* the
    /// loop; read in the order the keywords appear, the leading `then`
    /// answered first and the loop was in the model nowhere at all,
    /// with the body it was handed. `action aLoop while c { ... }`
    /// introduces a name rather than a plain action, and what the loop
    /// asks begins after the keyword that says which loop it is. A
    /// `for` is the same both ways round, and so is the branch of a
    /// `then if c { ... }`.
    #[test]
    fn a_loop_after_a_then_or_under_a_name_is_still_a_loop() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "action def A {\n\
             \tattribute i;\n\
             \tthen while i > 0 { action d; }\n\
             \tthen action aLoop while i > 0 { action e; }\n\
             \tthen for t in 1..3 { action f; }\n\
             \taction aFor for u in 1..3 { action g; }\n\
             \tthen if i > 0 { action h; }\n\
             }\n",
        ));
        let loops: Vec<(ElementKind, Vec<ElementKind>)> = model
            .owned(roots[0])
            .iter()
            .filter(|&&it| {
                matches!(
                    model.kind(it),
                    ElementKind::WhileLoopActionUsage | ElementKind::ForLoopActionUsage
                )
            })
            .map(|&it| {
                (
                    model.kind(it),
                    model.owned(it).iter().map(|&c| model.kind(c)).collect(),
                )
            })
            .collect();
        // each asks something and is handed a body: a `while` its
        // condition, a `for` the variable it binds and what it runs over
        let while_loop = ElementKind::WhileLoopActionUsage;
        let for_loop = ElementKind::ForLoopActionUsage;
        assert_eq!(
            loops,
            [
                (
                    while_loop,
                    vec![ElementKind::Expression, ElementKind::ActionUsage]
                ),
                (
                    while_loop,
                    vec![ElementKind::Expression, ElementKind::ActionUsage]
                ),
                (
                    for_loop,
                    vec![
                        ElementKind::ReferenceUsage,
                        ElementKind::Expression,
                        ElementKind::ActionUsage
                    ]
                ),
                (
                    for_loop,
                    vec![
                        ElementKind::ReferenceUsage,
                        ElementKind::Expression,
                        ElementKind::ActionUsage
                    ]
                ),
            ]
        );
        // and a branch after a `then` is the branch it declares, asking
        // its condition and handed its body
        assert_eq!(
            model
                .owned(roots[0])
                .iter()
                .filter(|&&it| model.kind(it) == ElementKind::IfActionUsage)
                .map(|&it| model.owned(it).iter().map(|&c| model.kind(c)).collect())
                .collect::<Vec<Vec<ElementKind>>>(),
            [vec![ElementKind::Expression, ElementKind::ActionUsage]]
        );
    }

    /// What a behaviour is handed, which the notation writes nowhere.
    ///
    /// `AcceptNode : AcceptActionUsage = ... 'accept'
    /// PayloadParameterMember ( 'via' NodeParameterMember )?` and
    /// `PayloadParameter : ReferenceUsage`, so what an accept node waits
    /// for is its first parameter and not an action of its own.
    /// `TriggerAction : AcceptActionUsage` gives a transition an accept
    /// action to wait with, and the transition keeps two parameters of
    /// its own: the occurrence it transitions from, and what the trigger
    /// accepted -- which subsets the trigger's payload, and is how `bind
    /// payload = aState.aTransition.apayload;` names it in the library.
    ///
    /// `validateAcceptActionUsageParameters`,
    /// `validateSendActionParameters` and
    /// `validateTransitionUsageParameters` count all of these, and none
    /// of them was there.
    #[test]
    fn a_behaviour_keeps_the_parameters_it_is_handed() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "state def S {\n\
             \tstate off;\n\
             \taccept go via q do send m via p then off;\n\
             }\n",
        ));
        let transition = model
            .owned(roots[0])
            .iter()
            .copied()
            .find(|&it| model.kind(it) == ElementKind::TransitionUsage)
            .expect("the accept before the then is a transition");
        let directed = |of: ElementId| -> Vec<(ElementKind, Option<&str>)> {
            model
                .owned(of)
                .iter()
                .copied()
                .filter(|&it| model.get(it, "direction").is_some())
                .map(|it| (model.kind(it), model.name(it)))
                .collect()
        };
        // the occurrence it transitions from, and what the trigger took
        assert_eq!(
            directed(transition),
            [
                (ElementKind::ReferenceUsage, None),
                (ElementKind::ReferenceUsage, None),
            ]
        );
        let trigger = reference_list(&model, transition, "triggerAction")[0];
        // the payload it waits for, and the port it waits on
        assert_eq!(
            directed(trigger),
            [
                (ElementKind::ReferenceUsage, Some("go")),
                (ElementKind::ReferenceUsage, None),
            ]
        );
        // what the transition accepted subsets what the trigger did
        let accepted = model
            .owned(transition)
            .iter()
            .copied()
            .filter(|&it| model.get(it, "direction").is_some())
            .nth(1)
            .expect("the second parameter");
        let subsets = model.owned(accepted)[0];
        assert_eq!(model.kind(subsets), ElementKind::Subsetting);
        assert_eq!(
            model
                .get(subsets, "subsettedFeature")
                .and_then(Value::as_id)
                .and_then(|it| model.name(it)),
            Some("go")
        );
        // and the effect is a send action, which is handed three
        let effect = reference_list(&model, transition, "effectAction")[0];
        assert_eq!(directed(effect).len(), 3);
    }

    /// `then event x;` declares the occurrence the flow runs into.
    ///
    /// A sequence writes `event producer.publish_request[1]; then event
    /// server.publish_request[1];`, and the keyword after the `then` is
    /// as much a declaration as the `send` of a `then send ...`. Read as
    /// the succession alone, the step the source wrote was in the model
    /// nowhere and the succession ran into nothing -- thirty-five of the
    /// corpus related fewer than the two things a concrete connector
    /// relates.
    #[test]
    fn a_then_before_a_declaring_keyword_still_declares() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "part def P {\n\
             \tpart producer;\n\
             \tflow f {\n\
             \t\tevent producer.a[1];\n\
             \t\tthen event producer.b[1];\n\
             \t}\n\
             }\n",
        ));
        let flow = model.owned(roots[0])[1];
        assert_eq!(
            model
                .owned(flow)
                .iter()
                .map(|&it| model.kind(it))
                .collect::<Vec<_>>(),
            [
                ElementKind::EventOccurrenceUsage,
                // the succession the `then` writes stands ahead of what
                // it declares, where the step above it is
                ElementKind::SuccessionAsUsage,
                ElementKind::EventOccurrenceUsage,
            ]
        );
    }

    /// A guard before a `then`, and a lone `else`, are transitions.
    ///
    /// `if x > 1 then A2;` after a decision node is
    /// `GuardedTargetSuccession : TransitionUsage`, and the `else A3;`
    /// under it is `DefaultTargetSuccession : TransitionUsage`. Read as
    /// the `if` node of a structured body, the first declared an action
    /// with a branch it never had; the second declared nothing at all,
    /// and the branch the source wrote was in the model nowhere. What
    /// tells them from a structured node is the `then` -- `if c { ... }
    /// else { ... }` writes its branches in braces and no `then`.
    #[test]
    fn a_guard_before_a_then_and_a_lone_else_are_transitions() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "action def A {\n\
             \tattribute x;\n\
             \tdecide;\n\
             \tif x > 1 then A2;\n\
             \telse A3;\n\
             \tif x > 1 { action A4; }\n\
             \taction A2; action A3;\n\
             }\n",
        ));
        let kinds: Vec<ElementKind> = model
            .owned(roots[0])
            .iter()
            .map(|&it| model.kind(it))
            .collect();
        assert_eq!(
            kinds,
            [
                ElementKind::AttributeUsage,
                ElementKind::DecisionNode,
                ElementKind::TransitionUsage,
                ElementKind::TransitionUsage,
                ElementKind::IfActionUsage,
                ElementKind::ActionUsage,
                ElementKind::ActionUsage,
            ]
        );
    }

    /// A loop names the body it repeats, and the `until` after it is
    /// its own.
    ///
    /// `loop action charging { ... } until c;` is one node:
    /// `ActionBodyParameter : ActionUsage = ( 'action' UsageDeclaration?
    /// )? '{' ActionBodyItem* '}'` gives the body a name, which
    /// `charging.monitor` reads through, and `WhileLoopNode = ... ( 'until'
    /// ExpressionParameterMember ';' )?` closes it. Read as it was
    /// written -- a loop, an action beside it and a second loop -- the
    /// node asked for the text of its own body and the flow repeated
    /// twice over.
    #[test]
    fn a_loop_names_the_body_it_repeats_and_the_until_after_it_is_its_own() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "action def A {\n\
             \tattribute c;\n\
             \tloop action charging { action m; } until c >= 100;\n\
             \tthen action aLoop while c > 0 { action d; } until c;\n\
             }\n",
        ));
        let asked = |of: ElementId| -> Option<String> {
            let condition = model
                .owned(of)
                .iter()
                .find(|&&it| model.kind(it) == ElementKind::Expression)?;
            let written = model.owned(*condition)[0];
            model
                .get(written, "body")
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let loops: Vec<ElementId> = model
            .owned(roots[0])
            .iter()
            .copied()
            .filter(|&it| model.kind(it) == ElementKind::WhileLoopActionUsage)
            .collect();
        assert_eq!(loops.len(), 2, "one loop each, not one per statement");

        // `loop` asks nothing and is handed an empty parameter; the
        // body it repeats keeps the name the source gave it; and what
        // the `until` asks is the loop's third
        assert_eq!(
            model
                .owned(loops[0])
                .iter()
                .map(|&it| (model.kind(it), model.name(it)))
                .collect::<Vec<_>>(),
            [
                (ElementKind::ReferenceUsage, None),
                (ElementKind::ActionUsage, Some("charging")),
                (ElementKind::Expression, None),
            ]
        );
        assert_eq!(asked(loops[0]).as_deref(), Some("c >= 100"));

        // and a loop written under a name asks what follows the keyword
        // that says which loop it is, not the name before it
        assert_eq!(asked(loops[1]).as_deref(), Some("c > 0"));
        assert_eq!(
            model
                .owned(loops[1])
                .iter()
                .map(|&it| model.kind(it))
                .collect::<Vec<_>>(),
            [
                ElementKind::Expression,
                ElementKind::ActionUsage,
                ElementKind::Expression,
            ]
        );
    }

    /// The two notations default composition the other way about.
    /// KerML writes `composite feature ...` where it means one and the
    /// metamodel declares `isComposite = false` for everything else;
    /// SysML's `part wheel;` is what its owner is made of unless it
    /// says `ref`. Reading the SysML default into KerML made every
    /// feature of the standard library composite, including
    /// `Base::Anything::self`.
    #[test]
    fn kerml_composes_only_where_it_says_so_and_sysml_unless_it_says_otherwise() {
        let kerml = sysml_syntax::parse_dialect(
            "class V {\n\
             \tcomposite tanks : Tank;\n\
             \tfeature plain : Tank;\n\
             }\n\
             class Tank;\n",
            sysml_syntax::Dialect::KerML,
        );
        let (model, _) = build_model(&kerml);
        let composed = |name: &str| {
            let id = model
                .ids()
                .find(|&id| model.name(id) == Some(name))
                .unwrap_or_else(|| panic!("`{name}` is declared"));
            model.get(id, "isComposite").cloned()
        };
        assert_eq!(composed("tanks"), Some(Value::Bool(true)));
        assert_eq!(composed("plain"), Some(Value::Bool(false)));

        let (model, _) = build_model(&sysml_syntax::parse(
            "part def V {\n\tpart wheel;\n\tref part borrowed;\n}\n",
        ));
        let composed = |name: &str| {
            let id = model
                .ids()
                .find(|&id| model.name(id) == Some(name))
                .unwrap_or_else(|| panic!("`{name}` is declared"));
            model.get(id, "isComposite").cloned()
        };
        assert_eq!(composed("wheel"), Some(Value::Bool(true)));
        assert_eq!(composed("borrowed"), Some(Value::Bool(false)));

        // `validateAttributeDefinitionFeatures` -- "the features of an
        // attribute are all referential". A data value has no parts, so
        // nothing declared inside one is something it is made of,
        // whatever kind that thing is.
        let (model, _) = build_model(&sysml_syntax::parse(
            "attribute def A {\n\tpart inside;\n}\npart def P {\n\tpart inside;\n}\n",
        ));
        let composed: Vec<Option<Value>> = model
            .ids()
            .filter(|&id| model.name(id) == Some("inside"))
            .map(|id| model.get(id, "isComposite").cloned())
            .collect();
        assert_eq!(
            composed,
            [Some(Value::Bool(false)), Some(Value::Bool(true))]
        );
    }

    /// is in the model nowhere at all.
    #[test]
    fn an_action_written_after_then_is_the_action_it_declares() {
        for (source, expected) in [
            (
                "attribute def S;\npart def P { port p; }\n\
                 action def A {\n\taction x;\n\tthen send new S() via P::p;\n}\n",
                ElementKind::SendActionUsage,
            ),
            (
                "attribute def S;\naction def A {\n\taction x;\n\tthen accept s : S;\n}\n",
                ElementKind::AcceptActionUsage,
            ),
            (
                "action def A {\n\tattribute v;\n\taction x;\n\tthen assign v := 1;\n}\n",
                ElementKind::AssignmentActionUsage,
            ),
            (
                "action def A {\n\taction x;\n\tthen terminate y;\n}\n",
                ElementKind::TerminateActionUsage,
            ),
        ] {
            let (model, roots) = build_model(&sysml_syntax::parse(source));
            let action = roots
                .iter()
                .flat_map(|&root| model.owned(root).to_vec())
                .find(|&member| model.name(member) == Some("A"))
                .or_else(|| roots.iter().copied().find(|&r| model.name(r) == Some("A")))
                .expect("`A` is declared");
            let inside: Vec<ElementKind> = model
                .owned(action)
                .iter()
                .map(|&it| model.kind(it))
                .collect();
            assert!(inside.contains(&expected), "{inside:?} for {source:?}");
        }

        // but a transition writes one as the effect it carries across,
        // and the transition is what that statement declares
        let (model, _) = build_model(&sysml_syntax::parse(
            "part def B { port p; }\nstate def S {\n\tstate a;\n\tstate b;\n\
             \tpart sink : B;\n\ttransition t1 first a do send 1 to sink.p then b;\n}\n",
        ));
        let state = model
            .ids()
            .find(|&id| model.name(id) == Some("S"))
            .expect("`S` is declared");
        let inside: Vec<ElementKind> = model
            .owned(state)
            .iter()
            .map(|&it| model.kind(it))
            .collect();
        assert!(inside.contains(&ElementKind::TransitionUsage), "{inside:?}");
        assert!(
            !inside.contains(&ElementKind::SendActionUsage),
            "the effect is the transition's, not a member beside it: {inside:?}"
        );
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
        // `validateAttributeUsageIsReference: isReference` -- "An
        // AttributeUsage is always referential" -- and `isReference =
        // not isComposite`, so an attribute is not something its owner
        // is made of however it is written
        assert_eq!(flag("d", "isComposite"), Some(Value::Bool(false)));
        assert_eq!(flag("k", "isComposite"), Some(Value::Bool(false)));
        // Every flag the builder reads off the source says what the
        // specification declares it to be where the source is silent,
        // and every one of them carries such a default: that is what
        // lets a reader tell a model that said nothing from a model
        // this does not build.
        for flag in BUILT_FLAGS {
            let carried = ElementKind::PartUsage
                .feature(flag)
                .or_else(|| ElementKind::PartDefinition.feature(flag))
                .or_else(|| ElementKind::Feature.feature(flag));
            assert!(
                carried.is_none_or(|meta| meta.default.is_some()),
                "`{flag}` has no default for the model to fall back on"
            );
        }
        // and the flags the specification states outright rather than
        // leaving to a keyword: a variation is abstract
        // (`validateDefinitionVariationIsAbstract`) and a constant
        // feature is a variable one (`validateFeatureConstantIsVariable`)
        assert_eq!(flag("Choice", "isAbstract"), Some(Value::Bool(true)));
        assert_eq!(flag("k", "isVariable"), Some(Value::Bool(true)));
        // `validateEnumerationDefinitionIsVariation` -- an enumeration
        // definition is a variation, written `enum def` and no more
        let (enums, _) = build_model(&sysml_syntax::parse("enum def Colour { red; }\n"));
        let colour = enums
            .ids()
            .find(|&id| enums.name(id) == Some("Colour"))
            .expect("`Colour` is declared");
        assert_eq!(
            enums.get(colour, "isVariation"),
            Some(&Value::Bool(true)),
            "an enumeration is a variation"
        );
        assert_eq!(enums.get(colour, "isAbstract"), Some(&Value::Bool(true)));

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

    /// `ref b;`, `subject s;` and a bare `c;` all name no kind of their
    /// own; `ref part a;` does, so it keeps it.
    #[test]
    fn a_usage_without_a_kind_keyword_is_a_reference() {
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

    /// The guard is an expression tree, which is not made of elements, so
    /// its source text is kept as a textual representation instead.
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
        let guard = model
            .owned(transition)
            .iter()
            .copied()
            .find(|&id| model.kind(id) == ElementKind::Expression)
            .expect("the guard is kept");

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

    /// `in event occurrence x;` parses as an anonymous wrapper around the
    /// usage carrying the name, unlike `event occurrence x;`. The name has
    /// to end up where it was written either way, and the wrapper must
    /// leave nothing of its own beside it.
    #[test]
    fn a_direction_wrapper_does_not_swallow_the_name_it_wraps() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "part def M {\n\tevent occurrence eo;\n\tin event occurrence ieo;\n}\n",
        ));
        let members: Vec<(Option<&str>, ElementKind)> = model
            .owned(roots[0])
            .iter()
            .map(|&id| (model.name(id), model.kind(id)))
            .collect();
        // the wrapper's `event` says what `ieo` is, and its `in` which way
        // the occurrence is passed -- neither leaves an element behind
        assert_eq!(
            members,
            [
                (Some("eo"), ElementKind::EventOccurrenceUsage),
                (Some("ieo"), ElementKind::EventOccurrenceUsage),
            ]
        );
        let ieo = model.owned(roots[0])[1];
        assert_eq!(
            model.get(ieo, "direction"),
            Some(&Value::EnumLit("in")),
            "the wrapper's direction belongs to what it wraps"
        );
    }

    /// `variant part optA;` wraps the declaration the same way, and a
    /// phantom beside it would double every variant a variation offers.
    #[test]
    fn a_variant_wrapper_leaves_one_element_per_variant() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "variation part def Choice {\n\tvariant part optA;\n\tvariant part optB;\n}\n",
        ));
        let members: Vec<(Option<&str>, ElementKind, Option<Role>)> = model
            .owned(roots[0])
            .iter()
            .map(|&id| (model.name(id), model.kind(id), model.member_role(id)))
            .collect();
        assert_eq!(
            members,
            [
                (Some("optA"), ElementKind::PartUsage, Some(Role::Variant)),
                (Some("optB"), ElementKind::PartUsage, Some(Role::Variant)),
            ]
        );
    }

    /// `EnumeratedValue : EnumerationUsage = 'enum'? Usage` -- a literal
    /// that leaves the keyword out is still one of the enumeration's
    /// values, and a reference usage is not one.
    #[test]
    fn an_enumeration_literal_is_a_variant_with_or_without_the_keyword() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "enum def Color {\n\tred;\n\tenum green;\n}\n",
        ));
        let values: Vec<(Option<&str>, ElementKind, Option<Role>)> = model
            .owned(roots[0])
            .iter()
            .map(|&id| (model.name(id), model.kind(id), model.member_role(id)))
            .collect();
        assert_eq!(
            values,
            [
                (
                    Some("red"),
                    ElementKind::EnumerationUsage,
                    Some(Role::Variant)
                ),
                (
                    Some("green"),
                    ElementKind::EnumerationUsage,
                    Some(Role::Variant)
                ),
            ]
        );
        // outside an enumeration body a bare name still names no kind
        let (model, roots) = build_model(&sysml_syntax::parse("part def P {\n\tred;\n}\n"));
        assert_eq!(
            model.kind(model.owned(roots[0])[0]),
            ElementKind::ReferenceUsage
        );
    }

    /// `individual part x : X;` is a part that is one individual, and
    /// `assert not satisfy R by p;` a satisfaction that is asserted: in
    /// both the first keyword written says how, not what.
    #[test]
    fn a_prefix_keyword_does_not_outrank_the_kind_that_follows_it() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "part def X;\n\
             requirement def R;\n\
             part def Rig {\n\
             \tindividual part x : X;\n\
             \tsnapshot s;\n\
             \tpart p : X;\n\
             \tassert not satisfy R by p;\n\
             }\n",
        ));
        let kinds: Vec<ElementKind> = model
            .owned(roots[2])
            .iter()
            .map(|&id| model.kind(id))
            .collect();
        assert_eq!(
            kinds,
            [
                ElementKind::PartUsage,
                ElementKind::OccurrenceUsage,
                ElementKind::PartUsage,
                ElementKind::SatisfyRequirementUsage,
            ]
        );
        let individual = model.owned(roots[2])[0];
        assert_eq!(
            model.get(individual, "isIndividual"),
            Some(&Value::Bool(true))
        );
        let snapshot = model.owned(roots[2])[1];
        assert_eq!(
            model.get(snapshot, "portionKind"),
            Some(&Value::EnumLit("snapshot"))
        );
        let asserted = model.owned(roots[2])[3];
        assert_eq!(model.get(asserted, "isNegated"), Some(&Value::Bool(true)));
    }

    /// `TargetTransitionUsage` puts the trigger before the `then`, so a
    /// statement read as the succession its `then` spells leaves the
    /// trigger with nothing standing for it.
    #[test]
    fn an_accept_before_then_is_a_transition_and_keeps_its_trigger() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "state def S {\n\
             \tstate off;\n\
             \tstate on;\n\
             \taccept go if ready do send m via p then on;\n\
             }\n",
        ));
        let transition = model.owned(roots[0])[2];
        assert_eq!(model.kind(transition), ElementKind::TransitionUsage);
        // `TriggerAction : AcceptActionUsage = AcceptParameterPart` --
        // the trigger is an accept action written with no name, and what
        // it waits for is its first parameter
        let trigger = reference_list(&model, transition, "triggerAction")[0];
        assert_eq!(model.kind(trigger), ElementKind::AcceptActionUsage);
        assert_eq!(model.name(trigger), None);
        assert_eq!(
            crate::payload_parameter(&model, trigger).and_then(|it| model.name(it)),
            Some("go")
        );
        let guard = reference_list(&model, transition, "guardExpression")[0];
        let written = model.owned(guard)[0];
        assert_eq!(
            model.get(written, "body").and_then(Value::as_str),
            Some("ready")
        );
        let effect = reference_list(&model, transition, "effectAction")[0];
        assert_eq!(model.kind(effect), ElementKind::SendActionUsage);
    }

    /// `StateActionUsage` is the action itself, not a wrapper around a
    /// declaration: `do providePower;` performs what it names, and the
    /// keyword before it says in which of a state's three roles.
    #[test]
    fn a_state_subaction_written_as_a_reference_still_becomes_one() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "state def S {\n\
             \tentry performSelfTest { in vehicle = 1; }\n\
             \tdo providePower;\n\
             \texit shutDown;\n\
             \texit action tidyUp;\n\
             }\n",
        ));
        let members: Vec<(ElementKind, Option<Role>)> = model
            .owned(roots[0])
            .iter()
            .map(|&id| (model.kind(id), model.member_role(id)))
            .collect();
        assert_eq!(
            members,
            [
                (ElementKind::PerformActionUsage, Some(Role::Entry)),
                (ElementKind::PerformActionUsage, Some(Role::Do)),
                (ElementKind::PerformActionUsage, Some(Role::Exit)),
                // an `exit` that declares an action rather than naming
                // one keeps the declaration, and the role with it
                (ElementKind::ActionUsage, Some(Role::Exit)),
            ]
        );
        // what a performed action's body declares is its own member, and
        // the name it performs by is a reference rather than a name of
        // its own
        let entry = model.owned(roots[0])[0];
        assert_eq!(model.name(entry), None);
        assert_eq!(
            model
                .owned(entry)
                .iter()
                .filter_map(|&id| model.name(id))
                .collect::<Vec<_>>(),
            ["vehicle"]
        );
    }

    /// `SendNode`, `AcceptNode` and `AssignmentNode` are metaclasses of
    /// their own; written on their own line they were building nothing at
    /// all, and an assignment written after an action name was arriving
    /// as if the action had a value.
    #[test]
    fn a_bare_action_node_becomes_the_node_it_is() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "action def A {\n\
             \tsend x via p;\n\
             \taccept sig : Sig;\n\
             \tassign x := 1;\n\
             \taction a1 assign y := 2;\n\
             }\n",
        ));
        let members: Vec<ElementKind> = model
            .owned(roots[0])
            .iter()
            .map(|&id| model.kind(id))
            .collect();
        assert_eq!(
            members,
            [
                ElementKind::SendActionUsage,
                ElementKind::AcceptActionUsage,
                ElementKind::AssignmentActionUsage,
                ElementKind::AssignmentActionUsage,
            ]
        );
        // `assign y := 2` gives the value to `y`; the assignment is not a
        // feature that has one
        let assignment = model.owned(roots[0])[3];
        assert!(model
            .owned(assignment)
            .iter()
            .all(|&id| model.kind(id) != ElementKind::FeatureValue));
    }

    /// `specialization s subtype A :> B;` writes as a statement of its own
    /// what a declaration writes as a clause, and the model had nothing
    /// standing for any of them.
    #[test]
    fn a_relationship_written_as_a_statement_is_an_element() {
        let text = "package K {\n\
                    \tclassifier A;\n\
                    \tclassifier B;\n\
                    \tspecialization s subtype A :> B;\n\
                    \tdisjoining d disjoint A from B;\n\
                    \tsubtype A :> B;\n\
                    }\n";
        let parse = sysml_syntax::parse_dialect(text, sysml_syntax::Dialect::KerML);
        assert!(parse.ok());
        let (model, roots) = build_model(&parse);
        let members: Vec<(Option<&str>, ElementKind)> = model
            .owned(roots[0])
            .iter()
            .map(|&id| (model.name(id), model.kind(id)))
            .collect();
        assert_eq!(
            members,
            [
                (Some("A"), ElementKind::Classifier),
                (Some("B"), ElementKind::Classifier),
                (Some("s"), ElementKind::Specialization),
                (Some("d"), ElementKind::Disjoining),
                // `Specialization = ( 'specialization' Identification )?
                // 'subtype' ...` -- the name is optional
                (None, ElementKind::Specialization),
            ]
        );
    }

    /// `ForLoopNode = 'for' ForVariableDeclarationMember 'in'
    /// ExpressionParameterMember ...` -- what the loop asks for is the
    /// sequence after `in`. Kept whole it arrives as a comparison, and
    /// the `in` was being read as a direction besides.
    #[test]
    fn a_for_loop_asks_for_what_it_iterates_over() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "action def A {\n\tfor t in 1..3 { action each; }\n}\n",
        ));
        let loop_node = model.owned(roots[0])[0];
        assert_eq!(model.kind(loop_node), ElementKind::ForLoopActionUsage);
        assert_eq!(model.get(loop_node, "direction"), None);
        let variable = model
            .owned(loop_node)
            .iter()
            .find_map(|&id| (model.name(id) == Some("t")).then_some(id))
            .expect("the loop declares what it binds");
        assert_eq!(model.kind(variable), ElementKind::ReferenceUsage);
        let asked = model
            .owned(loop_node)
            .iter()
            .find(|&&id| model.kind(id) == ElementKind::Expression)
            .copied()
            .expect("the loop says what it iterates over");
        let written = model.owned(asked)[0];
        assert_eq!(
            model.get(written, "body").and_then(Value::as_str),
            Some("1..3")
        );
    }

    #[test]
    fn a_condition_the_parser_pieced_together_is_kept_whole() {
        // `if x , y { ... }` is not a condition anybody meant, but the
        // parser recovers it as several pieces rather than one
        // expression, and what the model keeps has to be all of them --
        // reading only the first would say the branch asks about `x`
        let (model, roots) = build_model(&sysml_syntax::parse(
            "action def A {\n\tif x , y { action b; }\n}\n",
        ));
        let branch = model.owned(roots[0])[0];
        assert_eq!(model.kind(branch), ElementKind::IfActionUsage);
        let asked = model
            .owned(branch)
            .iter()
            .find(|&&id| model.kind(id) == ElementKind::Expression)
            .copied()
            .expect("the branch says what it asks");
        let written = model.owned(asked)[0];
        assert_eq!(
            model.get(written, "body").and_then(Value::as_str),
            Some("x , y")
        );
    }

    /// `WhileLoopNode : WhileLoopActionUsage = ... ( 'until'
    /// ExpressionParameterMember ';' )?` -- the parser writes the `until`
    /// as a statement of its own, and a second loop standing for it says
    /// the flow repeats twice over.
    #[test]
    fn an_until_closes_the_loop_written_before_it() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "action def A {\n\tloop { action tick; } until done;\n}\n",
        ));
        let members: Vec<ElementKind> = model
            .owned(roots[0])
            .iter()
            .map(|&id| model.kind(id))
            .collect();
        assert_eq!(members, [ElementKind::WhileLoopActionUsage]);
        let repeats = model.owned(roots[0])[0];
        let asked = model
            .owned(repeats)
            .iter()
            .find(|&&id| model.kind(id) == ElementKind::Expression)
            .copied()
            .expect("the loop says what ends it");
        let written = model.owned(asked)[0];
        assert_eq!(
            model.get(written, "body").and_then(Value::as_str),
            Some("done")
        );
    }

    /// `import all P::*` brings in what is private as well and `import
    /// P::**` everything nested under what it names. Neither was
    /// arriving, so both went out as the plain import they are not.
    #[test]
    fn an_import_keeps_what_it_says_it_brings_in() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "package P {\n\timport all Q::*;\n\timport Q::**;\n}\n",
        ));
        let imports = model.owned(roots[0]).to_vec();
        assert_eq!(
            model.get(imports[0], "isImportAll"),
            Some(&Value::Bool(true))
        );
        assert_eq!(model.get(imports[0], "isRecursive"), None);
        assert_eq!(
            model.get(imports[1], "isRecursive"),
            Some(&Value::Bool(true))
        );
    }

    /// A string literal is what stands between its quotes with the
    /// escapes resolved: trimming the quotes off ate the escaped one at
    /// the end, and the backslash before it stayed.
    #[test]
    fn a_string_is_read_with_its_escapes_resolved() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "package P {\n\
             \tcomment C locale \"en-GB\" /* said */\n\
             \tpart def V { attribute greeting = \"say \\\"hi\\\"\"; }\n\
             }\n",
        ));
        let comment = model.owned(roots[0])[0];
        assert_eq!(
            model.get(comment, "locale").and_then(Value::as_str),
            Some("en-GB")
        );
        let greeting = model.owned(model.owned(roots[0])[1])[0];
        let literal = reference(&model, model.owned(greeting)[0], "value").unwrap();
        assert_eq!(model.kind(literal), ElementKind::LiteralString);
        assert_eq!(
            model.get(literal, "value").and_then(Value::as_str),
            Some("say \"hi\"")
        );
    }

    /// The elements a property points at, which a caller has just asked
    /// the builder to have written.
    fn reference_list<'a>(model: &'a Model, of: ElementId, property: &str) -> &'a [ElementId] {
        model
            .get(of, property)
            .and_then(Value::as_ids)
            .unwrap_or_default()
    }

    /// What stands between an `end` and the declaration after it is the
    /// cross feature, and the declaration is the end.
    ///
    /// `end [1] feature src references source;` -- `EndUsagePrefix :
    /// Usage = isEnd ?= 'end' ( ownedRelationship +=
    /// OwnedCrossFeatureMember )?`, and the standard says where the
    /// cross feature lands: "owned cross features are in the namespace
    /// of the owning association ends, so their names are qualified by
    /// the name of the association ends". Read the other way round, the
    /// association's end is the `[1]` and the end the source declared
    /// is nested inside it.
    #[test]
    fn a_cross_feature_belongs_to_the_end_written_after_it() {
        let (model, roots) = build_model(&sysml_syntax::parse(
            "connection def C {\n\tend [1] feature src references source;\n}\n",
        ));
        let ends: Vec<Option<&str>> = model
            .owned(roots[0])
            .iter()
            .map(|&id| model.name(id))
            .collect();
        assert_eq!(ends, [Some("src")]);
        let end = model.owned(roots[0])[0];
        assert_eq!(model.get(end, "isEnd"), Some(&Value::Bool(true)));
        // the cross feature is nameless here and is not an end itself
        let cross = model
            .owned(end)
            .iter()
            .copied()
            .find(|&it| model.kind(it).is_a(ElementKind::Feature))
            .expect("the `[1]` stands for a cross feature");
        assert_eq!(model.name(cross), None);
        assert_eq!(model.get(cross, "isEnd"), None);
        assert_eq!(
            model
                .owned(cross)
                .iter()
                .map(|&it| model.kind(it))
                .collect::<Vec<_>>(),
            [ElementKind::MultiplicityRange]
        );
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

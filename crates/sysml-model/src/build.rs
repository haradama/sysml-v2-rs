//! Structural conversion from the syntax tree to model elements.
//!
//! Builds the ownership tree with element kinds and declared names for the
//! structural constructs (packages, definitions, usages, imports,
//! annotations). Relationships (typings, specializations, redefinitions) are
//! reified with resolved targets by `sysml-semantics`; expression trees are
//! not represented as elements.

use sysml_syntax::{Parse, SyntaxKind, SyntaxNode};

use crate::{ElementId, ElementKind, Model, Value};

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
        // an alias is a Membership whose memberElement is resolved later
        ALIAS => Some(ElementKind::Membership),
        DOCUMENTATION => Some(ElementKind::Documentation),
        COMMENT_ELEM => Some(ElementKind::Comment),
        REP => Some(ElementKind::TextualRepresentation),
        METADATA_ANNOTATION => Some(ElementKind::MetadataUsage),
        _ => None,
    };

    let Some(kind) = kind else {
        // statements, filters, expressions: no structural element of
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
    if let Some(short) = declared_short_name(node) {
        model.set(id, "declaredShortName", Value::String(short));
    }
    if has_token(node, ABSTRACT_KW) && kind.feature("isAbstract").is_some() {
        model.set(id, "isAbstract", Value::Bool(true));
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

    // recurse into the element's body, parameter list and nested
    // declarations (`end x [1..*] feature y : T;`)
    for child in node.children() {
        match child.kind() {
            BODY | PARAM_LIST => {
                for member in child.children() {
                    build_node(model, &member, Some(id), built);
                }
            }
            // `then action b;` declares b for the enclosing behaviour; the
            // succession only points at it, so it must not own it
            DEFINITION | USAGE => {
                let parent = if node.kind() == CONTROL_STMT {
                    owner
                } else {
                    Some(id)
                };
                build_node(model, &child, parent, built);
            }
            _ => {}
        }
    }
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
            SyntaxKind::FIRST_KW | SyntaxKind::THEN_KW => Some(ElementKind::SuccessionAsUsage),
            _ => None,
        })
    })
}

/// The control node a keyword declares, if it declares one.
fn control_node_kind(token: SyntaxKind) -> Option<ElementKind> {
    match token {
        SyntaxKind::MERGE_KW => Some(ElementKind::MergeNode),
        SyntaxKind::DECIDE_KW => Some(ElementKind::DecisionNode),
        SyntaxKind::FORK_KW => Some(ElementKind::ForkNode),
        SyntaxKind::JOIN_KW => Some(ElementKind::JoinNode),
        _ => None,
    }
}

/// Some statements write their own name as a plain reference rather than
/// the `NAME` node a declaration carries: `merge continue;`, `transition
/// off_to_on first off then on`, `action engineStarted accept engineStart`.
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
        )
    };
    match node.kind() {
        SyntaxKind::CONTROL_STMT => {}
        SyntaxKind::USAGE if tokens(node).any(introduces_operands) => {}
        _ => return None,
    }
    // `then merge continue;` names the node it declares, so the leading
    // `then` does not end the search -- the declaration keyword restarts it
    let declares = tokens(node).any(|t| control_node_kind(t).is_some());
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
        _ => "Classifier",
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
        _ => "Usage",
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
    #[test]
    fn statements_name_themselves_with_a_plain_reference() {
        // `action b accept x;` writes `b` as a reference, not a NAME node,
        // while a bare `fork;` names nothing at all
        let (model, roots) = build_model(&sysml_syntax::parse(
            "action def A {\n\tmerge m;\n\tfork;\n\taction b accept x;\n\tfirst m then b;\n}\n",
        ));
        let names: Vec<&str> = model
            .owned(roots[0])
            .iter()
            .filter_map(|&id| model.name(id))
            .collect();
        assert_eq!(names, ["m", "b"]);
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

        // `#keyword def X` has no kind keyword: falls back to Classifier
        let parse = sysml_syntax::parse("#service def X;");
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

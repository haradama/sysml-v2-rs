//! Turning a resolved [`Model`] into the graph a diagram draws.

use std::collections::HashMap;

use sysml_model::{ElementId, ElementKind, Model, Value};

/// One entry of a box's feature compartment, e.g. `attribute mass : Real`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feature {
    /// SysML keyword of the usage, e.g. `attribute` or `port`.
    pub keyword: String,
    pub name: String,
    /// Declared type, when the model reifies a `FeatureTyping` for it.
    pub ty: Option<String>,
}

impl Feature {
    /// The compartment line as it appears in the drawing.
    pub fn label(&self) -> String {
        match &self.ty {
            Some(ty) => format!("{} {} : {ty}", self.keyword, self.name),
            None => format!("{} {}", self.keyword, self.name),
        }
    }
}

/// How a node is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    /// A labelled box: a definition, part, state or action.
    #[default]
    Box,
    /// The filled circle a state machine or action flow starts from,
    /// carrying no label of its own.
    Initial,
}

/// One box: a named definition and the features it declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub id: ElementId,
    pub name: String,
    /// SysML keyword shown in guillemets, e.g. `part def`.
    pub keyword: String,
    pub features: Vec<Feature>,
    pub shape: Shape,
}

/// What an edge between two boxes means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    /// `from` specializes `to` (`part def Engine :> PowerSource`). Drives
    /// the layering: the supertype is drawn above its subtypes.
    Specialization,
    /// `from` declares a part typed by `to` (`part def Vehicle { part eng
    /// : Engine; }`), so `to` is one of the things `from` is made of.
    Composition,
    /// `from` and `to` are wired together (`connect w.hub to a.mount`).
    /// Undirected: which end is `from` only reflects declaration order.
    Connection,
    /// Control flows from `from` to `to` (`transition first off then on`,
    /// `first a then b`). Directed, unlike a connection.
    Transition,
}

/// A relationship between two boxes. Both index fields index
/// [`Diagram::nodes`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub relation: Relation,
    /// For a connection, the feature each end attaches to (`hub`, `mount`),
    /// which is what tells two connections between the same pair apart.
    pub ends: Option<(String, String)>,
}

/// The definitions to draw and the specializations between them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diagram {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

/// Collect every named definition owned (directly or transitively) by one of
/// `roots`, plus the specializations that run between two collected ones.
///
/// Specializations pointing outside the collected set are dropped rather than
/// drawn as dangling stubs: with the standard library loaded, most of them
/// would leave the diagram anyway.
pub fn definition_diagram(model: &Model, roots: &[ElementId]) -> Diagram {
    let mut nodes: Vec<Node> = Vec::new();
    let mut index: HashMap<ElementId, usize> = HashMap::new();

    for &root in roots {
        for id in model.descendants(root) {
            if !model.kind(id).is_a(ElementKind::Definition) {
                continue;
            }
            let Some(name) = model.name(id) else {
                continue;
            };
            // roots may overlap, and a definition must not be drawn twice
            if index.contains_key(&id) {
                continue;
            }
            index.insert(id, nodes.len());
            nodes.push(Node {
                id,
                name: name.to_string(),
                keyword: keyword(model.kind(id)),
                features: features_of(model, id),
                shape: Shape::Box,
            });
        }
    }

    let mut edges = Vec::new();
    for (from, node) in nodes.iter().enumerate() {
        for &rel in model.owned(node.id) {
            if model.kind(rel) != ElementKind::Subclassification {
                continue;
            }
            let Some(Value::Ref(target)) = model.get(rel, "superclassifier") else {
                continue;
            };
            if let Some(&to) = index.get(target) {
                edges.push(Edge {
                    from,
                    to,
                    relation: Relation::Specialization,
                    ends: None,
                });
            }
        }
        compositions_of(model, node.id, from, &index, &mut edges);
    }

    Diagram { nodes, edges }
}

/// The internal structure of one definition: a box per part, state or action
/// it is composed of, and an edge per connection or transition declared
/// between two of them.
///
/// The same shape serves a `part def` (parts wired by `connect`), a
/// `state def` (states linked by `transition`) and an `action def` (actions
/// sequenced by `first ... then`), because all three are children of the
/// definition related by a two-ended statement.
///
/// Edges whose ends leave the definition, and self-edges between two
/// features of one box, are left undrawn.
pub fn interconnection_diagram(model: &Model, definition: ElementId) -> Diagram {
    let mut nodes: Vec<Node> = Vec::new();
    let mut index: HashMap<ElementId, usize> = HashMap::new();

    for &child in model.owned(definition) {
        if !is_structure_box(model.kind(child)) {
            continue;
        }
        let Some(name) = model.name(child) else {
            continue;
        };
        // a part is read as `role : Type`, unlike a definition's bare name
        let label = match type_of(model, child) {
            Some(ty) => format!("{name} : {ty}"),
            None => name.to_string(),
        };
        index.insert(child, nodes.len());
        nodes.push(Node {
            id: child,
            name: label,
            keyword: keyword(model.kind(child)),
            features: features_of(model, child),
            shape: Shape::Box,
        });
    }

    let mut edges = Vec::new();
    for &child in model.owned(definition) {
        let ends = connector_ends(model, child);
        // `entry; then off;` names only where the flow goes, so it starts
        // from the filled circle every state machine begins at
        if let [only] = &ends[..] {
            if model.kind(child) == ElementKind::SuccessionAsUsage {
                if let Some(&to) = index.get(&only.0) {
                    edges.push(Edge {
                        from: nodes.len(),
                        to,
                        relation: Relation::Transition,
                        ends: None,
                    });
                    nodes.push(Node {
                        id: child,
                        name: String::new(),
                        keyword: String::new(),
                        features: Vec::new(),
                        shape: Shape::Initial,
                    });
                }
            }
            continue;
        }
        let Ok([first, second]) = <[_; 2]>::try_from(ends) else {
            continue;
        };
        let (Some(&from), Some(&to)) = (index.get(&first.0), index.get(&second.0)) else {
            continue;
        };
        if from != to {
            let directed = model.kind(child).is_a(ElementKind::TransitionUsage)
                || model.kind(child) == ElementKind::SuccessionAsUsage;
            edges.push(Edge {
                from,
                to,
                relation: if directed {
                    Relation::Transition
                } else {
                    Relation::Connection
                },
                // a transition names its states twice over; only a
                // connection's port labels add anything
                ends: (!directed).then_some((first.1, second.1)),
            });
        }
    }

    Diagram { nodes, edges }
}

/// Whether a usage is one of the things a definition is composed of, rather
/// than a relationship between two of them.
///
/// The exclusions matter because the metamodel makes every relationship a
/// specialization of what it relates: `ConnectionUsage` is a `PartUsage`,
/// and `TransitionUsage` and the control nodes are all `ActionUsage`. A
/// named `connect c : Conn ...` or `transition t first a then b` must be an
/// edge only, never also a box.
fn is_structure_box(kind: ElementKind) -> bool {
    let composed = kind.is_a(ElementKind::PartUsage)
        || kind.is_a(ElementKind::StateUsage)
        || kind.is_a(ElementKind::ActionUsage);
    let relates =
        kind.is_a(ElementKind::ConnectorAsUsage) || kind.is_a(ElementKind::TransitionUsage);
    composed && !relates
}

/// Each end of a connector as `(part it starts at, feature it attaches to)`,
/// read off the feature chains name resolution reified onto it. Anything
/// that is not a connector simply has no such ends.
///
/// `connect w.hub to a.mount` chains to `[w, hub]`, so the first entry
/// picks the box and the last names the port on it. A bare `connect w to a`
/// chains to `[w]`, where both are the same element.
fn connector_ends(model: &Model, connector: ElementId) -> Vec<(ElementId, String)> {
    model
        .owned(connector)
        .iter()
        .filter_map(|&end| match model.get(end, "chainingFeature") {
            Some(Value::RefList(chain)) => {
                let part = *chain.first()?;
                let feature = model.name(*chain.last()?).unwrap_or_default();
                Some((part, feature.to_string()))
            }
            _ => None,
        })
        .collect()
}

/// One composition edge per distinct part type a definition declares.
///
/// Two parts of the same type would draw the same line twice, so the target
/// is only linked once; a definition holding a part of its own type is left
/// to its compartment line rather than drawn as a loop back onto the box.
fn compositions_of(
    model: &Model,
    definition: ElementId,
    from: usize,
    index: &HashMap<ElementId, usize>,
    edges: &mut Vec<Edge>,
) {
    let mut linked: Vec<usize> = Vec::new();
    for &child in model.owned(definition) {
        // composition is about parts; a state or action a definition owns is
        // its behaviour, not something it is assembled from
        if !model.kind(child).is_a(ElementKind::PartUsage) || !is_structure_box(model.kind(child)) {
            continue;
        }
        let Some(&to) = type_reference(model, child).and_then(|ty| index.get(&ty)) else {
            continue;
        };
        if to == from || linked.contains(&to) {
            continue;
        }
        linked.push(to);
        edges.push(Edge {
            from,
            to,
            relation: Relation::Composition,
            ends: None,
        });
    }
}

/// The element a usage's reified `FeatureTyping` points at.
fn type_reference(model: &Model, usage: ElementId) -> Option<ElementId> {
    model.owned(usage).iter().find_map(|&rel| {
        if model.kind(rel) != ElementKind::FeatureTyping {
            return None;
        }
        match model.get(rel, "type") {
            Some(Value::Ref(target)) => Some(*target),
            _ => None,
        }
    })
}

/// The SysML keyword a metaclass is written with: `PartDefinition` becomes
/// `part def`, `AttributeUsage` becomes `attribute`, and a multi-word
/// metaclass such as `AnalysisCaseDefinition` becomes `analysis case def`.
fn keyword(kind: ElementKind) -> String {
    let name = kind.name();
    let (base, suffix) = match name.strip_suffix("Definition") {
        Some(base) => (base, " def"),
        None => (name.strip_suffix("Usage").unwrap_or(name), ""),
    };
    let mut out = String::new();
    for (i, ch) in base.char_indices() {
        if i > 0 && ch.is_uppercase() {
            out.push(' ');
        }
        out.extend(ch.to_lowercase());
    }
    out.push_str(suffix);
    out
}

/// The named usages a definition declares directly, in source order.
fn features_of(model: &Model, definition: ElementId) -> Vec<Feature> {
    let mut out = Vec::new();
    for &child in model.owned(definition) {
        if !model.kind(child).is_a(ElementKind::Usage) {
            continue;
        }
        let Some(name) = model.name(child) else {
            continue;
        };
        out.push(Feature {
            keyword: keyword(model.kind(child)),
            name: name.to_string(),
            ty: type_of(model, child),
        });
    }
    out
}

/// The type name of a usage, read off the `FeatureTyping` that name
/// resolution reified for its `:` clause.
fn type_of(model: &Model, usage: ElementId) -> Option<String> {
    let target = type_reference(model, usage)?;
    model.name(target).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::resolved;

    #[test]
    fn collects_definitions_and_their_specializations() {
        let ws = resolved(
            "package P {\n\
             	abstract part def PowerSource;\n\
             	part def Engine :> PowerSource;\n\
             	part def Turbine :> PowerSource;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);

        let names: Vec<&str> = diagram.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["PowerSource", "Engine", "Turbine"]);
        // the package itself is not a Definition, so it is not a box
        assert!(diagram.nodes.iter().all(|n| n.keyword == "part def"));

        let edges: Vec<(&str, &str)> = diagram
            .edges
            .iter()
            .map(|e| {
                (
                    diagram.nodes[e.from].name.as_str(),
                    diagram.nodes[e.to].name.as_str(),
                )
            })
            .collect();
        assert_eq!(
            edges,
            [("Engine", "PowerSource"), ("Turbine", "PowerSource")]
        );
    }

    #[test]
    fn features_carry_their_keyword_and_resolved_type() {
        let ws = resolved(
            "part def FuelPort;\n\
             part def Engine {\n\
             	attribute power;\n\
             	port fuelIn : FuelPort;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let engine = diagram.nodes.iter().find(|n| n.name == "Engine").unwrap();

        assert_eq!(
            engine.features,
            [
                Feature {
                    keyword: "attribute".to_string(),
                    name: "power".to_string(),
                    ty: None,
                },
                Feature {
                    keyword: "port".to_string(),
                    name: "fuelIn".to_string(),
                    ty: Some("FuelPort".to_string()),
                },
            ]
        );
        assert_eq!(engine.features[0].label(), "attribute power");
        assert_eq!(engine.features[1].label(), "port fuelIn : FuelPort");
    }

    /// `(from, to, relation)` for every edge, by name.
    fn edges_of(diagram: &Diagram) -> Vec<(&str, &str, Relation)> {
        diagram
            .edges
            .iter()
            .map(|e| {
                (
                    diagram.nodes[e.from].name.as_str(),
                    diagram.nodes[e.to].name.as_str(),
                    e.relation,
                )
            })
            .collect()
    }

    #[test]
    fn parts_become_composition_edges() {
        let ws = resolved(
            "part def Engine;\n\
             part def Wheel;\n\
             part def Vehicle {\n\
             	part eng : Engine;\n\
             	part front : Wheel;\n\
             	part rear : Wheel;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);

        // two wheels, but the pair of boxes is only linked once
        assert_eq!(
            edges_of(&diagram),
            [
                ("Vehicle", "Engine", Relation::Composition),
                ("Vehicle", "Wheel", Relation::Composition),
            ]
        );
    }

    #[test]
    fn only_parts_compose_and_only_resolved_ones() {
        let ws = resolved(
            "part def Engine;\n\
             part def Vehicle {\n\
             	attribute mass;\n\
             	part missing : NoSuchDefinition;\n\
             	part eng : Engine;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        assert_eq!(
            edges_of(&diagram),
            [("Vehicle", "Engine", Relation::Composition)]
        );
    }

    #[test]
    fn a_part_of_the_definitions_own_type_is_left_to_its_compartment() {
        let ws = resolved("part def Node {\n\tpart children : Node;\n}\n");
        let diagram = definition_diagram(ws.model(), &[ws.root()]);

        assert!(diagram.edges.is_empty(), "{:?}", diagram.edges);
        assert_eq!(diagram.nodes[0].features[0].label(), "part children : Node");
    }

    #[test]
    fn specializations_leaving_the_diagram_are_dropped() {
        // `Base::Anything` is not loaded here, so nothing resolves to a box
        let ws = resolved("part def A;\npart def B :> A;\n");
        let all = definition_diagram(ws.model(), &[ws.root()]);
        assert_eq!(all.edges.len(), 1);

        // scope the diagram at B: A is outside it, so the edge has no target
        let b = all.nodes.iter().find(|n| n.name == "B").unwrap().id;
        let scoped = definition_diagram(ws.model(), &[b]);
        assert_eq!(scoped.nodes.len(), 1);
        assert!(scoped.edges.is_empty());
    }

    #[test]
    fn overlapping_roots_do_not_duplicate_boxes() {
        let ws = resolved("package P {\n\tpart def A;\n}\n");
        let root = ws.root();
        let once = definition_diagram(ws.model(), &[root]);
        let twice = definition_diagram(ws.model(), &[root, root]);
        assert_eq!(once, twice);
        assert_eq!(twice.nodes.len(), 1);
    }

    #[test]
    fn keywords_come_from_the_metaclass_name() {
        assert_eq!(keyword(ElementKind::PartDefinition), "part def");
        assert_eq!(keyword(ElementKind::AttributeUsage), "attribute");
        assert_eq!(
            keyword(ElementKind::AnalysisCaseDefinition),
            "analysis case def"
        );
        // neither suffix: the metaclass name itself, split into words
        assert_eq!(keyword(ElementKind::Subclassification), "subclassification");
    }

    /// Hand-built models reach the defensive paths that parsing cannot: an
    /// unnamed definition, a relationship with no target, and a typing whose
    /// target has no name.
    #[test]
    fn incomplete_elements_are_skipped() {
        let mut model = Model::new();
        let root = model.create(ElementKind::Package);

        let anonymous = model.create(ElementKind::PartDefinition);
        model.add_owned(root, anonymous);

        let named = model.create(ElementKind::PartDefinition);
        model.set(named, "declaredName", Value::String("A".to_string()));
        model.add_owned(root, named);

        // a Subclassification that never got its `superclassifier` set
        let dangling = model.create(ElementKind::Subclassification);
        model.add_owned(named, dangling);

        // a usage typed by an element that has no declaredName, behind a
        // relationship that is not the typing being looked for
        let usage = model.create(ElementKind::AttributeUsage);
        model.set(usage, "declaredName", Value::String("x".to_string()));
        model.add_owned(named, usage);
        let redefinition = model.create(ElementKind::Redefinition);
        model.add_owned(usage, redefinition);
        let unnamed_type = model.create(ElementKind::AttributeDefinition);
        model.add_owned(root, unnamed_type);
        let typing = model.create(ElementKind::FeatureTyping);
        model.set(typing, "type", Value::Ref(unnamed_type));
        model.add_owned(usage, typing);

        // an unnamed usage, which never becomes a compartment line
        let anonymous_usage = model.create(ElementKind::PartUsage);
        model.add_owned(named, anonymous_usage);

        let diagram = definition_diagram(&model, &[root]);
        assert_eq!(diagram.nodes.len(), 1);
        assert!(diagram.edges.is_empty());
        assert_eq!(
            diagram.nodes[0].features,
            [Feature {
                keyword: "attribute".to_string(),
                name: "x".to_string(),
                ty: None,
            }]
        );
    }

    #[test]
    fn a_typing_without_a_reference_yields_no_type() {
        let mut model = Model::new();
        let definition = model.create(ElementKind::PartDefinition);
        model.set(definition, "declaredName", Value::String("A".to_string()));
        let usage = model.create(ElementKind::AttributeUsage);
        model.set(usage, "declaredName", Value::String("x".to_string()));
        model.add_owned(definition, usage);
        let typing = model.create(ElementKind::FeatureTyping);
        // `isImplied` is set, `type` is not
        model.set(typing, "isImplied", Value::Bool(true));
        model.add_owned(usage, typing);

        let diagram = definition_diagram(&model, &[definition]);
        assert_eq!(diagram.nodes[0].features[0].ty, None);
    }

    #[test]
    fn an_empty_model_yields_an_empty_diagram() {
        let ws = resolved("");
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        assert_eq!(diagram, Diagram::default());
    }
}

#[cfg(test)]
mod interconnection_tests {
    use super::*;
    use crate::tests::resolved;

    const CAR: &str = "part def Wheel { port hub; }\n\
                       part def Axle { port mount; }\n\
                       part def Car {\n\
                       \tpart w : Wheel;\n\
                       \tpart a : Axle;\n\
                       \tconnect w.hub to a.mount;\n\
                       }\n";

    fn definition(ws: &sysml_semantics::Workspace, name: &str) -> ElementId {
        ws.named_elements()
            .find(|(_, declared)| *declared == name)
            .map(|(id, _)| id)
            .unwrap()
    }

    #[test]
    fn draws_parts_and_the_connections_between_them() {
        let ws = resolved(CAR);
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "Car"));

        let names: Vec<&str> = diagram.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["w : Wheel", "a : Axle"]);
        assert!(diagram.nodes.iter().all(|n| n.keyword == "part"));
        assert_eq!(
            diagram.edges,
            [Edge {
                from: 0,
                to: 1,
                relation: Relation::Connection,
                ends: Some(("hub".to_string(), "mount".to_string())),
            }]
        );
    }

    #[test]
    fn an_end_outside_the_definition_is_not_drawn() {
        let ws = resolved(
            "part def Wheel { port hub; }\n\
             part def Car {\n\
             \tpart w : Wheel;\n\
             \tconnect w.hub to Wheel;\n\
             }\n",
        );
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "Car"));
        assert_eq!(diagram.nodes.len(), 1);
        assert!(diagram.edges.is_empty());
    }

    #[test]
    fn a_connection_between_two_features_of_one_part_is_not_drawn() {
        let ws = resolved(
            "part def Wheel { port hub; port rim; }\n\
             part def Car {\n\
             \tpart w : Wheel;\n\
             \tconnect w.hub to w.rim;\n\
             }\n",
        );
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "Car"));
        assert_eq!(diagram.nodes.len(), 1);
        assert!(diagram.edges.is_empty());
    }

    #[test]
    fn an_untyped_part_keeps_its_bare_name() {
        let ws = resolved("part def Car {\n\tpart w;\n}\n");
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "Car"));
        assert_eq!(diagram.nodes[0].name, "w");
    }

    #[test]
    fn an_unnamed_part_is_skipped() {
        let mut model = Model::new();
        let definition = model.create(ElementKind::PartDefinition);
        let anonymous = model.create(ElementKind::PartUsage);
        model.add_owned(definition, anonymous);
        let diagram = interconnection_diagram(&model, definition);
        assert_eq!(diagram, Diagram::default());
    }
}

#[cfg(test)]
mod connector_box_tests {
    use super::*;
    use crate::tests::resolved;

    /// `ConnectionUsage` specializes `PartUsage`, so a named connection
    /// would become a box unless connectors are excluded.
    #[test]
    fn a_named_connection_is_an_edge_and_not_a_box() {
        let ws = resolved(
            "part def Wheel { port hub; }\n\
             part def Axle { port mount; }\n\
             connection def Link;\n\
             part def Car {\n\
             \tattribute mass;\n\
             \tpart w : Wheel;\n\
             \tpart a : Axle;\n\
             \tconnect wheelToAxle : Link connect w.hub to a.mount;\n\
             }\n",
        );
        let car = ws
            .named_elements()
            .find(|(_, name)| *name == "Car")
            .map(|(id, _)| id)
            .unwrap();

        let inner = interconnection_diagram(ws.model(), car);
        let names: Vec<&str> = inner.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["w : Wheel", "a : Axle"]);

        // the same exclusion keeps it out of the definition diagram's
        // composition edges, where it would otherwise link Car to Link
        let outer = definition_diagram(ws.model(), &[ws.root()]);
        let composed: Vec<&str> = outer
            .edges
            .iter()
            .filter(|e| e.relation == Relation::Composition)
            .map(|e| outer.nodes[e.to].name.as_str())
            .collect();
        assert_eq!(composed, ["Wheel", "Axle"]);
    }
}

#[cfg(test)]
mod behaviour_tests {
    use super::*;
    use crate::tests::resolved;

    fn internal(source: &str, name: &str) -> Diagram {
        let ws = resolved(source);
        let owner = ws
            .named_elements()
            .find(|(_, declared)| *declared == name)
            .map(|(id, _)| id)
            .unwrap();
        interconnection_diagram(ws.model(), owner)
    }

    #[test]
    fn a_state_definition_draws_its_states_and_transitions() {
        let diagram = internal(
            "state def Modes {\n\
             \tstate off;\n\
             \tstate on;\n\
             \ttransition off_to_on first off then on;\n\
             \ttransition on_to_off first on then off;\n\
             }\n",
            "Modes",
        );

        let names: Vec<&str> = diagram.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["off", "on"]);
        assert_eq!(
            diagram.edges,
            [
                Edge {
                    from: 0,
                    to: 1,
                    relation: Relation::Transition,
                    ends: None,
                },
                Edge {
                    from: 1,
                    to: 0,
                    relation: Relation::Transition,
                    ends: None,
                },
            ]
        );
    }

    #[test]
    fn an_action_definition_draws_its_successions() {
        let diagram = internal(
            "action def Flow {\n\
             \taction a;\n\
             \tmerge m;\n\
             \taction b;\n\
             \tfirst a then m;\n\
             \tfirst m then b;\n\
             }\n",
            "Flow",
        );

        // the merge node is one of the boxes the flow runs through
        let names: Vec<&str> = diagram.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["a", "m", "b"]);
        assert!(diagram
            .edges
            .iter()
            .all(|e| e.relation == Relation::Transition));
        assert_eq!(diagram.edges.len(), 2);
    }

    /// `TransitionUsage` is an `ActionUsage`, so a named transition would
    /// become a box unless relationships are excluded.
    #[test]
    fn a_named_transition_is_an_edge_and_not_a_box() {
        let diagram = internal(
            "state def Modes {\n\
             \tstate off;\n\
             \tstate on;\n\
             \ttransition named first off then on;\n\
             }\n",
            "Modes",
        );
        assert_eq!(diagram.nodes.len(), 2);
        assert_eq!(diagram.edges.len(), 1);
    }
}

#[cfg(test)]
mod initial_tests {
    use super::*;
    use crate::tests::resolved;

    #[test]
    fn an_entry_succession_starts_from_a_filled_circle() {
        let ws = resolved(
            "state def Modes {\n\
             \tentry; then off;\n\
             \tstate off;\n\
             \tstate on;\n\
             \ttransition first off then on;\n\
             }\n",
        );
        let modes = ws
            .named_elements()
            .find(|(_, name)| *name == "Modes")
            .map(|(id, _)| id)
            .unwrap();
        let diagram = interconnection_diagram(ws.model(), modes);

        // two states plus the circle the machine starts at
        assert_eq!(diagram.nodes.len(), 3);
        let initial = diagram
            .nodes
            .iter()
            .position(|n| n.shape == Shape::Initial)
            .unwrap();
        assert!(diagram.nodes[initial].name.is_empty());
        assert!(diagram
            .edges
            .iter()
            .any(|e| e.from == initial && e.relation == Relation::Transition));
        assert_eq!(diagram.edges.len(), 2);
    }

    #[test]
    fn a_one_ended_connector_that_is_not_a_succession_is_skipped() {
        // `bind w = 1;` resolves one operand, and a binding is not the
        // entry point of anything
        let ws = resolved("part def Car {\n\tpart w;\n\tbind w = 1;\n}\n");
        let car = ws
            .named_elements()
            .find(|(_, name)| *name == "Car")
            .map(|(id, _)| id)
            .unwrap();
        let diagram = interconnection_diagram(ws.model(), car);
        assert_eq!(diagram.nodes.len(), 1);
        assert!(diagram.edges.is_empty());
    }

    #[test]
    fn an_entry_succession_into_nothing_drawable_is_skipped() {
        let ws = resolved("state def Modes {\n\tentry; then elsewhere;\n}\n");
        let modes = ws
            .named_elements()
            .find(|(_, name)| *name == "Modes")
            .map(|(id, _)| id)
            .unwrap();
        assert_eq!(
            interconnection_diagram(ws.model(), modes),
            Diagram::default()
        );
    }
}

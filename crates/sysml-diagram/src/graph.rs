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

/// One box: a named definition and the features it declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub id: ElementId,
    pub name: String,
    /// SysML keyword shown in guillemets, e.g. `part def`.
    pub keyword: String,
    pub features: Vec<Feature>,
}

/// A specialization: `from` (the subtype) specializes `to` (the supertype).
/// Both fields index [`Diagram::nodes`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
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
                edges.push(Edge { from, to });
            }
        }
    }

    Diagram { nodes, edges }
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

/// The type of a usage, read off the `FeatureTyping` that name resolution
/// reified for its `:` clause.
fn type_of(model: &Model, usage: ElementId) -> Option<String> {
    model.owned(usage).iter().find_map(|&rel| {
        if model.kind(rel) != ElementKind::FeatureTyping {
            return None;
        }
        match model.get(rel, "type") {
            Some(Value::Ref(target)) => model.name(*target).map(str::to_string),
            _ => None,
        }
    })
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

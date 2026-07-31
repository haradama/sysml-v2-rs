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
    /// Declared multiplicity, rendered the way it was written: `[4]`.
    pub multiplicity: Option<String>,
    /// Declared default or initial value, rendered as ` = 4` / ` := 4`.
    pub value: Option<String>,
}

impl Feature {
    /// The compartment line as it appears in the drawing.
    pub fn label(&self) -> String {
        let mut line = format!("{} {}", self.keyword, self.name);
        if let Some(ty) = &self.ty {
            line.push_str(&format!(" : {ty}"));
        }
        if let Some(multiplicity) = &self.multiplicity {
            line.push_str(multiplicity);
        }
        if let Some(value) = &self.value {
            line.push_str(value);
        }
        line
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
    /// Whether the element is declared `abstract`, which the drawing shows
    /// the UML way: the name set in italic.
    pub is_abstract: bool,
    pub shape: Shape,
    /// The parts this box is itself assembled from, drawn inside it. Only
    /// an interconnection view fills this, and only one level deep.
    pub children: Vec<Node>,
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
    /// `from` satisfies the requirement `to` (`satisfy r by p`). Drawn the
    /// SysML way, as a dashed dependency pointing at the requirement.
    Satisfy,
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
    /// A name for the relationship itself, drawn beside the line. Only a
    /// transition carries one: `transition subscribing first ... then ...`.
    pub label: Option<String>,
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
                is_abstract: is_abstract(model, id),
                shape: Shape::Box,
                children: Vec::new(),
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
                    label: None,
                });
            }
        }
        compositions_of(model, node.id, from, &index, &mut edges);
        // `connection def D { end a : A; end b : B; }` relates the
        // definitions its ends are typed by, which is the only thing
        // holding them together in a definition diagram
        for (target, end) in connector_ends(model, node.id) {
            let Some(&to) = index.get(&target) else {
                continue;
            };
            if from != to {
                edges.push(Edge {
                    from,
                    to,
                    relation: Relation::Connection,
                    ends: None,
                    label: Some(end),
                });
            }
        }
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
        if !is_box(model, child) {
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
        let children = nested_parts(model, child);
        let mut features = features_with_type(model, child);
        // whatever became a box inside is not also a compartment line
        features.retain(|feature| children.is_empty() || feature.keyword != "part");
        index.insert(child, nodes.len());
        nodes.push(Node {
            id: child,
            name: label,
            keyword: keyword(model.kind(child)),
            features,
            is_abstract: is_abstract(model, child),
            shape: Shape::Box,
            children,
        });
    }

    let mut edges = Vec::new();
    // `action A1; then J;` continues from whatever came before it, so a
    // succession that names only where it goes needs its source remembered
    let mut previous: Option<usize> = None;
    for &child in model.owned(definition) {
        if let Some(&drawn) = index.get(&child) {
            previous = Some(drawn);
        }
        if model.kind(child).is_a(ElementKind::SatisfyRequirementUsage) {
            push_satisfaction(model, child, &index, &mut edges);
            continue;
        }

        let ends = connector_ends(model, child);
        // `then J;` names only where the flow goes. Its source is whatever
        // stands before it, and when nothing does -- `entry; then off;` --
        // it is the filled circle a machine starts at.
        if let [only] = &ends[..] {
            if model.kind(child) == ElementKind::SuccessionAsUsage {
                if let Some(&to) = index.get(&only.0) {
                    let from = previous.unwrap_or(nodes.len());
                    if previous.is_none() {
                        nodes.push(Node {
                            id: child,
                            name: String::new(),
                            keyword: String::new(),
                            features: Vec::new(),
                            is_abstract: false,
                            shape: Shape::Initial,
                            children: Vec::new(),
                        });
                    }
                    if from != to {
                        edges.push(Edge {
                            from,
                            to,
                            relation: Relation::Transition,
                            ends: None,
                            label: None,
                        });
                    }
                    // the flow now stands where it just arrived
                    previous = Some(to);
                }
            }
            continue;
        }
        // an n-ary connection -- `connection { end ::> a; end ::> b; end
        // ::> c; }` -- fans out from the end written first, which is the
        // one the others relate to
        let Some((first, rest)) = ends.split_first() else {
            continue;
        };
        let Some(&from) = index.get(&first.0) else {
            continue;
        };
        let directed = model.kind(child).is_a(ElementKind::TransitionUsage)
            || model.kind(child) == ElementKind::SuccessionAsUsage;
        for second in rest {
            let Some(&to) = index.get(&second.0) else {
                continue;
            };
            if from == to {
                continue;
            }
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
                ends: (!directed).then(|| (first.1.clone(), second.1.clone())),
                // `off_to_on / send action`, after the UML convention of
                // naming the step and then what it does
                label: directed.then(|| transition_label(model, child)).flatten(),
            });
        }
    }

    Diagram { nodes, edges }
}

/// What to write beside a transition, after the UML reading of `trigger /
/// effect`: its own name, the payload it waits for, and the action it
/// performs on the way across, as far as it declares each.
fn transition_label(model: &Model, transition: ElementId) -> Option<String> {
    let mut head: Vec<String> = Vec::new();
    if let Some(name) = model.name(transition) {
        head.push(name.to_string());
    }
    if let Some(trigger) = first_reference(model, transition, "triggerAction") {
        let payload = model.name(trigger).unwrap_or_default();
        let typed = type_of(model, trigger)
            .map(|ty| format!(" : {ty}"))
            .unwrap_or_default();
        head.push(format!("accept {payload}{typed}"));
    }
    if let Some(guard) = guard_text(model, transition) {
        head.push(format!("[{guard}]"));
    }
    let effect = first_reference(model, transition, "effectAction").map(|a| keyword(model.kind(a)));
    match (head.is_empty(), effect) {
        (true, effect) => effect,
        (false, Some(effect)) => Some(format!("{} / {effect}", head.join(" "))),
        (false, None) => Some(head.join(" ")),
    }
}

/// The condition a transition is guarded by, as it was written. The model
/// keeps it as a textual representation of the guard expression, since the
/// expression tree itself is not made of elements.
fn guard_text(model: &Model, transition: ElementId) -> Option<String> {
    let guard = first_reference(model, transition, "guardExpression")?;
    // the representation is the only thing the reified guard owns
    let written = *model.owned(guard).first()?;
    model.get(written, "body")?.as_str().map(str::to_string)
}

/// The first element a `RefList` property points at.
fn first_reference(model: &Model, element: ElementId, property: &str) -> Option<ElementId> {
    match model.get(element, property) {
        Some(Value::RefList(items)) => items.first().copied(),
        _ => None,
    }
}

/// Draw `satisfy r by p;` as an edge from the satisfying feature to the
/// requirement.
///
/// `satisfy requirement r : R by p;` declares the requirement inline, so
/// the assertion itself stands for it -- but that form is an edge here, not
/// a box, and there is nothing on the canvas to point at.
fn push_satisfaction(
    model: &Model,
    assertion: ElementId,
    index: &HashMap<ElementId, usize>,
    edges: &mut Vec<Edge>,
) {
    // in the declaring form the assertion is the requirement
    let requirement =
        single_reference(model, assertion, "satisfiedRequirement").unwrap_or(assertion);
    let Some(satisfier) = single_reference(model, assertion, "satisfyingFeature") else {
        return;
    };
    let (Some(&to), Some(&from)) = (index.get(&requirement), index.get(&satisfier)) else {
        return;
    };
    if from != to {
        edges.push(Edge {
            from,
            to,
            relation: Relation::Satisfy,
            ends: None,
            label: Some("satisfy".to_string()),
        });
    }
}

/// The element a single-valued reference property points at.
fn single_reference(model: &Model, element: ElementId, property: &str) -> Option<ElementId> {
    match model.get(element, property) {
        Some(Value::Ref(target)) => Some(*target),
        _ => None,
    }
}

/// An end declared with a type rather than a reference, as `connection def
/// Req1_Derivation { end #original r1 : Req1; }`. What it is typed by is
/// the box it reaches; its own name goes on the line.
fn typed_end(model: &Model, end: ElementId) -> Option<(ElementId, String)> {
    if !matches!(model.get(end, "isEnd"), Some(Value::Bool(true))) {
        return None;
    }
    let target = type_reference(model, end)?;
    Some((target, model.name(end).unwrap_or_default().to_string()))
}

/// Whether an element gets a box of its own in an interconnection view.
///
/// A satisfy assertion is normally only an edge, but `satisfy requirement
/// r : R by p;` declares the requirement rather than naming one, so there
/// the assertion is what the edge has to point at.
fn is_box(model: &Model, element: ElementId) -> bool {
    let kind = model.kind(element);
    if kind.is_a(ElementKind::SatisfyRequirementUsage) {
        return single_reference(model, element, "satisfiedRequirement").is_none();
    }
    is_structure_box(kind)
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
        || kind.is_a(ElementKind::ActionUsage)
        || kind.is_a(ElementKind::RequirementUsage);
    let relates = kind.is_a(ElementKind::ConnectorAsUsage)
        || kind.is_a(ElementKind::TransitionUsage)
        // `satisfy r by p;` is a requirement usage in the metamodel, but
        // what it says is a relationship between two other things
        || kind.is_a(ElementKind::SatisfyRequirementUsage);
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
        .filter_map(|&end| {
            chained_end(model, end)
                .or_else(|| referenced_end(model, end))
                .or_else(|| typed_end(model, end))
        })
        .collect()
}

/// An end written inline, as `connect w.hub to a.mount`.
fn chained_end(model: &Model, end: ElementId) -> Option<(ElementId, String)> {
    let Some(Value::RefList(chain)) = model.get(end, "chainingFeature") else {
        return None;
    };
    let part = *chain.first()?;
    let feature = model.name(*chain.last()?).unwrap_or_default();
    Some((part, feature.to_string()))
}

/// An end declared as its own member, as `end ::> vehicleMassRequirement;`.
/// What it references is both the box and the name to put on the line.
fn referenced_end(model: &Model, end: ElementId) -> Option<(ElementId, String)> {
    // only a reference subsetting carries `referencedFeature`, so the
    // property alone picks the relationship out of whatever the end owns
    model
        .owned(end)
        .iter()
        .find_map(|&rel| match model.get(rel, "referencedFeature") {
            Some(Value::Ref(target)) => {
                Some((*target, model.name(*target).unwrap_or_default().to_string()))
            }
            _ => None,
        })
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
            label: None,
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
pub(crate) fn keyword(kind: ElementKind) -> String {
    let name = kind.name();
    // `Usage` and `Definition` are the abstract bases: stripping the suffix
    // would leave nothing, so they answer to their own name
    let (base, suffix) = match (name.strip_suffix("Definition"), name.strip_suffix("Usage")) {
        (Some(base), _) if !base.is_empty() => (base, " def"),
        (_, Some(base)) if !base.is_empty() => (base, ""),
        _ => (name, ""),
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

/// The parts a box is assembled from, as boxes to draw inside it.
///
/// Taken from the usage and then its type, the same way its features are:
/// `part w : Wheel;` declares nothing itself, so the sub-parts come from
/// `Wheel`. Nesting stops here -- one level is what a box has room for.
fn nested_parts(model: &Model, usage: ElementId) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::new();
    let owners = [Some(usage), type_reference(model, usage)];
    for owner in owners.into_iter().flatten() {
        for &part in model.owned(owner) {
            if !model.kind(part).is_a(ElementKind::PartUsage) || !is_structure_box(model.kind(part))
            {
                continue;
            }
            let Some(name) = model.name(part) else {
                continue;
            };
            let label = match type_of(model, part) {
                Some(ty) => format!("{name} : {ty}"),
                None => name.to_string(),
            };
            if out.iter().any(|drawn| drawn.name == label) {
                continue;
            }
            out.push(Node {
                id: part,
                name: label,
                keyword: keyword(model.kind(part)),
                features: Vec::new(),
                is_abstract: is_abstract(model, part),
                shape: Shape::Box,
                children: Vec::new(),
            });
        }
    }
    out
}

/// A part's own features followed by the ones its type declares.
///
/// `part w : Wheel;` usually declares nothing itself -- its ports come from
/// `Wheel` -- so a box listing only what the usage writes would be empty
/// even where a connection attaches to one of those ports. A feature the
/// usage redefines keeps the usage's own entry.
fn features_with_type(model: &Model, usage: ElementId) -> Vec<Feature> {
    let mut out = features_of(model, usage);
    let Some(ty) = type_reference(model, usage) else {
        return out;
    };
    for inherited in features_of(model, ty) {
        if !out.iter().any(|own| own.name == inherited.name) {
            out.push(inherited);
        }
    }
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
            multiplicity: multiplicity_of(model, child),
            value: value_of(model, child),
        });
    }
    out
}

/// Was the element declared `abstract`? A definition that is has no
/// instances of its own, which the drawing is expected to say.
fn is_abstract(model: &Model, element: ElementId) -> bool {
    model.get(element, "isAbstract") == Some(&Value::Bool(true))
}

/// The multiplicity a usage was declared with, as the text between its
/// brackets: `[4]`, `[0..1]`, `[*]`.
fn multiplicity_of(model: &Model, usage: ElementId) -> Option<String> {
    let Some(Value::Ref(range)) = model.get(usage, "multiplicity") else {
        return None;
    };
    let bound = |name: &str| -> Option<String> {
        let Some(Value::Ref(bound)) = model.get(*range, name) else {
            return None;
        };
        Some(expression_text(model, *bound))
    };
    match (bound("bound"), bound("lowerBound"), bound("upperBound")) {
        (Some(only), _, _) => Some(format!("[{only}]")),
        (None, Some(lower), Some(upper)) => Some(format!("[{lower}..{upper}]")),
        _ => None,
    }
}

/// The value a usage was declared with, as the clause that set it:
/// ` = 1200.0` for a value, ` := x` for an initial one.
fn value_of(model: &Model, usage: ElementId) -> Option<String> {
    let membership = model
        .owned(usage)
        .iter()
        .copied()
        .find(|&child| model.kind(child) == ElementKind::FeatureValue)?;
    let Some(Value::Ref(expression)) = model.get(membership, "value") else {
        return None;
    };
    let wrote = if model.get(membership, "isInitial") == Some(&Value::Bool(true)) {
        ":="
    } else {
        "="
    };
    Some(format!(" {wrote} {}", expression_text(model, *expression)))
}

/// An expression the way the source wrote it: a literal renders its value,
/// anything else kept its text when it was built.
fn expression_text(model: &Model, expression: ElementId) -> String {
    if model.kind(expression) == ElementKind::LiteralInfinity {
        return "*".to_string();
    }
    match model.get(expression, "value") {
        Some(Value::Int(int)) => return int.to_string(),
        Some(Value::Real(real)) => return format!("{real:?}"),
        Some(Value::Bool(bool)) => return bool.to_string(),
        Some(Value::String(string)) => return format!("\"{string}\""),
        _ => {}
    }
    // a non-literal expression carries the text it was written as
    model
        .owned(expression)
        .iter()
        .copied()
        .find(|&child| model.kind(child) == ElementKind::TextualRepresentation)
        .and_then(|written| model.get(written, "body"))
        .and_then(|body| body.as_str())
        .unwrap_or("...")
        .to_string()
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
    fn a_compartment_line_carries_multiplicity_and_value() {
        let ws = resolved(
            "part def Wheel;\npart def V {\n\tpart wheels : Wheel[4];\n\tattribute m = 1.5;\n}\n",
        );
        let diagram = crate::definition_diagram(ws.model(), &[ws.root()]);
        let v = diagram
            .nodes
            .iter()
            .find(|node| node.name == "V")
            .expect("V is drawn");
        let labels: Vec<String> = v.features.iter().map(Feature::label).collect();
        assert_eq!(labels, ["part wheels : Wheel[4]", "attribute m = 1.5"]);
    }

    #[test]
    fn a_compartment_line_survives_odd_declarations() {
        // three bounds fit no range, `=;` sets nothing, a string renders
        // quoted -- none of them may break the label
        let ws = resolved(
            "part def W;\npart def V {\n\tattribute a =;\n\tpart w : W[1..2..3];\n\
             \tattribute s = \"boot\";\n}\n",
        );
        let diagram = crate::definition_diagram(ws.model(), &[ws.root()]);
        let v = diagram
            .nodes
            .iter()
            .find(|node| node.name == "V")
            .expect("V is drawn");
        let labels: Vec<String> = v.features.iter().map(Feature::label).collect();
        assert_eq!(
            labels,
            ["attribute a", "part w : W", "attribute s = \"boot\"",]
        );
    }

    #[test]
    fn an_abstract_definition_is_marked_as_one() {
        let ws = resolved("abstract part def PowerSource;\npart def Engine;\n");
        let diagram = crate::definition_diagram(ws.model(), &[ws.root()]);
        let marked: Vec<(&str, bool)> = diagram
            .nodes
            .iter()
            .map(|node| (node.name.as_str(), node.is_abstract))
            .collect();
        assert_eq!(marked, vec![("PowerSource", true), ("Engine", false)]);
    }

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
                    multiplicity: None,
                    value: None,
                },
                Feature {
                    keyword: "port".to_string(),
                    name: "fuelIn".to_string(),
                    ty: Some("FuelPort".to_string()),
                    multiplicity: None,
                    value: None,
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
    fn a_connection_definition_relates_what_its_ends_are_typed_by() {
        // nothing else holds these three together: no `:>`, no parts
        let ws = resolved(
            "requirement def Req1;\n\
             requirement def Req1_1;\n\
             requirement def Req1_2;\n\
             connection def Derivation {\n\
             \tend r1 : Req1;\n\
             \tend r1_1 : Req1_1;\n\
             \tend r1_2 : Req1_2;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let names: Vec<&str> = diagram.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["Req1", "Req1_1", "Req1_2", "Derivation"]);

        // one line out of the connection to each end's type, named for it
        let drawn: Vec<(&str, Option<&str>)> = diagram
            .edges
            .iter()
            .map(|e| (diagram.nodes[e.to].name.as_str(), e.label.as_deref()))
            .collect();
        assert_eq!(
            drawn,
            [
                ("Req1", Some("r1")),
                ("Req1_1", Some("r1_1")),
                ("Req1_2", Some("r1_2")),
            ]
        );
        assert!(diagram.edges.iter().all(|e| e.from == 3));

        // scoped at the connection alone, the ends reach outside it
        let derivation = diagram.nodes[3].id;
        let scoped = definition_diagram(ws.model(), &[derivation]);
        assert_eq!(scoped.nodes.len(), 1);
        assert!(scoped.edges.is_empty());
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
    fn the_abstract_bases_keep_their_own_name() {
        // stripping the suffix off `Usage` or `Definition` leaves nothing
        assert_eq!(keyword(ElementKind::Usage), "usage");
        assert_eq!(keyword(ElementKind::Definition), "definition");
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
                multiplicity: None,
                value: None,
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
                label: None,
            }]
        );
    }

    #[test]
    fn requirements_and_their_derivation_are_drawn() {
        // ends written as their own members, and three of them: the first
        // is what the others derive from
        let ws = resolved(
            "requirement def R;\n\
             package P {\n\
             \trequirement a : R;\n\
             \trequirement b : R;\n\
             \trequirement c : R;\n\
             \tconnection {\n\
             \t\tend ::> a;\n\
             \t\tend ::> b;\n\
             \t\tend ::> c;\n\
             \t}\n\
             }\n",
        );
        let package = ws
            .named_elements()
            .find(|(_, name)| *name == "P")
            .map(|(id, _)| id)
            .unwrap();
        let diagram = interconnection_diagram(ws.model(), package);

        let names: Vec<&str> = diagram.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["a : R", "b : R", "c : R"]);
        assert_eq!(
            diagram
                .edges
                .iter()
                .map(|e| (e.from, e.to))
                .collect::<Vec<_>>(),
            [(0, 1), (0, 2)]
        );
        assert_eq!(
            diagram.edges[0].ends,
            Some(("a".to_string(), "b".to_string()))
        );
    }

    #[test]
    fn satisfaction_is_an_edge_from_the_satisfier_to_the_requirement() {
        let ws = resolved(
            "requirement def R;\n\
             part def P;\n\
             package K {\n\
             \trequirement r : R;\n\
             \tpart p : P;\n\
             \tsatisfy r by p;\n\
             }\n",
        );
        let package = ws
            .named_elements()
            .find(|(_, name)| *name == "K")
            .map(|(id, _)| id)
            .unwrap();
        let diagram = interconnection_diagram(ws.model(), package);

        // the assertion itself is not a box: it names a requirement
        let names: Vec<&str> = diagram.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["r : R", "p : P"]);
        assert_eq!(diagram.edges.len(), 1);
        let edge = &diagram.edges[0];
        assert_eq!(edge.relation, Relation::Satisfy);
        assert_eq!((edge.from, edge.to), (1, 0));
        assert_eq!(edge.label.as_deref(), Some("satisfy"));
    }

    #[test]
    fn a_satisfaction_declaring_its_requirement_is_a_box_as_well() {
        let ws = resolved(
            "requirement def R;\n\
             part def P;\n\
             package K {\n\
             \tpart p : P;\n\
             \tsatisfy requirement r : R by p;\n\
             }\n",
        );
        let package = ws
            .named_elements()
            .find(|(_, name)| *name == "K")
            .map(|(id, _)| id)
            .unwrap();
        let diagram = interconnection_diagram(ws.model(), package);

        // nothing else stands for the requirement, so the assertion does
        let names: Vec<&str> = diagram.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["p : P", "r : R"]);
        assert_eq!(diagram.edges.len(), 1);
        assert_eq!((diagram.edges[0].from, diagram.edges[0].to), (0, 1));
    }

    #[test]
    fn a_satisfaction_without_a_satisfier_is_not_drawn() {
        let ws = resolved(
            "requirement def R;\n\
             package K {\n\
             \trequirement r : R;\n\
             \tsatisfy r;\n\
             }\n",
        );
        let package = ws
            .named_elements()
            .find(|(_, name)| *name == "K")
            .map(|(id, _)| id)
            .unwrap();
        let diagram = interconnection_diagram(ws.model(), package);
        assert_eq!(diagram.nodes.len(), 1);
        assert!(diagram.edges.is_empty());
    }

    #[test]
    fn a_satisfier_outside_the_diagram_is_not_drawn() {
        let ws = resolved(
            "requirement def R;\n\
             part def P;\n\
             part outside : P;\n\
             package K {\n\
             \trequirement r : R;\n\
             \tsatisfy r by outside;\n\
             }\n",
        );
        let package = ws
            .named_elements()
            .find(|(_, name)| *name == "K")
            .map(|(id, _)| id)
            .unwrap();
        let diagram = interconnection_diagram(ws.model(), package);
        assert_eq!(diagram.nodes.len(), 1);
        assert!(diagram.edges.is_empty());
    }

    #[test]
    fn a_typed_end_still_names_what_it_references() {
        // the end owns a typing as well as the reference
        let ws = resolved(
            "requirement def R;\n\
             package P {\n\
             \trequirement a : R;\n\
             \trequirement b : R;\n\
             \tconnection {\n\
             \t\tend e1 : R ::> a;\n\
             \t\tend e2 : R ::> b;\n\
             \t}\n\
             }\n",
        );
        let package = ws
            .named_elements()
            .find(|(_, name)| *name == "P")
            .map(|(id, _)| id)
            .unwrap();
        let diagram = interconnection_diagram(ws.model(), package);
        assert_eq!(diagram.edges.len(), 1);
        assert_eq!(
            diagram.edges[0].ends,
            Some(("a".to_string(), "b".to_string()))
        );
    }

    #[test]
    fn an_end_outside_the_definition_is_not_drawn() {
        // whichever side leaves the diagram, the line has nowhere to land
        for wiring in ["\tconnect w.hub to Wheel;\n", "\tconnect Wheel to w.hub;\n"] {
            let ws = resolved(&format!(
                "part def Wheel {{ port hub; }}\n\
                 part def Car {{\n\
                 \tpart w : Wheel;\n\
                 {wiring}\
                 }}\n"
            ));
            let diagram = interconnection_diagram(ws.model(), definition(&ws, "Car"));
            assert_eq!(diagram.nodes.len(), 1);
            assert!(diagram.edges.is_empty(), "{wiring}");
        }
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
    fn a_part_box_lists_the_ports_its_type_declares() {
        let ws = resolved(
            "part def Wheel { port hub; port rim; }\n\
             part def Car {\n\
             \tpart w : Wheel;\n\
             }\n",
        );
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "Car"));
        let lines: Vec<String> = diagram.nodes[0]
            .features
            .iter()
            .map(Feature::label)
            .collect();
        // the usage declares nothing of its own, so both come from Wheel
        assert_eq!(lines, ["port hub", "port rim"]);
    }

    #[test]
    fn a_part_box_carries_its_sub_parts_as_boxes() {
        let ws = resolved(
            "part def Bolt;\n\
             part def Wheel { port hub; part bolt : Bolt; }\n\
             part def Car {\n\
             \tpart w : Wheel;\n\
             }\n",
        );
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "Car"));
        let wheel = &diagram.nodes[0];

        let nested: Vec<&str> = wheel.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(nested, ["bolt : Bolt"]);
        // the sub-part is a box now, so it is not also a compartment line
        let lines: Vec<String> = wheel.features.iter().map(Feature::label).collect();
        assert_eq!(lines, ["port hub"]);
        // nesting stops at one level
        assert!(wheel.children[0].children.is_empty());
    }

    #[test]
    fn nested_parts_skip_what_cannot_be_drawn() {
        let ws = resolved(
            "part def Bolt;\n\
             part def Wheel { part bolt : Bolt; part loose; }\n\
             part def Car {\n\
             \tpart w : Wheel { part bolt : Bolt; }\n\
             }\n",
        );
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "Car"));
        let nested: Vec<&str> = diagram.nodes[0]
            .children
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        // the usage writes `bolt` too, and the type's repeat is not drawn
        // twice; an untyped sub-part keeps its bare name
        assert_eq!(nested, ["bolt : Bolt", "loose"]);
    }

    #[test]
    fn an_unnamed_sub_part_is_skipped() {
        let mut model = Model::new();
        let definition = model.create(ElementKind::PartDefinition);
        let part = model.create(ElementKind::PartUsage);
        model.set(part, "declaredName", Value::String("w".to_string()));
        model.add_owned(definition, part);
        let anonymous = model.create(ElementKind::PartUsage);
        model.add_owned(part, anonymous);

        let diagram = interconnection_diagram(&model, definition);
        assert_eq!(diagram.nodes.len(), 1);
        assert!(diagram.nodes[0].children.is_empty());
    }

    #[test]
    fn a_part_redefining_a_feature_keeps_its_own_entry() {
        let ws = resolved(
            "port def Fast;\n\
             part def Wheel { port hub; }\n\
             part def Car {\n\
             \tpart w : Wheel { port hub : Fast; }\n\
             }\n",
        );
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "Car"));
        let lines: Vec<String> = diagram.nodes[0]
            .features
            .iter()
            .map(Feature::label)
            .collect();
        assert_eq!(lines, ["port hub : Fast"]);
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
        // a named transition carries its name onto the arrow
        assert_eq!(
            diagram.edges,
            [
                Edge {
                    from: 0,
                    to: 1,
                    relation: Relation::Transition,
                    ends: None,
                    label: Some("off_to_on".to_string()),
                },
                Edge {
                    from: 1,
                    to: 0,
                    relation: Relation::Transition,
                    ends: None,
                    label: Some("on_to_off".to_string()),
                },
            ]
        );
    }

    #[test]
    fn a_transition_label_names_the_step_and_what_it_does() {
        let diagram = internal(
            "part def B { port p; }\n\
             state def S {\n\
             \tstate a;\n\
             \tstate b;\n\
             \tpart sink : B;\n\
             \ttransition t1 first a do send 1 to sink.p then b;\n\
             \ttransition first b do send 2 to sink.p then a;\n\
             }\n",
            "S",
        );
        let labels: Vec<Option<&str>> = diagram.edges.iter().map(|e| e.label.as_deref()).collect();
        // the unnamed one still says what it does on the way across
        assert_eq!(labels, [Some("t1 / send action"), Some("send action")]);
    }

    #[test]
    fn a_transition_label_carries_the_payload_it_waits_for() {
        let diagram = internal(
            "item def P;\n\
             state def S {\n\
             \tstate a;\n\
             \tstate b;\n\
             \ttransition t1 first a accept pub : P then b;\n\
             }\n",
            "S",
        );
        assert_eq!(diagram.edges[0].label.as_deref(), Some("t1 accept pub : P"));

        // the whole UML reading: name, trigger, guard, effect
        let full = internal(
            "item def P;\n\
             part def B { port pt; }\n\
             state def S {\n\
             \tstate a;\n\
             \tstate b;\n\
             \tpart sink : B;\n\
             \ttransition t1 first a accept pub : P if pub != null \
             do send 1 to sink.pt then b;\n\
             }\n",
            "S",
        );
        assert_eq!(
            full.edges[0].label.as_deref(),
            Some("t1 accept pub : P [pub != null] / send action")
        );

        // an untyped payload still names what the transition waits for
        let untyped = internal(
            "state def S {\n\
             \tstate a;\n\
             \tstate b;\n\
             \ttransition first a accept pub then b;\n\
             }\n",
            "S",
        );
        assert_eq!(untyped.edges[0].label.as_deref(), Some("accept pub"));
    }

    #[test]
    fn an_unnamed_succession_has_no_label() {
        let diagram = internal(
            "action def Flow {\n\
             \taction a;\n\
             \taction b;\n\
             \tfirst a then b;\n\
             }\n",
            "Flow",
        );
        assert_eq!(diagram.edges.len(), 1);
        assert_eq!(diagram.edges[0].label, None);
    }

    #[test]
    fn a_then_succession_continues_from_what_stands_before_it() {
        // `action A1; then J;` is the shorthand chain: A1 flows into J.
        // Only a `then` with nothing before it starts from a circle.
        let diagram = internal(
            "action def Flow {\n\
             \taction a;\n\
             \tthen j;\n\
             \tjoin j;\n\
             \tthen b;\n\
             \taction b;\n\
             }\n",
            "Flow",
        );
        let names: Vec<&str> = diagram.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["a", "j", "b"]);
        assert!(diagram.nodes.iter().all(|n| n.shape == Shape::Box));
        assert_eq!(
            diagram
                .edges
                .iter()
                .map(|e| (e.from, e.to))
                .collect::<Vec<_>>(),
            [(0, 1), (1, 2)]
        );

        // a succession into something that is not drawn has nowhere to go
        let nowhere = internal(
            "action def Flow {\n\
             \tattribute x;\n\
             \tthen x;\n\
             }\n",
            "Flow",
        );
        assert!(nowhere.nodes.is_empty());
        assert!(nowhere.edges.is_empty());
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

//! Turning a resolved [`Model`] into the graph a diagram draws.

use std::collections::{HashMap, HashSet};

use sysml_model::{ElementId, ElementKind, Model, Role, Value};

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
    /// What the box holds, in the labelled compartments the standard
    /// stacks under the name: `attributes`, `parts`, `ports` and the
    /// rest. `extended-def = extended-def-name-compartment
    /// compartment-stack`, and `compartment-stack = (compartment)*`.
    pub compartments: Vec<Compartment>,
    /// Whether the element is declared `abstract`, which the drawing shows
    /// the UML way: the name set in italic.
    pub is_abstract: bool,
    /// The notation rounds the corners of usages and leaves definitions
    /// square.
    pub rounded: bool,
    pub shape: Shape,
    /// The parts this box is itself assembled from, drawn inside it. Only
    /// an interconnection view fills this, and only one level deep.
    pub children: Vec<Node>,
}

/// Gather features into the compartments the standard stacks them in,
/// keeping the order they were declared and the order the compartments
/// were first needed.
fn into_compartments(lines: Vec<(&'static str, Feature)>) -> Vec<Compartment> {
    let mut out: Vec<Compartment> = Vec::new();
    for (label, line) in lines {
        match out.iter_mut().find(|already| already.label == label) {
            Some(already) => already.lines.push(line),
            None => out.push(Compartment {
                label,
                lines: vec![line],
            }),
        }
    }
    out
}

/// Every line of a box, whichever compartment it is in.
///
/// A reader of the model often wants what the box holds rather than how
/// the standard files it, and one iterator is easier to be right about
/// than a fold over the stack at each call site.
pub fn lines(node: &Node) -> impl Iterator<Item = &Feature> {
    node.compartments
        .iter()
        .flat_map(|compartment| compartment.lines.iter())
}

/// One labelled compartment of a box.
///
/// The standard names every compartment after what it holds -- the
/// figure for `parts-compartment` carries the word `parts` -- and a
/// reader tells a part from a port by which compartment it is in rather
/// than by the keyword on the line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compartment {
    /// The word the standard writes in the compartment, e.g. `parts`.
    pub label: &'static str,
    pub lines: Vec<Feature>,
}

/// Which compartment a member belongs in.
///
/// One arm per compartment the standard admits for a feature of that
/// kind, read off `compartment = | attributes-compartment | ...` and
/// the `*-compartment-element` production under each. `features` is
/// where anything else goes: it is the one the standard leaves open.
fn compartment_of(model: &Model, member: ElementId) -> &'static str {
    if let Some(role) = model.member_role(member) {
        return match role {
            Role::Subject => "subject",
            Role::Actor => "actors",
            Role::Stakeholder => "stakeholders",
            Role::Objective => "objective",
            Role::Frame => "frames",
            Role::Verify => "verifies",
            Role::Assume => "assume constraints",
            Role::Require => "require constraints",
            Role::Entry | Role::Do | Role::Exit => "state actions",
            Role::Variant => "variants",
            Role::Return | Role::Result => "result",
        };
    }
    if model.get(member, "direction").is_some() {
        return "parameters";
    }
    let kind = model.kind(member);
    for (metaclass, label) in [
        (ElementKind::PerformActionUsage, "perform actions"),
        (ElementKind::AllocationUsage, "allocations"),
        (ElementKind::InterfaceUsage, "interfaces"),
        (ElementKind::ConnectionUsage, "connections"),
        (ElementKind::FlowUsage, "flows"),
        (ElementKind::ExhibitStateUsage, "exhibit states"),
        (ElementKind::StateUsage, "states"),
        (ElementKind::CalculationUsage, "calculations"),
        (ElementKind::AssertConstraintUsage, "assert constraints"),
        (ElementKind::RequirementUsage, "requirements"),
        (ElementKind::ConstraintUsage, "constraints"),
        (ElementKind::VerificationCaseUsage, "verifications"),
        (ElementKind::AnalysisCaseUsage, "analyses"),
        (ElementKind::UseCaseUsage, "use cases"),
        (ElementKind::ViewUsage, "views"),
        (ElementKind::ViewpointUsage, "viewpoints"),
        (ElementKind::RenderingUsage, "rendering"),
        (ElementKind::ActionUsage, "actions"),
        (ElementKind::PortUsage, "ports"),
        (ElementKind::PartUsage, "parts"),
        (ElementKind::EnumerationUsage, "enums"),
        (ElementKind::AttributeUsage, "attributes"),
        (ElementKind::OccurrenceUsage, "occurrences"),
        (ElementKind::ItemUsage, "items"),
    ] {
        if kind.is_a(metaclass) {
            return label;
        }
    }
    "features"
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
    /// `from` declares a feature typed by `to` that it does not own
    /// (`part def Trip { ref part driver : Driver; }`), so `to` is
    /// something `from` refers to rather than something it is made of.
    Reference,
    /// `from` subsets `to` (`part big :> engine`). The standard draws it
    /// with the same hollow triangle a subclassification carries.
    Subsetting,
    /// `from` redefines `to` (`part redefines mcu`). The same triangle
    /// again, with a bar across the line.
    Redefinition,
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
            // every classifier gets a box: SysML definitions and KerML's
            // own classifiers (`classifier`, `datatype`, `assoc`, ...)
            // alike, so a KerML model draws as more than an empty page
            if !model.kind(id).is_a(ElementKind::Classifier) {
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
                compartments: into_compartments(features_of(model, id)),
                is_abstract: is_abstract(model, id),
                rounded: model.kind(id).is_a(ElementKind::Usage),
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
    let members = assembled_from(model, definition);

    let mut drawn: HashMap<&str, usize> = HashMap::new();
    for &child in &members {
        if !is_box(model, child) {
            continue;
        }
        let Some(name) = effective_name(model, child) else {
            continue;
        };
        // A specialization that redeclares an inherited part names it
        // again, and the nearer one comes first. One box stands for
        // both, and both reach it: what the supertype connects is
        // written in terms of the part it declared.
        if let Some(&at) = drawn.get(name) {
            index.insert(child, at);
            continue;
        }
        drawn.insert(name, nodes.len());
        // a part is read as `role : Type`, unlike a definition's bare name
        let label = box_label(model, child, name);
        let children = nested_parts(model, child);
        let mut features = features_with_type(model, child);
        // whatever became a box inside is not also a compartment line
        features.retain(|(_, feature)| children.is_empty() || feature.keyword != "part");
        index.insert(child, nodes.len());
        nodes.push(Node {
            id: child,
            name: label,
            keyword: keyword(model.kind(child)),
            compartments: into_compartments(features),
            is_abstract: is_abstract(model, child),
            rounded: model.kind(child).is_a(ElementKind::Usage),
            shape: Shape::Box,
            children,
        });
    }

    let mut edges = Vec::new();
    specializations_of(model, definition, &index, &mut edges);
    // `action A1; then J;` continues from whatever came before it, so a
    // succession that names only where it goes needs its source remembered
    let mut previous: Option<usize> = None;
    for &child in &members {
        if let Some(&at) = index.get(&child) {
            previous = Some(at);
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
                            compartments: Vec::new(),
                            is_abstract: false,
                            rounded: false,
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
        let typed = type_name(model, trigger)
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
    // `not satisfy r by p;` asserts that it does not. Drawing it the
    // same way as `satisfy r by p;` would put the opposite of the model
    // on the canvas, and there is no line here for "does not".
    if model.get(assertion, "isNegated") == Some(&Value::Bool(true)) {
        return;
    }
    // the assertion has a box of its own only where nothing else on the
    // canvas stands for the requirement, so where a requirement is both
    // named and drawn, that is what the edge points at
    let requirement = match single_reference(model, assertion, "satisfiedRequirement") {
        Some(named) if !is_box(model, assertion) => named,
        _ => assertion,
    };
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
    let target = model.type_of(end)?;
    Some((target, model.name(end).unwrap_or_default().to_string()))
}

/// Whether an element gets a box of its own in an interconnection view.
///
/// A satisfy assertion is normally only an edge, drawn from the satisfier
/// to the requirement it names. Two forms have nothing to point at and so
/// stand for the requirement themselves: `satisfy requirement r : R by p;`,
/// which declares a requirement of its own rather than naming one already
/// on the canvas, and `satisfy requirement r by p;`, which names none.
fn is_box(model: &Model, element: ElementId) -> bool {
    let kind = model.kind(element);
    if kind.is_a(ElementKind::SatisfyRequirementUsage) {
        return model.type_of(element).is_some()
            || single_reference(model, element, "satisfiedRequirement").is_none();
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
        || kind.is_a(ElementKind::RequirementUsage)
        // KerML writes what SysML calls an action as a `step`, and a
        // behavior made of steps has to draw as more than an empty frame
        || kind.is_a(ElementKind::Step);
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
/// is only linked once. A definition holding a feature of its own type --
/// `part subcomponents : MassedThing;` inside `MassedThing` -- is drawn
/// like any other membership, back onto the box it left: the standard
/// exempts no feature from being drawn, and a recursive structure is
/// something a reader has to be able to see.
fn compositions_of(
    model: &Model,
    definition: ElementId,
    from: usize,
    index: &HashMap<ElementId, usize>,
    edges: &mut Vec<Edge>,
) {
    let mut linked: Vec<(usize, Relation)> = Vec::new();
    for &child in model.owned(definition) {
        // Every feature a type owns is drawn from the type to what the
        // feature is typed by: the standard's `type-relationship` reads
        // `composite-feature-membership | noncomposite-feature-
        // membership`, the same diamond filled or hollow, and the model
        // says which of the two on `isComposite`.
        //
        // A connector is left out. It is drawn as the edge between the
        // two things it relates, and its ends with it, so a diamond per
        // end would say the same thing a second time.
        //
        // What a feature subsets or redefines is drawn too, where both
        // are on the canvas: the standard has `subsetting` and
        // `redefinition` among its type relationships, and a `part big
        // :> engine` that is joined to nothing reads as unrelated to
        // the engine it is one of.
        if model.kind(child).is_a(ElementKind::ConnectorAsUsage)
            || model.get(child, "isEnd") == Some(&Value::Bool(true))
        {
            continue;
        }
        let relation = match model.get(child, "isComposite") {
            Some(&Value::Bool(true)) => Relation::Composition,
            Some(&Value::Bool(false)) => Relation::Reference,
            _ => continue,
        };
        let Some(&to) = model.type_of(child).and_then(|ty| index.get(&ty)) else {
            continue;
        };
        if linked.contains(&(to, relation)) {
            continue;
        }
        linked.push((to, relation));
        edges.push(Edge {
            from,
            to,
            relation,
            ends: None,
            label: None,
        });
    }
}

/// What the features of a definition subset or redefine, where the
/// drawing holds that too.
///
/// A specialization between two definitions is a subclassification; the
/// same thing between two usages is a subsetting, and a redeclaration
/// of one is a redefinition. All three are type relationships the
/// standard draws.
fn specializations_of(
    model: &Model,
    definition: ElementId,
    index: &HashMap<ElementId, usize>,
    edges: &mut Vec<Edge>,
) {
    for &child in model.owned(definition) {
        let Some(&from) = index.get(&child) else {
            continue;
        };
        for &rel in model.owned(child) {
            let (relation, names) = match model.kind(rel) {
                ElementKind::Subsetting => (Relation::Subsetting, "subsettedFeature"),
                ElementKind::Redefinition => (Relation::Redefinition, "redefinedFeature"),
                _ => continue,
            };
            let Some(&Value::Ref(target)) = model.get(rel, names) else {
                continue;
            };
            let Some(&to) = index.get(&target) else {
                continue;
            };
            if from == to {
                continue;
            }
            edges.push(Edge {
                from,
                to,
                relation,
                ends: None,
                label: None,
            });
        }
    }
}

/// The name a member answers to: its own, or -- for `part redefines mcu
/// : Atmega328p;`, which declares none -- the name of what it redefines.
///
/// A specialization narrows an inherited part by redeclaring it, and
/// the redeclaration is the nearer one and the one that says the type.
/// Reading only declared names skips it and draws the inherited part
/// instead, which is the same box under a vaguer type.
fn effective_name(model: &Model, member: ElementId) -> Option<&str> {
    if let Some(name) = model.name(member) {
        return Some(name);
    }
    model.owned(member).iter().find_map(|&rel| {
        if model.kind(rel) != ElementKind::Redefinition {
            return None;
        }
        match model.get(rel, "redefinedFeature") {
            Some(&Value::Ref(target)) => model.name(target),
            _ => None,
        }
    })
}

/// Everything a definition is assembled from: what it owns, and what it
/// inherits from the definitions it specializes, nearest first.
///
/// `part def BlinkingBoard :> ArduinoCompatibleBoard { part app : BlinkApp; }`
/// is a board with a sketch on it. Reading only what it owns draws the
/// sketch and none of the board -- and then every `connect` the board
/// declares is missing, and every `satisfy` that names one of its parts
/// points at nothing and is left standing alone on the canvas.
fn assembled_from(model: &Model, definition: ElementId) -> Vec<ElementId> {
    itself_and_supertypes(model, definition)
        .into_iter()
        .flat_map(|current| model.owned(current).iter().copied())
        .collect()
}

/// What a box calls itself: `wheels : Wheel[4]`, the way the model
/// declares it. A drawing that leaves the multiplicity off says there
/// is one of something the model said there are four of.
fn box_label(model: &Model, usage: ElementId, name: &str) -> String {
    let mut label = match type_name(model, usage) {
        Some(ty) => format!("{name} : {ty}"),
        None => name.to_string(),
    };
    if let Some(many) = multiplicity_of(model, usage) {
        label.push_str(&many);
    }
    label
}

/// A type and everything it specializes, nearest first.
///
/// Inheritance is what a drawing has to follow to say what a box holds:
/// a usage is typed by a definition that is not on the canvas, and that
/// definition is one of another that is not there either. Stopping at
/// the first hop leaves the box saying less than the model does.
fn itself_and_supertypes(model: &Model, ty: ElementId) -> Vec<ElementId> {
    let mut out = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = std::collections::VecDeque::from([ty]);
    while let Some(current) = queue.pop_front() {
        if !visited.insert(current) {
            continue;
        }
        out.push(current);
        for &rel in model.owned(current) {
            if model.kind(rel) != ElementKind::Subclassification {
                continue;
            }
            if let Some(&Value::Ref(target)) = model.get(rel, "superclassifier") {
                queue.push_back(target);
            }
        }
    }
    out
}

/// The SysML keyword a metaclass is written with: `PartDefinition` becomes
/// `part def`, `AttributeUsage` becomes `attribute`, and a multi-word
/// metaclass such as `AnalysisCaseDefinition` becomes `analysis case def`.
pub(crate) fn keyword(kind: ElementKind) -> String {
    // KerML spells a few of its keywords tighter than the metaclass name
    match kind {
        ElementKind::DataType => return "datatype".to_string(),
        ElementKind::Structure => return "struct".to_string(),
        ElementKind::Association => return "assoc".to_string(),
        ElementKind::AssociationStructure => return "assoc struct".to_string(),
        _ => {}
    }
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
    let owners = std::iter::once(usage).chain(
        model
            .type_of(usage)
            .into_iter()
            .flat_map(|ty| itself_and_supertypes(model, ty)),
    );
    for owner in owners {
        for &part in model.owned(owner) {
            if !model.kind(part).is_a(ElementKind::PartUsage) || !is_structure_box(model.kind(part))
            {
                continue;
            }
            let Some(name) = model.name(part) else {
                continue;
            };
            let label = box_label(model, part, name);
            if out.iter().any(|drawn| drawn.name == label) {
                continue;
            }
            out.push(Node {
                id: part,
                name: label,
                keyword: keyword(model.kind(part)),
                compartments: Vec::new(),
                is_abstract: is_abstract(model, part),
                rounded: model.kind(part).is_a(ElementKind::Usage),
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
fn features_with_type(model: &Model, usage: ElementId) -> Vec<(&'static str, Feature)> {
    let mut out = features_of(model, usage);
    for ty in model
        .type_of(usage)
        .into_iter()
        .flat_map(|ty| itself_and_supertypes(model, ty))
    {
        for (label, inherited) in features_of(model, ty) {
            match out
                .iter_mut()
                .map(|(_, own)| own)
                .find(|own| own.name == inherited.name)
            {
                // `attribute :>> forwardVoltage = 2 [V];` says the value
                // and leaves the type to what it redefines, so the two
                // entries are halves of one line rather than rivals
                Some(own) => {
                    own.ty = own.ty.take().or(inherited.ty);
                    own.multiplicity = own.multiplicity.take().or(inherited.multiplicity);
                    own.value = own.value.take().or(inherited.value);
                }
                None => out.push((label, inherited)),
            }
        }
    }
    out
}

/// The named features a definition declares directly, gathered into
/// the compartments the standard puts them in, in the order the
/// compartments were first needed.
///
/// Every feature, not only the usages SysML layers on top of them: a
/// KerML `step` or `feature` is what a KerML model is written out of,
/// and a box that lists only usages is an empty box on every page of
/// one.
fn features_of(model: &Model, definition: ElementId) -> Vec<(&'static str, Feature)> {
    let mut out = Vec::new();
    for &child in model.owned(definition) {
        if !model.kind(child).is_a(ElementKind::Feature) {
            continue;
        }
        let Some(name) = effective_name(model, child) else {
            continue;
        };
        out.push((
            compartment_of(model, child),
            Feature {
                keyword: keyword(model.kind(child)),
                name: name.to_string(),
                ty: type_name(model, child),
                multiplicity: multiplicity_of(model, child),
                value: value_of(model, child),
            },
        ));
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
/// resolution reified for its `:` clause -- or, where it wrote no `:`
/// clause, off whatever it subsets or redefines.
///
/// `part big :> engine;` is a part of the same kind `engine` is; drawing
/// it bare says less than the model does, and leaves a box that looks
/// like it stands for nothing in particular.
fn type_name(model: &Model, usage: ElementId) -> Option<String> {
    if let Some(target) = model.type_of(usage) {
        return model.name(target).map(str::to_string);
    }
    let mut visited = HashSet::new();
    let mut queue = std::collections::VecDeque::from([usage]);
    while let Some(current) = queue.pop_front() {
        if !visited.insert(current) {
            continue;
        }
        if let Some(target) = model.type_of(current) {
            return model.name(target).map(str::to_string);
        }
        for &rel in model.owned(current) {
            let names = match model.kind(rel) {
                ElementKind::Subsetting => "subsettedFeature",
                ElementKind::Redefinition => "redefinedFeature",
                ElementKind::ReferenceSubsetting => "referencedFeature",
                _ => continue,
            };
            if let Some(&Value::Ref(target)) = model.get(rel, names) {
                queue.push_back(target);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::resolved;

    /// What a box says of itself, for one internal view.
    fn boxes(source: &str, owner: &str) -> Vec<(String, Vec<String>)> {
        let ws = resolved(source);
        let model = ws.model();
        let target = model
            .descendants(ws.root())
            .into_iter()
            .find(|&id| model.name(id) == Some(owner))
            .expect("the owner is declared");
        interconnection_diagram(model, target)
            .nodes
            .iter()
            .map(|node| (node.name.clone(), lines(node).map(Feature::label).collect()))
            .collect()
    }

    #[test]
    fn a_box_says_what_the_model_says_of_the_part() {
        // the type it was declared with, how many of it there are, and
        // -- where it wrote no type -- the type of what it subsets
        let drawn = boxes(
            "part def Wheel;\n\
             part def V {\n\
             \tpart wheels : Wheel[4];\n\
             \tpart spare :> wheels;\n\
             }\n",
            "V",
        );
        let names: Vec<&str> = drawn.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["wheels : Wheel[4]", "spare : Wheel"]);
    }

    #[test]
    fn a_part_carries_what_its_type_inherits() {
        // the type of a part is not on the canvas, and neither is what
        // that type specializes, so a box has to say what it holds all
        // the way up -- and a redefinition says the value while leaving
        // the type to what it redefines
        let drawn = boxes(
            "attribute def Volt;\n\
             part def Base {\n\
             \tattribute forwardVoltage : Volt;\n\
             \tpart inner;\n\
             }\n\
             part def Led :> Base { attribute lit; }\n\
             part def Board {\n\
             \tpart statusLed : Led {\n\
             \t\tattribute :>> forwardVoltage = 2;\n\
             \t}\n\
             }\n",
            "Board",
        );
        assert_eq!(drawn.len(), 1, "{drawn:?}");
        let (name, features) = &drawn[0];
        assert_eq!(name, "statusLed : Led");
        assert_eq!(
            features,
            &["attribute forwardVoltage : Volt = 2", "attribute lit"]
        );
    }

    #[test]
    fn a_definition_is_assembled_from_what_it_inherits_too() {
        // and the connection its supertype declared holds, because both
        // faces of a redeclared part reach the one box drawn for it
        let drawn = boxes(
            "part def Motor;\n\
             part def Big :> Motor;\n\
             part def Chassis;\n\
             part def Base {\n\
             \tpart motor : Motor;\n\
             \tpart chassis : Chassis;\n\
             \tconnect motor to chassis;\n\
             }\n\
             part def Uprated :> Base { part redefines motor : Big; }\n",
            "Uprated",
        );
        let names: Vec<&str> = drawn.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["motor : Big", "chassis : Chassis"]);

        let ws = resolved(
            "part def Motor;\npart def Big :> Motor;\npart def Chassis;\n\
             part def Base {\n\tpart motor : Motor;\n\tpart chassis : Chassis;\n\
             \tconnect motor to chassis;\n}\n\
             part def Uprated :> Base { part redefines motor : Big; }\n",
        );
        let model = ws.model();
        let uprated = model
            .descendants(ws.root())
            .into_iter()
            .find(|&id| model.name(id) == Some("Uprated"))
            .unwrap();
        let diagram = interconnection_diagram(model, uprated);
        assert_eq!(diagram.edges.len(), 1, "{:?}", diagram.edges);
        assert_eq!(diagram.edges[0].relation, Relation::Connection);
    }

    #[test]
    fn a_definition_is_joined_to_what_it_is_made_of() {
        // A port definition nothing is joined to reads as one nothing
        // uses, which is not what the model said. Values and items are
        // held the same way; a `ref` names something the definition
        // does not own, and behaviour is not what it is made of.
        let ws = resolved(
            "attribute def Volt;\n\
             item def Fuel;\n\
             port def Pin;\n\
             part def Wheel;\n\
             part def Driver;\n\
             action def Spin;\n\
             part def Car {\n\
             \tpart w : Wheel;\n\
             \tport p : Pin;\n\
             \tattribute v : Volt;\n\
             \titem f : Fuel;\n\
             \tref part driver : Driver;\n\
             \taction s : Spin;\n\
             }\n\
             calc def Trip { in item load : Fuel; }\n",
        );
        let model = ws.model();
        let diagram = definition_diagram(model, &[ws.root()]);
        let name = |at: usize| diagram.nodes[at].name.as_str();
        let joined = |relation| {
            let mut out: Vec<&str> = diagram
                .edges
                .iter()
                .filter(|edge| edge.relation == relation)
                .map(|edge| name(edge.to))
                .collect();
            out.sort_unstable();
            out
        };
        // an action a definition owns is composite too -- the standard
        // has `Parts::Part::ownedActions` for exactly that
        assert_eq!(
            joined(Relation::Composition),
            ["Fuel", "Pin", "Spin", "Volt", "Wheel"]
        );
        // `ref` says reference outright, and so does a direction: a
        // parameter is not part of what its owner is
        assert_eq!(joined(Relation::Reference), ["Driver", "Fuel"]);
    }

    #[test]
    fn what_a_feature_subsets_or_redefines_is_drawn() {
        // The standard's type relationships are `subclassification |
        // subsetting | definition | redefinition | composite-feature-
        // membership | noncomposite-feature-membership`. Between two
        // definitions the specialization is a subclassification; the
        // same thing between two usages is a subsetting, and a
        // redeclaration of one a redefinition.
        let ws = resolved(
            "part def A;\n\
             part def Big :> A;\n\
             part def Rig {\n\
             \tpart a1 : A;\n\
             \tpart a2 :> a1;\n\
             \tpart a3 redefines a1 : Big;\n\
             }\n",
        );
        let model = ws.model();
        let rig = model
            .descendants(ws.root())
            .into_iter()
            .find(|&id| model.name(id) == Some("Rig"))
            .expect("Rig is declared");
        let diagram = interconnection_diagram(model, rig);
        let mut drawn: Vec<(&str, Relation)> = diagram
            .edges
            .iter()
            .map(|edge| (diagram.nodes[edge.to].name.as_str(), edge.relation))
            .collect();
        drawn.sort_by_key(|(name, relation)| (name.to_string(), format!("{relation:?}")));
        assert_eq!(
            drawn,
            [
                ("a1 : A", Relation::Redefinition),
                ("a1 : A", Relation::Subsetting)
            ]
        );
    }

    #[test]
    fn a_negated_assertion_is_not_drawn_as_one_that_holds() {
        // `not satisfy r by p;` says p does not, and there is no line
        // here for "does not" -- drawing the same one as `satisfy`
        // would put the opposite of the model on the canvas.
        let ws = resolved(
            "part def A;\n\
             requirement def R;\n\
             part def Rig { part a : A; satisfy requirement r : R by a; }\n\
             part def Bad { part a : A; not satisfy requirement r : R by a; }\n",
        );
        let model = ws.model();
        let inside = |name: &str| {
            let owner = model
                .descendants(ws.root())
                .into_iter()
                .find(|&id| model.name(id) == Some(name))
                .expect("declared");
            interconnection_diagram(model, owner)
                .edges
                .iter()
                .filter(|edge| edge.relation == Relation::Satisfy)
                .count()
        };
        assert_eq!(inside("Rig"), 1);
        assert_eq!(inside("Bad"), 0);
    }

    #[test]
    fn a_feature_of_its_own_type_is_drawn_all_the_same() {
        // `part subparts : Assembly;` inside `Assembly` is a membership
        // like any other, and the standard exempts none from being
        // drawn. Leaving it out is a box that looks like nothing uses
        // it, of a definition that uses itself.
        let ws = resolved(
            "part def Assembly {\n\
             \tpart subparts : Assembly[0..*];\n\
             \tref part origin : Assembly;\n\
             }\n",
        );
        let model = ws.model();
        let diagram = definition_diagram(model, &[ws.root()]);
        assert_eq!(diagram.nodes.len(), 1);
        let mut drawn: Vec<Relation> = diagram.edges.iter().map(|edge| edge.relation).collect();
        drawn.sort_by_key(|relation| format!("{relation:?}"));
        assert_eq!(drawn, [Relation::Composition, Relation::Reference]);
        assert!(diagram.edges.iter().all(|edge| edge.from == edge.to));

        // and the drawing takes each of them a different way round, so
        // one does not hide the other
        let svg = crate::render(&diagram, &crate::Style::default());
        let routes: Vec<&str> = svg.matches("<path class=\"edge\"").collect();
        assert_eq!(routes.len(), 2, "{svg}");
        let first = svg.find("d=\"M ").expect("a route");
        let second = svg[first + 1..].find("d=\"M ").expect("a second route");
        assert_ne!(
            &svg[first..first + 40],
            &svg[first + 1 + second..first + 1 + second + 40],
            "the two loops are drawn on top of one another"
        );
    }

    #[test]
    fn a_then_before_a_declaration_still_joins_it() {
        // `then action b;` writes no operand: what it flows into is the
        // declaration it wraps, and what it flows from is whatever
        // stands before it -- the same reading `then b;` gets
        let ws = resolved(
            "action def Step;\n\
             action def Pipe {\n\
             \taction a : Step;\n\
             \tthen action b : Step;\n\
             \tthen action c : Step;\n\
             }\n",
        );
        let model = ws.model();
        let pipe = model
            .descendants(ws.root())
            .into_iter()
            .find(|&id| model.name(id) == Some("Pipe"))
            .expect("Pipe is declared");
        let diagram = interconnection_diagram(model, pipe);
        let names: Vec<&str> = diagram.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["a : Step", "b : Step", "c : Step"]);
        let joined: Vec<(usize, usize)> = diagram
            .edges
            .iter()
            .map(|edge| (edge.from, edge.to))
            .collect();
        assert_eq!(joined, [(0, 1), (1, 2)]);
        assert!(diagram
            .edges
            .iter()
            .all(|edge| edge.relation == Relation::Transition));
    }

    #[test]
    fn a_kerml_model_draws_as_more_than_empty_boxes() {
        // KerML is written out of `step` and `feature`, not the usages
        // SysML layers on them, and reading only usages leaves every
        // box on every page of one saying nothing
        let ws = {
            let mut ws = sysml_semantics::Workspace::new();
            ws.add_file(
                "k.kerml",
                "package K {\n\
                 \tbehavior Focus;\n\
                 \tbehavior T {\n\
                 \t\tstep one : Focus[2];\n\
                 \t\tfeature two = 5;\n\
                 \t\tstep other : Focus;\n\
                 \t\tsuccession one then other;\n\
                 \t}\n\
                 }\n",
            );
            ws.resolve_all();
            ws
        };
        let model = ws.model();
        let behaviour = model
            .descendants(ws.root())
            .into_iter()
            .find(|&id| model.name(id) == Some("T"))
            .expect("T is declared");

        let drawn = definition_diagram(model, &[ws.root()]);
        let t = drawn.nodes.iter().find(|n| n.name == "T").expect("T drawn");
        let labels: Vec<String> = lines(t).map(Feature::label).collect();
        assert!(
            labels.contains(&"step one : Focus[2]".to_string()),
            "{labels:?}"
        );
        assert!(
            labels.contains(&"feature two = 5".to_string()),
            "{labels:?}"
        );

        // and `succession a then b;` says what it joins, rather than
        // taking `a` for a name of its own and joining nothing
        let inside = interconnection_diagram(model, behaviour);
        let names: Vec<&str> = inside.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["one : Focus[2]", "other : Focus"]);
        assert_eq!(inside.edges.len(), 1, "{:?}", inside.edges);
        assert_eq!(inside.edges[0].relation, Relation::Transition);
    }

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
        let labels: Vec<String> = lines(v).map(Feature::label).collect();
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
        // the standard files a line by what it is, so the two
        // attributes are together whatever order they were written in
        let labels: Vec<String> = lines(v).map(Feature::label).collect();
        assert_eq!(
            labels,
            ["attribute a", "attribute s = \"boot\"", "part w : W"]
        );
        let stack: Vec<&str> = v.compartments.iter().map(|c| c.label).collect();
        assert_eq!(stack, ["attributes", "parts"]);
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
            lines(engine).cloned().collect::<Vec<_>>(),
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
        assert_eq!(lines(engine).next().unwrap().label(), "attribute power");
        assert_eq!(
            lines(engine).nth(1).unwrap().label(),
            "port fuelIn : FuelPort"
        );
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
    fn kerml_classifiers_are_drawn_with_their_specializations() {
        let mut ws = sysml_semantics::Workspace::new();
        ws.add_file(
            "model.kerml",
            "package Vehicles {\n\tclassifier Vehicle;\n\tclassifier Car specializes Vehicle;\n\
             \tdatatype Mass;\n\tstruct Chassis;\n\tassoc Owns;\n}\n",
        );
        ws.resolve_all();
        let diagram = crate::definition_diagram(ws.model(), &[ws.root()]);
        let drawn: Vec<(String, String)> = diagram
            .nodes
            .iter()
            .map(|node| (node.keyword.clone(), node.name.clone()))
            .collect();
        assert_eq!(
            drawn,
            [
                ("classifier".to_string(), "Vehicle".to_string()),
                ("classifier".to_string(), "Car".to_string()),
                ("datatype".to_string(), "Mass".to_string()),
                ("struct".to_string(), "Chassis".to_string()),
                ("assoc".to_string(), "Owns".to_string()),
            ]
        );
        assert_eq!(diagram.edges.len(), 1);
        assert_eq!(diagram.edges[0].relation, Relation::Specialization);
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

        // an unnamed usage whose redefinition never resolved either, so
        // it answers to no name at all and never becomes a line
        let anonymous_usage = model.create(ElementKind::PartUsage);
        model.add_owned(named, anonymous_usage);
        let nameless = model.create(ElementKind::Redefinition);
        model.add_owned(anonymous_usage, nameless);

        // a subsetting that never got its `subsettedFeature` set, on a
        // usage an internal view does draw
        let inside = model.create(ElementKind::PartUsage);
        model.set(inside, "declaredName", Value::String("p".to_string()));
        model.add_owned(named, inside);
        let unresolved = model.create(ElementKind::Subsetting);
        model.add_owned(inside, unresolved);
        let internal = interconnection_diagram(&model, named);
        assert_eq!(internal.nodes.len(), 1, "{:?}", internal.nodes);
        assert!(internal.edges.is_empty(), "{:?}", internal.edges);

        let diagram = definition_diagram(&model, &[root]);
        assert_eq!(diagram.nodes.len(), 1);
        assert!(diagram.edges.is_empty());
        let named_lines: Vec<&str> = lines(&diagram.nodes[0])
            .map(|feature| feature.name.as_str())
            .collect();
        assert_eq!(named_lines, ["x", "p"], "only what answers to a name");
        assert_eq!(lines(&diagram.nodes[0]).next().unwrap().ty, None);
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
        assert_eq!(lines(&diagram.nodes[0]).next().unwrap().ty, None);
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
        let drawn: Vec<String> = lines(&diagram.nodes[0]).map(Feature::label).collect();
        // the usage declares nothing of its own, so both come from Wheel
        assert_eq!(drawn, ["port hub", "port rim"]);
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
        let lines: Vec<String> = lines(wheel).map(Feature::label).collect();
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
        let drawn: Vec<String> = lines(&diagram.nodes[0]).map(Feature::label).collect();
        assert_eq!(drawn, ["port hub : Fast"]);
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

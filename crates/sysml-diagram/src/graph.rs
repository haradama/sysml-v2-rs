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
    /// `in`, `out` or `inout`, when the feature declares a direction.
    /// The standard writes it in the line (`directed-features-compartment-element
    /// = el-prefix FeatureDirection DefinitionBodyItem*`) and draws it as
    /// an arrow inside a port's square.
    pub direction: Option<&'static str>,
}

impl Feature {
    /// The compartment line as it appears in the drawing.
    pub fn label(&self) -> String {
        let mut line = String::new();
        if let Some(direction) = self.direction {
            line.push_str(direction);
            line.push(' ');
        }
        if !self.keyword.is_empty() {
            line.push_str(&self.keyword);
            line.push(' ');
        }
        line.push_str(&self.name);
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
    /// The filled circle three or more connection ends meet at
    /// (`n-ary-connection-dot`), named beside it by the connection.
    ConnectionDot,
    /// The bar a flow splits at or comes back together at (`fork-node`,
    /// `join-node`, drawn alike).
    Bar,
    /// The diamond a flow chooses at or comes back together at
    /// (`decision-node`, `merge-node`, drawn alike).
    Diamond,
    /// The cross a flow stops at (`terminate-node`).
    Cross,
}

/// The shape the standard draws an action- or state-flow node with. The
/// control nodes are not boxes: `fork-node` and `join-node` are bars,
/// `decision-node` and `merge-node` diamonds, `terminate-node` a cross.
fn shape_of(kind: ElementKind) -> Shape {
    match kind {
        ElementKind::ForkNode | ElementKind::JoinNode => Shape::Bar,
        ElementKind::MergeNode | ElementKind::DecisionNode => Shape::Diamond,
        ElementKind::TerminateActionUsage => Shape::Cross,
        _ => Shape::Box,
    }
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
    /// The relationships between those children, indexing `children`.
    /// A view holding the parts but not what wires them together is
    /// half of `interconnection-view`, and the half that says less.
    pub links: Vec<Edge>,
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
    let kind = model.kind(member);
    // `end [1] part bead : TireBead;` goes in `ends-compartment`, whatever
    // the feature it declares happens to be
    if model.get(member, "isEnd") == Some(&Value::Bool(true)) {
        return "ends";
    }
    // A directed port is still a port: the standard keeps it in the ports
    // compartment and draws its direction as an arrow in the square on the
    // border, rather than moving it in with the parameters.
    //
    // The standard has two compartments for a directed feature and picks
    // by what owns it: a behaviour's are its `parameters`, and anything
    // else's are `directed features`.
    if model.get(member, "direction").is_some() && !kind.is_a(ElementKind::PortUsage) {
        let behaviour = model
            .owner(member)
            .is_some_and(|owner| model.kind(owner).is_a(ElementKind::Behavior));
        return if behaviour {
            "parameters"
        } else {
            "directed features"
        };
    }
    // `individuals-compartment` and the two portion compartments come
    // from the flags the notation writes, not from a metaclass of their
    // own: `individual`, `snapshot` and `timeslice` are all occurrences.
    if model.get(member, "isIndividual") == Some(&Value::Bool(true)) {
        return "individuals";
    }
    match model.get(member, "portionKind") {
        Some(Value::EnumLit("snapshot")) => return "snapshots",
        Some(Value::EnumLit("timeslice")) => return "timeslices",
        _ => {}
    }
    for (metaclass, label) in [
        // `successions-compartment` is the standard's own; a transition
        // has no compartment there at all, and is only ever the line
        (ElementKind::SuccessionAsUsage, "successions"),
        (ElementKind::PerformActionUsage, "perform actions"),
        (ElementKind::AllocationUsage, "allocations"),
        (ElementKind::InterfaceUsage, "interfaces"),
        (ElementKind::ConnectionUsage, "connections"),
        (ElementKind::FlowUsage, "flows"),
        (ElementKind::ExhibitStateUsage, "exhibit states"),
        (ElementKind::StateUsage, "states"),
        // `calcs-compartment ='calcs'`, not the metaclass spelled out
        (ElementKind::CalculationUsage, "calcs"),
        (ElementKind::AssertConstraintUsage, "assert constraints"),
        (ElementKind::SatisfyRequirementUsage, "satisfy requirements"),
        (ElementKind::IncludeUseCaseUsage, "include use cases"),
        (ElementKind::ConcernUsage, "concerns"),
        // a viewpoint is a requirement in the metamodel and has a
        // compartment of its own in the notation, so it comes first
        (ElementKind::ViewpointUsage, "viewpoints"),
        (ElementKind::RequirementUsage, "requirements"),
        (ElementKind::ConstraintUsage, "constraints"),
        (ElementKind::VerificationCaseUsage, "verifications"),
        (ElementKind::AnalysisCaseUsage, "analyses"),
        (ElementKind::UseCaseUsage, "use cases"),
        (ElementKind::ViewUsage, "views"),
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
    /// A state machine goes from `from` to `to` (`transition off_to_on
    /// first off then on`). The `transition` figure: a plain line with an
    /// open arrowhead, labelled the UML way.
    Transition,
    /// One step follows another (`first a then b`, `succession a then b`).
    /// `aflow-succession` draws it dashed, which is what tells a step
    /// following a step from a state machine changing state.
    Succession,
    /// `from` satisfies the requirement `to` (`satisfy r by p`). Drawn the
    /// SysML way (`satisfy-edge`): a plain line with an open arrowhead,
    /// keyworded `\u{ab}satisfy\u{bb}` and pointing at the requirement.
    Satisfy,
    /// `from` and `to` are bound to the same value (`bind a = b`). A plain
    /// line with `=` written on it (`binding-connection`).
    Binding,
    /// `from` and `to` are joined by an interface (`interface i connect a
    /// to b`). A plain line keyworded `«interface»`.
    Interface,
    /// `from` is allocated to `to` (`allocate a to b`). An open arrowhead
    /// and the keyword `«allocate»` (`allocate-relationship`).
    Allocation,
    /// Something flows from `from` to `to` (`flow f from a.out to b.in`),
    /// drawn with the filled arrowhead the standard gives a flow.
    Flow,
    /// The same, ordered in time (`succession flow`), keyworded
    /// `«succession flow»`.
    SuccessionFlow,
    /// A message between two occurrences (`message m from a to b`), drawn
    /// with the open arrowhead the standard reserves for it.
    Message,
    /// `from` asserts the constraint `to` (`assert constraint c`), drawn
    /// with the open arrowhead and `«assert»` (`assert-edge`).
    Assert,
    /// `from` assumes the constraint `to` (`assume constraint c`),
    /// keyworded `«assume»` (`assume-edge`).
    Assume,
    /// `from` requires the constraint or requirement `to` (`require
    /// constraint c`), keyworded `«require»` (`require-edge`).
    Require,
    /// `from` performs the action `to` (`perform a`), the `perform-edge`.
    Perform,
    /// `from` exhibits the state `to` (`exhibit s`), the `exhibit-edge`.
    Exhibit,
    /// `from` depends on `to` (`dependency use from A to B`). The one
    /// dashed line in the notation (`binary-dependency`), with an open
    /// arrowhead and the dependency's own name on it.
    Dependency,
    /// `to` is a portion of `from` (`snapshot s : O`, `timeslice t : O`).
    /// The `portion-relationship`: a plain line with the filled marker at
    /// the whole, the way a composition carries its diamond.
    Portion,
    /// `from` is an event of `to` (`event occurrence ev;`), keyworded
    /// `\u{ab}event\u{bb}` (`event-edge`).
    Event,
}

/// A relationship between two boxes. Both index fields index
/// [`Diagram::nodes`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub relation: Relation,
    /// The `rolename` the standard writes at each end: the feature the
    /// line attaches to (`hub`, `mount`), which is what tells two
    /// connections between the same pair apart. `None` where there is
    /// nothing to name -- a transition, or an end that is the box itself,
    /// as in `allocate tank to eng`.
    pub ends: (Option<String>, Option<String>),
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
                links: Vec::new(),
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
                    ends: (None, None),
                    label: None,
                });
            }
        }
        compositions_of(model, node.id, from, &index, &mut edges);
        annotations_of(model, node.id, from, &index, &mut edges);
        // `connection def D { end a : A; end b : B; }` relates the
        // definitions its ends are typed by, which is the only thing
        // holding them together in a definition diagram
        for end in connector_ends(model, node.id) {
            let Some(&to) = index.get(&end.target) else {
                continue;
            };
            if from != to {
                edges.push(Edge {
                    from,
                    to,
                    relation: Relation::Connection,
                    ends: (None, None),
                    label: Some(format!("{}{}", end.role, end.adornment)),
                });
            }
        }
    }
    // most dependencies are written in a package rather than inside a
    // definition, and a package is not one of the boxes
    for &root in roots {
        for id in model.descendants(root) {
            dependencies_of(model, id, &index, &mut edges);
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
        let links = links_between(model, child, &children);
        let mut features = features_with_type(model, child);
        // whatever became a box inside is not also a compartment line
        let nested: HashSet<&str> = children
            .iter()
            .filter_map(|child| model.name(child.id))
            .collect();
        features.retain(|(_, feature)| !nested.contains(feature.name.as_str()));
        index.insert(child, nodes.len());
        nodes.push(Node {
            id: child,
            name: label,
            keyword: box_keyword(model, child),
            compartments: into_compartments(features),
            is_abstract: is_abstract(model, child),
            rounded: model.kind(child).is_a(ElementKind::Usage),
            shape: shape_of(model.kind(child)),
            children,
            links,
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
                if let Some(&to) = index.get(&only.target) {
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
                            links: Vec::new(),
                        });
                    }
                    if from != to {
                        edges.push(Edge {
                            from,
                            to,
                            relation: Relation::Succession,
                            ends: (None, None),
                            label: None,
                        });
                    }
                    // the flow now stands where it just arrived
                    previous = Some(to);
                }
            }
            continue;
        }
        let relation = connector_relation(model, child);
        let directed = matches!(relation, Relation::Transition | Relation::Succession);
        // `n-ary-connection = n-ary-connection-dot n-ary-segment+`: three
        // or more ends meet at a dot, with one segment running out to
        // each. Fanning them out from whichever end was written first
        // would say that end relates the others, which is not what an
        // n-ary connection means.
        if ends.len() > 2 && !directed {
            push_n_ary(
                model, child, &ends, &index, relation, &mut nodes, &mut edges,
            );
            continue;
        }
        let Some((first, rest)) = ends.split_first() else {
            continue;
        };
        let Some(&from) = index.get(&first.target) else {
            continue;
        };
        for second in rest {
            let Some(&to) = index.get(&second.target) else {
                continue;
            };
            if from == to {
                continue;
            }
            edges.push(Edge {
                from,
                to,
                relation,
                // a transition names its states twice over; only a
                // connection's port labels add anything
                ends: if directed {
                    (None, None)
                } else {
                    (rolename(model, first), rolename(model, second))
                },
                // `off_to_on / send action`, after the UML convention of
                // naming the step and then what it does
                label: connector_label(model, child, relation),
            });
        }
    }

    Diagram { nodes, edges }
}

/// Which line the standard draws for a two-ended statement.
///
/// The specification gives each its own notation -- clauses 8.2.3.13 to
/// 8.2.3.16 -- and drawing them all as one plain line loses what the
/// source said: that `a` is *bound* to `b`, or *allocated* to it, rather
/// than merely wired to it.
fn connector_relation(model: &Model, connector: ElementId) -> Relation {
    match model.kind(connector) {
        ElementKind::BindingConnector | ElementKind::BindingConnectorAsUsage => Relation::Binding,
        ElementKind::InterfaceUsage => Relation::Interface,
        ElementKind::AllocationUsage => Relation::Allocation,
        ElementKind::SuccessionFlowUsage => Relation::SuccessionFlow,
        // A message has no metaclass of its own: `Message : FlowUsage =
        // ... { isAbstract = true }` is how the standard writes one, so
        // the flag is what tells it from a flow.
        ElementKind::FlowUsage => match model.get(connector, "isAbstract") {
            Some(&Value::Bool(true)) => Relation::Message,
            _ => Relation::Flow,
        },
        ElementKind::SuccessionAsUsage => Relation::Succession,
        kind if kind.is_a(ElementKind::TransitionUsage) => Relation::Transition,
        _ => Relation::Connection,
    }
}

/// Draw an n-ary connection the way the standard does: a dot carrying the
/// connection's own name, and a segment out to each end it names.
///
/// A dot with fewer than two segments joins nothing, so it is only drawn
/// once the ends that landed in the diagram are known.
fn push_n_ary(
    model: &Model,
    connector: ElementId,
    ends: &[End],
    index: &HashMap<ElementId, usize>,
    relation: Relation,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let landed: Vec<&End> = ends
        .iter()
        .filter(|end| index.contains_key(&end.target))
        .collect();
    if landed.len() < 2 {
        return;
    }
    let dot = nodes.len();
    nodes.push(Node {
        id: connector,
        // `cdot-label`, read beside the dot
        name: model.name(connector).unwrap_or_default().to_string(),
        keyword: String::new(),
        compartments: Vec::new(),
        is_abstract: false,
        rounded: false,
        shape: Shape::ConnectionDot,
        children: Vec::new(),
        links: Vec::new(),
    });
    for end in landed {
        edges.push(Edge {
            from: dot,
            to: index[&end.target],
            relation,
            ends: (None, rolename(model, end)),
            label: None,
        });
    }
}

/// The `rolename` to write at one end of a line: the feature it attaches
/// to, and nothing at all when that feature is the box itself. `allocate
/// tank to eng` attaches to the whole of `tank`, and a square labelled
/// `tank` on the box already labelled `tank` says nothing twice.
fn rolename(model: &Model, end: &End) -> Option<String> {
    let named = !end.role.is_empty() && effective_name(model, end.target) != Some(&end.role);
    (named || !end.adornment.is_empty())
        .then(|| format!("{}{}", end.role, end.adornment).trim().to_string())
}

/// What the standard writes on a two-ended line.
///
/// Every one of these comes from the production that draws it --
/// `binding-connection` writes `=`, `allocate-relationship` writes
/// `«allocate»`, a flow writes what it carries -- and without them one
/// plain line would stand for six different statements.
fn connector_label(model: &Model, child: ElementId, relation: Relation) -> Option<String> {
    let written = match relation {
        Relation::Transition => return transition_label(model, child),
        Relation::Binding => "=".to_string(),
        Relation::Interface => keyworded("interface", model.name(child)),
        Relation::Allocation => keyworded("allocate", model.name(child)),
        Relation::SuccessionFlow => {
            let carried = carries(model, child);
            keyworded("succession flow", carried.as_deref())
        }
        // a flow and a message write what they carry and nothing else
        Relation::Flow | Relation::Message => carries(model, child)?,
        _ => return None,
    };
    Some(written)
}

/// `«keyword»`, and the name after it when the statement was given one.
fn keyworded(keyword: &str, name: Option<&str>) -> String {
    match name {
        Some(name) => format!("\u{ab}{keyword}\u{bb} {name}"),
        None => format!("\u{ab}{keyword}\u{bb}"),
    }
}

/// What a flow carries, as `flow-label` writes it: `UsageDeclaration? ('of'
/// FlowPayloadFeatureMember)?`. A flow whose payload went unread is one
/// that appears to carry nothing.
fn carries(model: &Model, flow: ElementId) -> Option<String> {
    let payload = model
        .owned(flow)
        .iter()
        .find(|&&owned| model.kind(owned) == ElementKind::PayloadFeature)
        .and_then(|&owned| payload_label(model, owned));
    match (model.name(flow), payload) {
        (Some(name), Some(payload)) => Some(format!("{name} of {payload}")),
        (Some(name), None) => Some(name.to_string()),
        (None, Some(payload)) => Some(format!("of {payload}")),
        (None, None) => None,
    }
}

/// The `FlowPayloadFeatureMember` as the notation writes it: `Fuel`,
/// `Fuel[2]`, or `fuelCommand : FuelCommand` where the payload was
/// declared and named.
fn payload_label(model: &Model, payload: ElementId) -> Option<String> {
    let mut written = match (effective_name(model, payload), type_name(model, payload)) {
        (Some(name), Some(ty)) => format!("{name} : {ty}"),
        (Some(name), None) => name.to_string(),
        (None, Some(ty)) => ty,
        (None, None) => return None,
    };
    if let Some(range) = multiplicity_of(model, payload) {
        written.push_str(&range);
    }
    Some(written)
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
            ends: (None, None),
            label: Some(keyworded("satisfy", None)),
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
    // `end [1] part bead : TireBead` writes the multiplicity between the
    // keyword and the feature, which makes the `end` an anonymous wrapper
    // around the feature it declares: what the end reaches is a level in,
    // while the multiplicity and the other adornments stay out here.
    std::iter::once(end)
        .chain(model.owned(end).iter().copied())
        .find_map(|feature| {
            let target = model.type_of(feature)?;
            Some((target, model.name(feature).unwrap_or_default().to_string()))
        })
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
fn connector_ends(model: &Model, connector: ElementId) -> Vec<End> {
    // `require Load;` inside a requirement owns a reference of its own,
    // and it is not an end. What makes a member one is the `end` keyword
    // -- or that the thing owning it relates things for a living, which
    // is how `connect w.hub to a.mount` writes its two without one.
    let relates = model.kind(connector).is_a(ElementKind::Connector)
        || model.kind(connector).is_a(ElementKind::TransitionUsage);
    model
        .owned(connector)
        .iter()
        .filter(|&&end| relates || model.get(end, "isEnd") == Some(&Value::Bool(true)))
        .filter_map(|&end| {
            let (target, role) = chained_end(model, end)
                .or_else(|| referenced_end(model, end))
                .or_else(|| typed_end(model, end))?;
            Some(End {
                target,
                role,
                adornment: end_adornment(model, end),
            })
        })
        .collect()
}

/// One end of a two-ended statement: what it reaches, the `rolename` that
/// names it there, and what the standard writes after that name.
struct End {
    target: ElementId,
    role: String,
    adornment: String,
}

/// The `multiplicity` and `c-adornment` the standard writes at an end,
/// after its rolename: `c-adornment = (a-property | a-direction |
/// a-subsetting | a-redefinition)*`.
///
/// (`readonly` is in that list but in neither grammar nor metamodel, so
/// there is no way to write one and nothing to draw.)
fn end_adornment(model: &Model, end: ElementId) -> String {
    let mut out = String::new();
    if let Some(range) = multiplicity_of(model, end) {
        out.push_str(&format!(" {range}"));
    }
    if let Some(direction) = direction_of(model, end) {
        out.push_str(&format!(" {direction}"));
    }
    for (flag, written) in [
        ("isOrdered", "ordered"),
        ("isAbstract", "abstract"),
        ("isDerived", "derived"),
    ] {
        if model.get(end, flag) == Some(&Value::Bool(true)) {
            out.push_str(&format!(" {written}"));
        }
    }
    if model.get(end, "isUnique") == Some(&Value::Bool(false)) {
        out.push_str(" nonunique");
    }
    // exact kinds, so the reference subsetting that gives the end its
    // rolename (`end ::> w.hub`) is not written out a second time
    let refinements = model.owned(end).iter().filter_map(|&rel| {
        let (written, property) = match model.kind(rel) {
            ElementKind::Subsetting => ("subsets", "subsettedFeature"),
            ElementKind::Redefinition => ("redefines", "redefinedFeature"),
            _ => return None,
        };
        let name = model.name(model.get(rel, property)?.as_id()?)?;
        Some(format!(" {written} {name}"))
    });
    out.extend(refinements);
    out
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

/// One edge per client-supplier pair of every `dependency` in scope.
///
/// `Dependency = 'dependency' ( Identification? 'from' )? client += ...
/// 'to' supplier += ...`, and the standard draws `binary-dependency`
/// between each pair. A dependency naming several of either is written
/// out pairwise, which says the same thing as its n-ary figure.
fn dependencies_of(
    model: &Model,
    scope: ElementId,
    index: &HashMap<ElementId, usize>,
    edges: &mut Vec<Edge>,
) {
    for &child in model.owned(scope) {
        if model.kind(child) != ElementKind::Dependency {
            continue;
        }
        let ends = |property| match model.get(child, property) {
            // a dependency whose names went unresolved has neither list
            Some(Value::RefList(ends)) => ends.clone(),
            _ => Vec::new(),
        };
        for client in ends("client") {
            for supplier in ends("supplier") {
                let (Some(&from), Some(&to)) = (index.get(&client), index.get(&supplier)) else {
                    continue;
                };
                if from != to {
                    edges.push(Edge {
                        from,
                        to,
                        relation: Relation::Dependency,
                        ends: (None, None),
                        label: model.name(child).map(str::to_string),
                    });
                }
            }
        }
    }
}

/// One edge per thing a definition asserts, assumes, requires, performs
/// or exhibits.
///
/// The standard draws each of these as a line of its own -- `assert-edge`,
/// `assume-edge`, `require-edge`, `perform-edge`, `exhibit-edge` -- and
/// listing them only in a compartment says the definition mentions them,
/// not that it answers for them.
fn annotations_of(
    model: &Model,
    definition: ElementId,
    from: usize,
    index: &HashMap<ElementId, usize>,
    edges: &mut Vec<Edge>,
) {
    let mut linked: Vec<(usize, Relation)> = Vec::new();
    for &child in model.owned(definition) {
        let Some(relation) = annotation_relation(model, child) else {
            continue;
        };
        // `assert constraint c;` references the constraint rather than
        // declaring a typing of its own, so what it is about is a
        // reference or two away
        let Some(&to) = resolved_type(model, child).and_then(|ty| index.get(&ty)) else {
            continue;
        };
        // one line per pair: `assert constraint a; assert constraint b;`
        // over two constraints of one definition says the same thing twice
        if from == to || linked.contains(&(to, relation)) {
            continue;
        }
        linked.push((to, relation));
        edges.push(Edge {
            from,
            to,
            relation,
            ends: (None, None),
            label: Some(keyworded(annotation_keyword(relation), None)),
        });
    }
}

/// Which of the keyworded lines a member is, if it is one of them.
fn annotation_relation(model: &Model, member: ElementId) -> Option<Relation> {
    // the role a membership was given comes first: `assume constraint c`
    // and `require constraint c` are both plain constraint usages, and
    // only the role says which
    match model.member_role(member) {
        Some(Role::Assume) => return Some(Relation::Assume),
        Some(Role::Require) => return Some(Relation::Require),
        _ => {}
    }
    match model.kind(member) {
        ElementKind::AssertConstraintUsage => Some(Relation::Assert),
        ElementKind::PerformActionUsage => Some(Relation::Perform),
        ElementKind::ExhibitStateUsage => Some(Relation::Exhibit),
        ElementKind::EventOccurrenceUsage => Some(Relation::Event),
        _ => None,
    }
}

/// The keyword the standard writes on each of those lines.
fn annotation_keyword(relation: Relation) -> &'static str {
    match relation {
        Relation::Event => "event",
        Relation::Assume => "assume",
        Relation::Require => "require",
        Relation::Perform => "perform",
        Relation::Exhibit => "exhibit",
        _ => "assert",
    }
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
        // A portion is composite too, and the standard draws it
        // differently: `portion-relationship` carries its own marker,
        // because a timeslice is part of an occurrence in a way a wheel
        // is not part of a car.
        let relation = match model.get(child, "isPortion") {
            Some(&Value::Bool(true)) => Relation::Portion,
            _ => match model.get(child, "isComposite") {
                Some(&Value::Bool(true)) => Relation::Composition,
                Some(&Value::Bool(false)) => Relation::Reference,
                _ => continue,
            },
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
            ends: (None, None),
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
                ends: (None, None),
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

/// The keyword a box writes above its name, with the role the membership
/// gave it: `entry-action-name-comp = '\u{ab}' 'entry' OccurrenceUsagePrefix
/// 'action' '\u{bb}'`, so a state's entry action says which of the three it
/// is rather than reading as any other action.
fn box_keyword(model: &Model, element: ElementId) -> String {
    let written = keyword(model.kind(element));
    match model.member_role(element) {
        Some(Role::Entry) => format!("entry {written}"),
        Some(Role::Do) => format!("do {written}"),
        Some(Role::Exit) => format!("exit {written}"),
        _ => written,
    }
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
    // And so does SysML: the name compartment the standard writes is not
    // always the metaclass name spelled out. `«analysis def»`, not
    // `«analysis case def»`; `«loop»` for both loops, whichever kind.
    let written = match kind {
        ElementKind::AnalysisCaseDefinition => Some("analysis def"),
        ElementKind::AnalysisCaseUsage => Some("analysis"),
        ElementKind::VerificationCaseDefinition => Some("verification def"),
        ElementKind::VerificationCaseUsage => Some("verification"),
        ElementKind::CalculationDefinition => Some("calc def"),
        ElementKind::CalculationUsage => Some("calc"),
        ElementKind::EnumerationDefinition => Some("enum def"),
        ElementKind::EnumerationUsage => Some("enum"),
        ElementKind::AssignmentActionUsage => Some("assign"),
        ElementKind::IfActionUsage => Some("if"),
        ElementKind::WhileLoopActionUsage | ElementKind::ForLoopActionUsage => Some("loop"),
        // the metaclass is `SuccessionAsUsage`; the notation writes it
        // `succession a then b`
        ElementKind::SuccessionAsUsage => Some("succession"),
        _ => None,
    };
    if let Some(written) = written {
        return written.to_string();
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

/// What wires the nested parts of one usage together: the standard's
/// `interconnection-view` holds `(interconnection-element)*`, and a view
/// with the parts but nothing between them is the half that says less.
///
/// Read from the same owners `nested_parts` draws from, so a connection a
/// supertype declared joins the parts a subtype inherited.
fn links_between(model: &Model, usage: ElementId, children: &[Node]) -> Vec<Edge> {
    let at: HashMap<ElementId, usize> = children
        .iter()
        .enumerate()
        .map(|(index, child)| (child.id, index))
        .collect();
    let mut links = Vec::new();
    for owner in nesting_owners(model, usage) {
        for &child in model.owned(owner) {
            let ends = connector_ends(model, child);
            let [first, second] = &ends[..] else { continue };
            let (Some(&from), Some(&to)) = (at.get(&first.target), at.get(&second.target)) else {
                continue;
            };
            if from == to {
                continue;
            }
            let relation = connector_relation(model, child);
            let directed = matches!(relation, Relation::Transition | Relation::Succession);
            links.push(Edge {
                from,
                to,
                relation,
                ends: if directed {
                    (None, None)
                } else {
                    (rolename(model, first), rolename(model, second))
                },
                label: connector_label(model, child, relation),
            });
        }
    }
    links
}

/// Where a nested view reads its members from: the usage itself, and then
/// the type it was declared with, since `part w : Wheel;` declares nothing
/// of its own.
fn nesting_owners(model: &Model, usage: ElementId) -> Vec<ElementId> {
    std::iter::once(usage)
        .chain(
            model
                .type_of(usage)
                .into_iter()
                .flat_map(|ty| itself_and_supertypes(model, ty)),
        )
        .collect()
}

/// The parts a box is assembled from, as boxes to draw inside it.
///
/// Taken from the usage and then its type, the same way its features are:
/// `part w : Wheel;` declares nothing itself, so the sub-parts come from
/// `Wheel`. Nesting stops here -- one level is what a box has room for.
fn nested_parts(model: &Model, usage: ElementId) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::new();
    for owner in nesting_owners(model, usage) {
        for &part in model.owned(owner) {
            // an action's steps and a state's states nest the same way a
            // part's parts do: `action-flow-compartment` and
            // `state-transition-compartment` hold the view, and drawing
            // only the parts leaves a behaviour as an empty frame
            if !is_structure_box(model.kind(part)) {
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
                keyword: box_keyword(model, part),
                compartments: Vec::new(),
                is_abstract: is_abstract(model, part),
                rounded: model.kind(part).is_a(ElementKind::Usage),
                shape: shape_of(model.kind(part)),
                children: Vec::new(),
                links: Vec::new(),
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
        // A view says what it exposes and what it filters by, and
        // neither is a feature: `exposes-compartment-element =
        // MembershipExpose | NamespaceExpose` and
        // `filters-compartment-element = el-prefix? MemberPrefix
        // OwnedExpression`.
        if let Some(line) = shown_relationship(model, child) {
            out.push(line);
            continue;
        }
        // A transition is a line and nothing else: the states clause has
        // `state-transition-compartment` for the view and no compartment
        // to list one in, and listing it says the state owns an action by
        // that name.
        if !model.kind(child).is_a(ElementKind::Feature)
            || model.kind(child).is_a(ElementKind::TransitionUsage)
        {
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
                direction: direction_of(model, child),
            },
        ));
    }
    out
}

/// A member the standard lists that is a relationship rather than a
/// feature: what a view exposes, and the expression a package filters by.
fn shown_relationship(model: &Model, member: ElementId) -> Option<(&'static str, Feature)> {
    let (compartment, keyword) = match model.kind(member) {
        ElementKind::MembershipExpose | ElementKind::NamespaceExpose => ("exposes", "expose"),
        ElementKind::ElementFilterMembership => ("filters", "filter"),
        // `documentation-compartment` holds what an element documents
        // about itself, and `\u{ab}rep\u{bb}` what it says in another
        // language. Both were built and neither was ever drawn.
        // the compartment is already labelled `doc`, so the prose stands
        // on its own; a `rep` says which language it is in
        ElementKind::Documentation => ("doc", ""),
        ElementKind::TextualRepresentation => ("doc", "rep"),
        _ => return None,
    };
    // none of these names an element: what each says is the text it was
    // written as -- the name exposed, the expression filtered by, or the
    // prose itself
    let named = model
        .get(member, "body")
        .and_then(Value::as_str)
        .map(one_line)
        .or_else(|| written_text(model, member))?;
    Some((
        compartment,
        Feature {
            keyword: keyword.to_string(),
            name: named,
            ty: None,
            multiplicity: None,
            value: None,
            direction: None,
        },
    ))
}

/// A block of prose as one compartment line. The standard writes `…`
/// where a compartment holds more than it shows, and a paragraph on one
/// line would set the width of the box it is in.
fn one_line(text: &str) -> String {
    const ROOM: usize = 60;
    let first = text.lines().next().unwrap_or_default().trim();
    let short = first.char_indices().nth(ROOM).map(|(at, _)| at);
    match (short, first.len() < text.trim().len()) {
        (Some(at), _) => format!("{}\u{2026}", &first[..at]),
        (None, true) => format!("{first}\u{2026}"),
        (None, false) => first.to_string(),
    }
}

/// The text an element was written as, where the build kept one -- on the
/// element itself, or on the expression it owns.
fn written_text(model: &Model, element: ElementId) -> Option<String> {
    std::iter::once(element)
        .chain(model.owned(element).iter().copied())
        .find_map(|holder| {
            model
                .owned(holder)
                .iter()
                .filter(|&&rep| model.kind(rep) == ElementKind::TextualRepresentation)
                .find_map(|&rep| model.get(rep, "body")?.as_str().map(str::to_string))
        })
}

/// Was the element declared `abstract`? A definition that is has no
/// instances of its own, which the drawing is expected to say.
fn is_abstract(model: &Model, element: ElementId) -> bool {
    model.get(element, "isAbstract") == Some(&Value::Bool(true))
}

/// The direction a feature was declared with, when it declares one.
fn direction_of(model: &Model, feature: ElementId) -> Option<&'static str> {
    match model.get(feature, "direction") {
        Some(Value::EnumLit(direction)) => Some(direction),
        _ => None,
    }
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
    model.name(resolved_type(model, usage)?).map(str::to_string)
}

/// What a feature is typed by, following what it subsets, redefines or
/// references until a type turns up.
///
/// A declared typing answers straight away. Otherwise `part big :> engine`
/// is one of whatever `engine` is, and `flow f of carried :> Fuel` reaches
/// a definition rather than another feature -- subsetting a definition is
/// how KerML says a feature is one of those, so that definition is the
/// type and not another feature to follow.
fn resolved_type(model: &Model, usage: ElementId) -> Option<ElementId> {
    let mut visited = HashSet::new();
    let mut queue = std::collections::VecDeque::from([usage]);
    while let Some(current) = queue.pop_front() {
        if !visited.insert(current) {
            continue;
        }
        if let Some(target) = model.type_of(current) {
            return Some(target);
        }
        if current != usage && !model.kind(current).is_a(ElementKind::Feature) {
            return Some(current);
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
        // `then` is a succession, which the standard draws dashed
        assert!(diagram
            .edges
            .iter()
            .all(|edge| edge.relation == Relation::Succession));
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
        assert_eq!(inside.edges[0].relation, Relation::Succession);
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
                    direction: None,
                },
                Feature {
                    keyword: "port".to_string(),
                    name: "fuelIn".to_string(),
                    ty: Some("FuelPort".to_string()),
                    multiplicity: None,
                    value: None,
                    direction: None,
                },
            ]
        );
        assert_eq!(lines(engine).next().unwrap().label(), "attribute power");
        assert_eq!(
            lines(engine).nth(1).unwrap().label(),
            "port fuelIn : FuelPort"
        );
    }

    #[test]
    fn a_declared_direction_is_written_in_the_line_it_belongs_to() {
        let ws = resolved(
            "part def Fuel;\n\
             port def FuelPort;\n\
             part def Engine {\n\
             	in item supply : Fuel;\n\
             	out item spent : Fuel;\n\
             	inout port fuelIn : FuelPort;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let engine = diagram.nodes.iter().find(|n| n.name == "Engine").unwrap();

        // the direction reads at the front of the line, the way the
        // notation writes it
        let written: Vec<String> = lines(engine).map(Feature::label).collect();
        assert_eq!(
            written,
            [
                "in item supply : Fuel",
                "out item spent : Fuel",
                "inout port fuelIn : FuelPort",
            ]
        );

        // a part's directed features are `directed features`; only a
        // behaviour's are its `parameters`, which is how the standard
        // picks between its two compartments for them
        let where_each: Vec<(&str, &str)> = engine
            .compartments
            .iter()
            .flat_map(|compartment| {
                compartment
                    .lines
                    .iter()
                    .map(move |line| (compartment.label, line.name.as_str()))
            })
            .collect();
        assert_eq!(
            where_each,
            [
                ("directed features", "supply"),
                ("directed features", "spent"),
                // and a directed port stays a port either way, because it
                // is still drawn on the border
                ("ports", "fuelIn"),
            ]
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
        assert_eq!(keyword(ElementKind::UseCaseDefinition), "use case def");
        // neither suffix: the metaclass name itself, split into words
        assert_eq!(keyword(ElementKind::Subclassification), "subclassification");
    }

    #[test]
    fn a_name_compartment_is_written_the_way_the_standard_writes_it() {
        // where the standard's name compartment is not the metaclass name
        // spelled out, it is the standard that decides: `«analysis def»`,
        // not `«analysis case def»`
        for (kind, written) in [
            (ElementKind::AnalysisCaseDefinition, "analysis def"),
            (ElementKind::AnalysisCaseUsage, "analysis"),
            (ElementKind::VerificationCaseDefinition, "verification def"),
            (ElementKind::VerificationCaseUsage, "verification"),
            (ElementKind::CalculationDefinition, "calc def"),
            (ElementKind::CalculationUsage, "calc"),
            (ElementKind::EnumerationDefinition, "enum def"),
            (ElementKind::EnumerationUsage, "enum"),
            (ElementKind::AssignmentActionUsage, "assign"),
            (ElementKind::IfActionUsage, "if"),
            (ElementKind::WhileLoopActionUsage, "loop"),
            (ElementKind::ForLoopActionUsage, "loop"),
            // and where it is, the metaclass name still answers
            (ElementKind::SendActionUsage, "send action"),
            (ElementKind::AcceptActionUsage, "accept action"),
            (ElementKind::EventOccurrenceUsage, "event occurrence"),
        ] {
            assert_eq!(keyword(kind), written, "{kind:?}");
        }
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
    fn what_a_definition_answers_for_is_a_line_of_its_own() {
        // `assert-edge`, `assume-edge`, `require-edge`, `perform-edge`
        // and `exhibit-edge`: a compartment line says the definition
        // mentions these, not that it answers for them
        let ws = resolved(
            "constraint def Safe { true }\n\
             action def Warm;\n\
             state def Idle;\n\
             part def Oven {\n\
             \tconstraint c : Safe;\n\
             \tassert c;\n\
             \taction w : Warm;\n\
             \tperform w;\n\
             \tstate s : Idle;\n\
             \texhibit s;\n\
             \tassume constraint a : Safe;\n\
             \trequire constraint q : Safe;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let mut keyworded: Vec<(Relation, &str)> = diagram
            .edges
            .iter()
            .filter_map(|edge| Some((edge.relation, edge.label.as_deref()?)))
            .collect();
        keyworded.sort_by_key(|(_, label)| *label);
        assert_eq!(
            keyworded,
            [
                (Relation::Assert, "\u{ab}assert\u{bb}"),
                (Relation::Assume, "\u{ab}assume\u{bb}"),
                (Relation::Exhibit, "\u{ab}exhibit\u{bb}"),
                (Relation::Perform, "\u{ab}perform\u{bb}"),
                (Relation::Require, "\u{ab}require\u{bb}"),
            ]
        );
    }

    #[test]
    fn a_dependency_is_drawn_from_each_client_to_each_supplier() {
        // `Dependency = 'dependency' ( Identification? 'from' )? client
        // += ... 'to' supplier += ...`, drawn as `binary-dependency` --
        // and until now not drawn, or even built, at all
        let ws = resolved(
            "package P {\n\
             \tpart def A;\n\
             \tpart def B;\n\
             \tpart def Z;\n\
             \tdependency Use from A to B;\n\
             \tdependency Z to A, B;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let names = |at: usize| diagram.nodes[at].name.clone();
        let drawn: Vec<(String, String, Option<&str>)> = diagram
            .edges
            .iter()
            .filter(|edge| edge.relation == Relation::Dependency)
            .map(|edge| (names(edge.from), names(edge.to), edge.label.as_deref()))
            .collect();
        assert_eq!(
            drawn,
            [
                // the name before `from` is the dependency's own
                ("A".to_string(), "B".to_string(), Some("Use")),
                // and without `from`, the first name is a client
                ("Z".to_string(), "A".to_string(), None),
                ("Z".to_string(), "B".to_string(), None),
            ]
        );
    }

    #[test]
    fn what_a_definition_documents_about_itself_is_drawn() {
        // `documentation-compartment` holds the prose, and
        // `textual-representation-node` the language and what it says
        let ws = resolved(
            "part def Thing {\n\
             \tdoc /* A thing.\n\
             \t * And more about it than one line holds.\n\
             \t */\n\
             \trep asJson language \"json\" /* {} */\n\
             \tattribute a;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        assert_eq!(
            diagram.nodes[0]
                .compartments
                .iter()
                .flat_map(|compartment| {
                    compartment
                        .lines
                        .iter()
                        .map(move |line| (compartment.label, line.label()))
                })
                .collect::<Vec<_>>(),
            [
                // the compartment is labelled `doc`, so the prose stands
                // on its own, and the standard's own `\u{2026}` says there
                // is more of it than the line holds
                ("doc", "A thing.\u{2026}".to_string()),
                ("doc", "rep {}".to_string()),
                ("attributes", "attribute a".to_string()),
            ]
        );
    }

    #[test]
    fn a_long_line_of_prose_is_cut_where_the_standard_cuts_one() {
        assert_eq!(one_line("short"), "short");
        assert_eq!(one_line("first\nsecond"), "first\u{2026}");
        let long = "x".repeat(80);
        assert_eq!(one_line(&long).chars().count(), 61);
        assert!(one_line(&long).ends_with('\u{2026}'));
    }

    #[test]
    fn a_view_says_what_it_exposes_and_what_it_filters_by() {
        // `exposes-compartment`, `filters-compartment`,
        // `viewpoints-compartment` and `rendering-compartment`: neither an
        // expose nor a filter is a feature, and a view listing neither
        // says only that it exposes and filters something
        let ws = resolved(
            "package P {\n\
             \tpart def Thing;\n\
             \tviewpoint def Concerned;\n\
             \trendering def Tree;\n\
             \tview def Overview {\n\
             \t\tviewpoint c : Concerned;\n\
             \t\texpose P::Thing;\n\
             \t\tfilter @Safety;\n\
             \t\trendering asTree : Tree;\n\
             \t}\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let overview = diagram.nodes.iter().find(|n| n.name == "Overview").unwrap();
        assert_eq!(
            overview
                .compartments
                .iter()
                .flat_map(|compartment| {
                    compartment
                        .lines
                        .iter()
                        .map(move |line| (compartment.label, line.label()))
                })
                .collect::<Vec<_>>(),
            [
                // a viewpoint is a requirement in the metamodel, and has
                // a compartment of its own in the notation
                ("viewpoints", "viewpoint c : Concerned".to_string()),
                ("exposes", "expose P::Thing".to_string()),
                ("filters", "filter @Safety".to_string()),
                ("rendering", "rendering asTree : Tree".to_string()),
            ]
        );
    }

    #[test]
    fn a_portion_is_not_drawn_as_an_ordinary_composition() {
        // `portion-relationship` carries a marker of its own: a timeslice
        // is part of an occurrence in a way a wheel is not part of a car,
        // and an `\u{ab}event\u{bb}` line says what an event is an event of
        let ws = resolved(
            "occurrence def O;\n\
             part def P {\n\
             \toccurrence o : O;\n\
             \tsnapshot s : O;\n\
             \tevent occurrence ev : O;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let mut drawn: Vec<Relation> = diagram.edges.iter().map(|edge| edge.relation).collect();
        drawn.sort_by_key(|relation| format!("{relation:?}"));
        assert_eq!(
            drawn,
            [Relation::Composition, Relation::Event, Relation::Portion]
        );
        let event = diagram
            .edges
            .iter()
            .find(|edge| edge.relation == Relation::Event)
            .unwrap();
        assert_eq!(event.label.as_deref(), Some("\u{ab}event\u{bb}"));
    }

    #[test]
    fn a_dependency_that_names_nothing_draws_nothing() {
        let ws = resolved(
            "package P {\n\
             \tpart def A;\n\
             \tdependency A to NotThere;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        assert!(diagram.edges.is_empty());
    }

    #[test]
    fn one_line_per_pair_however_often_the_model_says_it() {
        // two constraints of one definition, asserted separately, are
        // one `\u{ab}assert\u{bb}` between the same two boxes
        let ws = resolved(
            "constraint def Safe { true }\n\
             part def Oven {\n\
             \tconstraint c : Safe;\n\
             \tconstraint d : Safe;\n\
             \tassert c;\n\
             \tassert d;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        assert_eq!(
            diagram
                .edges
                .iter()
                .filter(|edge| edge.relation == Relation::Assert)
                .count(),
            1
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
                ends: (Some("hub".to_string()), Some("mount".to_string())),
                label: None,
            }]
        );
    }

    #[test]
    fn an_n_ary_connection_meets_at_a_dot() {
        // ends written as their own members, and three of them:
        // `n-ary-connection = n-ary-connection-dot n-ary-segment+`
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
        // the fourth box is the dot, which this connection did not name
        assert_eq!(names, ["a : R", "b : R", "c : R", ""]);
        assert_eq!(diagram.nodes[3].shape, Shape::ConnectionDot);
        // one segment out of the dot to each end, rather than a fan from
        // whichever end happened to be written first
        assert_eq!(
            diagram
                .edges
                .iter()
                .map(|e| (e.from, e.to))
                .collect::<Vec<_>>(),
            [(3, 0), (3, 1), (3, 2)]
        );
        // an end that reaches the whole of `a` has no rolename to write:
        // a square labelled `a` on the box labelled `a` says it twice
        assert_eq!(diagram.edges[0].ends, (None, None));
    }

    #[test]
    fn a_named_n_ary_connection_reads_its_name_beside_the_dot() {
        let ws = resolved(
            "part def P;\n\
             part def Whole {\n\
             \tpart a : P;\n\
             \tpart b : P;\n\
             \tpart c : P;\n\
             \tconnection wiring {\n\
             \t\tend ::> a;\n\
             \t\tend ::> b;\n\
             \t\tend ::> c;\n\
             \t}\n\
             }\n",
        );
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "Whole"));
        let dot = diagram
            .nodes
            .iter()
            .find(|node| node.shape == Shape::ConnectionDot)
            .unwrap();
        assert_eq!(dot.name, "wiring");
        assert_eq!(
            diagram.edges.iter().filter(|e| e.from == 3).count(),
            3,
            "one segment per end"
        );
    }

    #[test]
    fn an_n_ary_connection_reaching_one_box_draws_no_dot() {
        // a dot with a single segment joins nothing to anything
        let ws = resolved(
            "part def P;\n\
             part def Whole {\n\
             \tpart a : P;\n\
             \tconnection {\n\
             \t\tend ::> a;\n\
             \t\tend ::> P;\n\
             \t\tend ::> P;\n\
             \t}\n\
             }\n",
        );
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "Whole"));
        assert!(diagram
            .nodes
            .iter()
            .all(|node| node.shape != Shape::ConnectionDot));
        assert!(diagram.edges.is_empty());
    }

    #[test]
    fn an_end_is_written_with_its_multiplicity_and_adornments() {
        // `connection-graphical` writes `rolename multiplicity
        // c-adornment` at each end, and `c-adornment = (a-property |
        // a-direction | a-subsetting | a-redefinition)*`
        let ws = resolved(
            "part def TireBead;\n\
             part def TireMountingRim;\n\
             connection def BeadSeat {\n\
             \tend [1] in ordered part bead : TireBead;\n\
             \tend [0..*] abstract derived nonunique part rim : TireMountingRim;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let mut written: Vec<&str> = diagram
            .edges
            .iter()
            .filter_map(|edge| edge.label.as_deref())
            .collect();
        written.sort_unstable();
        assert_eq!(
            written,
            [
                "bead [1] in ordered",
                "rim [0..*] abstract derived nonunique"
            ]
        );
    }

    #[test]
    fn a_payload_is_written_however_it_was_declared() {
        // `flow-label = UsageDeclaration? ('of' FlowPayloadFeatureMember)?`,
        // and a payload may be written as a bare type, a type with a
        // multiplicity, or a feature declared and named
        let ws = resolved(
            "part def A;\n\
             item def Fuel;\n\
             part def V {\n\
             \tpart a : A;\n\
             \tpart b : A;\n\
             \tflow one of Fuel[2] from a to b;\n\
             \tflow two of carried :> Fuel from a to b;\n\
             \tflow three of named : Fuel from a to b;\n\
             \tflow four from a to b;\n\
             }\n",
        );
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "V"));
        let written: Vec<&str> = diagram
            .edges
            .iter()
            .filter_map(|edge| edge.label.as_deref())
            .collect();
        assert_eq!(
            written,
            [
                "one of Fuel[2]",
                "two of carried : Fuel",
                "three of named : Fuel",
                // a flow that carries nothing named still says which flow
                // it is
                "four",
            ]
        );
    }

    #[test]
    fn a_payload_may_be_declared_rather_than_named_by_its_type() {
        // `PayloadFeature = Identification PayloadFeatureSpecializationPart
        // ValuePart? | Identification ValuePart | ...`: a payload may be
        // declared and typed, or declared and only given a value
        let ws = resolved(
            "part def A;\n\
             item def Fuel;\n\
             part def V {\n\
             \tpart a : A;\n\
             \tpart b : A;\n\
             \tflow one of named : Fuel [2] from a to b;\n\
             \tflow two of counted = 5 from a to b;\n\
             }\n",
        );
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "V"));
        let written: Vec<&str> = diagram
            .edges
            .iter()
            .filter_map(|edge| edge.label.as_deref())
            .collect();
        assert_eq!(written, ["one of named : Fuel[2]", "two of counted"]);
    }

    #[test]
    fn a_payload_may_redefine_the_one_it_inherits() {
        let ws = resolved(
            "part def A;\n\
             item def Fuel;\n\
             item def Diesel :> Fuel;\n\
             part def Base {\n\
             \tpart a : A;\n\
             \tpart b : A;\n\
             \tflow f of carried : Fuel from a to b;\n\
             }\n\
             part def Sub :> Base {\n\
             \tflow g :>> f of carried :>> Base::f::carried : Diesel from a to b;\n\
             }\n",
        );
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "Sub"));
        let written: Vec<&str> = diagram
            .edges
            .iter()
            .filter_map(|edge| edge.label.as_deref())
            .collect();
        // the narrowed payload, and the inherited flow it redefines
        assert_eq!(written, ["g of carried : Diesel", "f of carried : Fuel"]);
    }

    #[test]
    fn an_end_that_refines_another_says_which() {
        // `a-subsetting` and `a-redefinition` -- what an end narrows in
        // the connection it specializes
        let ws = resolved(
            "part def TireBead;\n\
             part def TireMountingRim;\n\
             connection def Seating {\n\
             \tend part bead : TireBead;\n\
             \tend part rim : TireMountingRim;\n\
             }\n\
             connection def BeadSeat :> Seating {\n\
             \tend part inner : TireBead redefines bead;\n\
             \tend part outer : TireMountingRim subsets rim;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let mut written: Vec<&str> = diagram
            .edges
            .iter()
            .filter_map(|edge| edge.label.as_deref())
            .collect();
        written.sort_unstable();
        assert_eq!(
            written,
            ["bead", "inner redefines bead", "outer subsets rim", "rim"]
        );
    }

    #[test]
    fn each_two_ended_statement_gets_the_line_its_production_draws() {
        // clauses 8.2.3.13 to 8.2.3.16: six statements the notation draws
        // six different ways, which one plain line each would flatten
        let ws = resolved(
            "item def Fuel;\n\
             port def FuelPort;\n\
             part def Tank { port out1 : FuelPort; }\n\
             part def Engine { port in1 : FuelPort; }\n\
             part def Vehicle {\n\
             \tpart tank : Tank;\n\
             \tpart eng : Engine;\n\
             \tconnect tank.out1 to eng.in1;\n\
             \tflow fuel of Fuel from tank.out1 to eng.in1;\n\
             \tsuccession flow later of Fuel from tank.out1 to eng.in1;\n\
             \tbind tank.out1 = eng.in1;\n\
             \tallocate tank to eng;\n\
             \tinterface iface connect tank.out1 to eng.in1;\n\
             \tmessage note from tank to eng;\n\
             }\n",
        );
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "Vehicle"));
        let drawn: Vec<(Relation, Option<&str>)> = diagram
            .edges
            .iter()
            .map(|edge| (edge.relation, edge.label.as_deref()))
            .collect();
        assert_eq!(
            drawn,
            [
                (Relation::Connection, None),
                (Relation::Flow, Some("fuel of Fuel")),
                (
                    Relation::SuccessionFlow,
                    Some("\u{ab}succession flow\u{bb} later of Fuel")
                ),
                (Relation::Binding, Some("=")),
                (Relation::Allocation, Some("\u{ab}allocate\u{bb}")),
                (Relation::Interface, Some("\u{ab}interface\u{bb} iface")),
                (Relation::Message, Some("note")),
            ]
        );

        // `allocate tank to eng` reaches the whole of each part, so
        // neither end has a rolename; the rest attach to a port
        let allocation = diagram
            .edges
            .iter()
            .find(|edge| edge.relation == Relation::Allocation)
            .unwrap();
        assert_eq!(allocation.ends, (None, None));
        let flow = diagram
            .edges
            .iter()
            .find(|edge| edge.relation == Relation::Flow)
            .unwrap();
        assert_eq!(
            flow.ends,
            (Some("out1".to_string()), Some("in1".to_string()))
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
        assert_eq!(edge.label.as_deref(), Some("\u{ab}satisfy\u{bb}"));
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
    fn a_typed_end_still_finds_what_it_references() {
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
        // the boxes it joins are `a` and `b` themselves, so neither end
        // has a rolename of its own to write
        assert_eq!((diagram.edges[0].from, diagram.edges[0].to), (0, 1));
        assert_eq!(diagram.edges[0].ends, (None, None));
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
    fn a_nested_view_holds_what_wires_it_together() {
        // `interconnection-view =| (interconnection-element)*`: the parts
        // inside a box and the connections between them are one view, and
        // the parts alone are the half that says less
        let ws = resolved(
            "port def Hub;\n\
             part def Wheel { port hub : Hub; }\n\
             part def Axle { port mount : Hub; }\n\
             part def Chassis {\n\
             \tpart w : Wheel;\n\
             \tpart a : Axle;\n\
             \tconnect w.hub to a.mount;\n\
             }\n\
             part def Car { part c : Chassis; }\n",
        );
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "Car"));
        let chassis = &diagram.nodes[0];
        assert_eq!(
            chassis
                .children
                .iter()
                .map(|child| child.name.as_str())
                .collect::<Vec<_>>(),
            ["w : Wheel", "a : Axle"]
        );
        assert_eq!(chassis.links.len(), 1);
        let link = &chassis.links[0];
        assert_eq!((link.from, link.to), (0, 1));
        assert_eq!(link.relation, Relation::Connection);
        assert_eq!(
            link.ends,
            (Some("hub".to_string()), Some("mount".to_string()))
        );
    }

    #[test]
    fn a_behaviour_nests_its_own_flow() {
        // `action-flow-compartment` holds the same kind of view, so an
        // action drawn as an empty frame is one whose steps went unsaid
        let ws = resolved(
            "action def Grind;\n\
             action def Brew;\n\
             action def MakeCoffee {\n\
             \taction g : Grind;\n\
             \taction b : Brew;\n\
             \tfirst g then b;\n\
             }\n\
             part def Machine { action make : MakeCoffee; }\n",
        );
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "Machine"));
        let make = &diagram.nodes[0];
        assert_eq!(
            make.children
                .iter()
                .map(|child| child.name.as_str())
                .collect::<Vec<_>>(),
            ["g : Grind", "b : Brew"]
        );
        assert_eq!(make.links.len(), 1);
        assert_eq!(make.links[0].relation, Relation::Succession);
        // and what became a box inside is not listed in a compartment too
        assert!(lines(make).all(|line| line.name != "g" && line.name != "b"));
    }

    #[test]
    fn a_succession_is_not_a_transition() {
        // `transition` is a plain line, `aflow-succession` a dashed one:
        // a step following a step is not a machine changing state, and
        // drawing both the same way says it is
        let ws = resolved(
            "state def Modes {\n\
             \tstate off;\n\
             \tstate on;\n\
             \ttransition off_to_on first off then on;\n\
             \tsuccession on then off;\n\
             }\n",
        );
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "Modes"));
        assert_eq!(
            diagram
                .edges
                .iter()
                .map(|edge| edge.relation)
                .collect::<Vec<_>>(),
            [Relation::Transition, Relation::Succession]
        );
    }

    #[test]
    fn a_behaviour_calls_its_directed_features_parameters() {
        let ws = resolved(
            "item def Fuel;\n\
             action def Burn {\n\
             \tin item supply : Fuel;\n\
             \tout item spent : Fuel;\n\
             }\n\
             part def Engine { action b : Burn; }\n",
        );
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "Engine"));
        assert_eq!(
            diagram.nodes[0]
                .compartments
                .iter()
                .map(|compartment| compartment.label)
                .collect::<Vec<_>>(),
            ["parameters"]
        );
    }

    #[test]
    fn a_portion_and_an_individual_have_compartments_of_their_own() {
        // `individuals-compartment`, `snapshots-compartment` and
        // `timeslices-compartment` come from the flags the notation
        // writes, and all three are occurrences
        let ws = resolved(
            "occurrence def O;\n\
             part def P {\n\
             \toccurrence o : O;\n\
             \tindividual i : O;\n\
             \tsnapshot s : O;\n\
             \ttimeslice t : O;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let part = diagram.nodes.iter().find(|n| n.name == "P").unwrap();
        assert_eq!(
            part.compartments
                .iter()
                .map(|compartment| compartment.label)
                .collect::<Vec<_>>(),
            ["occurrences", "individuals", "snapshots", "timeslices"]
        );
    }

    #[test]
    fn a_transition_is_a_line_and_a_succession_has_a_compartment() {
        // the states clause gives a succession its own compartment
        // (`successions-compartment`) and a transition none at all: a
        // transition listed under `actions` says the state owns an action
        // by that name
        let ws = resolved(
            "state def Modes {\n\
             \tstate off;\n\
             \tstate on;\n\
             \ttransition off_to_on first off then on;\n\
             \tsuccession back first on then off;\n\
             }\n\
             part def Box { state m : Modes; }\n",
        );
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "Box"));
        let listed: Vec<(&str, &str)> = diagram.nodes[0]
            .compartments
            .iter()
            .flat_map(|compartment| {
                compartment
                    .lines
                    .iter()
                    .map(move |line| (compartment.label, line.name.as_str()))
            })
            .collect();
        assert_eq!(listed, [("successions", "back")]);
    }

    #[test]
    fn a_state_action_says_which_of_the_three_it_is() {
        // `entry-action-name-comp = '\u{ab}' 'entry' OccurrenceUsagePrefix
        // 'action' '\u{bb}'`, and the same for `do` and `exit`
        let ws = resolved(
            "action def Warm;\n\
             action def Watch;\n\
             action def Cool;\n\
             state def Heating {\n\
             \tentry action begin : Warm;\n\
             \tdo action keep : Watch;\n\
             \texit action stop : Cool;\n\
             }\n\
             part def Oven { state h : Heating; }\n",
        );
        let diagram = interconnection_diagram(ws.model(), definition(&ws, "Oven"));
        assert_eq!(
            diagram.nodes[0]
                .children
                .iter()
                .map(|child| child.keyword.as_str())
                .collect::<Vec<_>>(),
            ["entry action", "do action", "exit action"]
        );
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
                    ends: (None, None),
                    label: Some("off_to_on".to_string()),
                },
                Edge {
                    from: 1,
                    to: 0,
                    relation: Relation::Transition,
                    ends: (None, None),
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
    fn a_control_node_is_drawn_as_its_glyph_not_as_a_box() {
        // `action-flow-node = start-node | done-node | terminate-node |
        // fork-node | join-node | decision-node | merge-node | ...`:
        // none of these is a box, and drawing them as boxes says a flow
        // splits at an action
        let diagram = internal(
            "action def Brew {\n\
             \taction heat;\n\
             \tfork split;\n\
             \tjoin gather;\n\
             \tmerge again;\n\
             \tdecide which;\n\
             \taction stop terminate;\n\
             \tfirst heat then split;\n\
             \tfirst split then gather;\n\
             \tfirst gather then which;\n\
             \tfirst which then again;\n\
             \tfirst again then stop;\n\
             }\n",
            "Brew",
        );
        let drawn: Vec<(&str, Shape)> = diagram
            .nodes
            .iter()
            .map(|node| (node.name.as_str(), node.shape))
            .collect();
        assert_eq!(
            drawn,
            [
                ("heat", Shape::Box),
                ("split", Shape::Bar),
                ("gather", Shape::Bar),
                ("again", Shape::Diamond),
                ("which", Shape::Diamond),
                ("stop", Shape::Cross),
            ]
        );
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
        // none of them is the start marker, and the join is the bar the
        // standard draws a `join-node` as
        assert_eq!(
            diagram.nodes.iter().map(|n| n.shape).collect::<Vec<_>>(),
            [Shape::Box, Shape::Bar, Shape::Box]
        );
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
            .all(|e| e.relation == Relation::Succession));
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
            .any(|e| e.from == initial && e.relation == Relation::Succession));
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

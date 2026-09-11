//! What a model implies for code, in no language in particular.
//!
//! `generate_rust` answers this in Rust and hands back Rust. Whoever
//! writes Python or C gets nothing from that, and reading the SysML
//! instead means guessing at what it cannot see: whether `wheels :
//! Wheel[4]` is an array or a list, whether a `part` is owned or
//! referred to, what `ISQ::MassValue` bottoms out in.
//!
//! So it is stated once, as data. Nothing here decides anything about a
//! language: no identifier is spelled, no container named, no type
//! mapped.

use serde::Serialize;

use sysml_model::{ElementId, ElementKind, Role, Value};
use sysml_semantics::Workspace;

/// Everything a generator would have to write.
#[derive(Serialize)]
pub struct Plan {
    /// Whether the model this was read off resolves.
    ///
    /// A feature whose type resolved to nothing arrives with no type at
    /// all. `generate_rust` refuses such a model outright; this hands
    /// the plan over and says what is missing.
    pub checked: Checked,
    /// Each definition the model declares, in declaration order.
    pub definitions: Vec<Definition>,
    /// What a package declares outright rather than inside a definition:
    /// `attribute ledPinNumber : Integer = 13;`. Without them a default
    /// that names one names nothing the plan contains.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub constants: Vec<Feature>,
}

/// Whether the model resolves, and what did not.
#[derive(Serialize)]
pub struct Checked {
    /// Whether every name in the planned files found something, and
    /// every one of them parsed.
    pub ok: bool,
    /// What the parser could not read. A file that does not parse is
    /// missing whole declarations rather than one type, so a plan built
    /// from it says nothing about what is not there.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub syntax: Vec<String>,
    /// The names that did not, as the model wrote them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unresolved: Vec<String>,
}

/// One definition, and what shape it has in code.
#[derive(Serialize)]
pub struct Definition {
    /// Its qualified name, as the model spells it.
    pub of: String,
    /// The short name it also answers to, where it has one:
    /// `requirement def <'S.2'> PerceptiblePeriod` is `S.2` to everything
    /// that refers to it, and a reader given only the long name cannot
    /// follow what points at it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_name: Option<String>,
    /// The metaclass, for a reader that knows SysML.
    pub kind: &'static str,
    /// What it is in code, for one that does not.
    pub shape: &'static str,
    /// What the definition itself bottoms out in, where it is a value
    /// with a primitive under it. `attribute def Millis :> Integer;` is
    /// an integer with a name, and writing an empty class for it is
    /// writing something the model does not say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primitive: Option<&'static str>,
    /// What the model says it is for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    /// What the model annotates it with, verbatim.
    ///
    /// Most usefully where the thing already exists: `@code { :>>
    /// writtenIn = "rust"; :>> path = "crate::hal::Gpio"; }` says this
    /// port *is* a written type, and the actions whose paths begin the
    /// same way are its methods. Without it they belong to nothing.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// Whether the model forbids instances of it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub is_abstract: bool,
    /// What the model declares it specializes, qualified: the direct
    /// parent, not everything above it and not the library
    /// specialization every definition gets whether it asks or not.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub specializes: Vec<String>,
    /// Its own features: attributes, parts, ports, parameters.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<Feature>,
    /// The members of an enumeration.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    /// What a requirement is about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Feature>,
    /// What a requirement demands, as the model wrote it. Without these
    /// a requirement arrives as a parameter list and a sentence, and
    /// whoever writes the check invents it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    /// What it takes as given before it demands anything.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub assumes: Vec<String>,
    /// The variants of a variation: exactly one of these, not all of
    /// them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<Variant>,
    /// The states of a state definition, in declaration order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<String>,
    /// The one it starts in, which the model writes as `entry; then x;`
    /// and is not the same as the first one declared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial: Option<String>,
    /// What happens on the way in, throughout, and on the way out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_entry: Option<String>,
    /// What it does for as long as it is in the machine.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub during: Option<String>,
    /// What it does on the way out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_exit: Option<String>,
    /// What moves it between them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub transitions: Vec<Transition>,
    /// What a calculation or behaviour hands back, where it declares
    /// one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returns: Option<Feature>,
    /// What a calculation or constraint works out, as the model wrote
    /// it. Translating it is the reader's; this toolchain does not
    /// pretend to have done it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    /// The behaviours it carries out, which is what makes a definition
    /// something that *does* anything: a part that performs an action
    /// is written, in a language with methods, as a method.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub performs: Vec<Performed>,
    /// The steps of an action, in the order the model puts them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
    /// What passes between them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub flows: Vec<Flow>,
    /// What is joined to what inside it. `connect pump to tank;` is the
    /// whole of why the two parts are there together.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<Connection>,
}

/// One feature of a definition, with everything a generator must ask
/// about it and nothing about how any language would write it.
#[derive(Serialize)]
pub struct Feature {
    /// What the model calls it. A `return : Real;` has no name and does
    /// not need one.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// The metaclass: `AttributeUsage`, `PartUsage`, `PortUsage`, and so
    /// on. What kind of thing it is, which decides more than its type
    /// does -- a part is a thing the whole is made of, a port is a place
    /// it connects.
    pub kind: &'static str,
    /// What types it, qualified.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub typed_by: Option<String>,
    /// Whether it is one of the two sides a connection relates rather
    /// than something the definition holds. `connection def Pipe { end
    /// source : Pump; end target : Tank; }` is a pipe between a pump and
    /// a tank, not a pipe with a pump inside it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub end: bool,
    /// What that type bottoms out in, where it bottoms out in one of the
    /// standard library's primitives: `Real`, `Integer`, `Natural`,
    /// `Positive`, `Boolean`, `String`. `ISQ::MassValue` is a `Real`,
    /// and no amount of reading its name says so.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primitive: Option<&'static str>,
    /// How many of it there are. Absent is one of it, which is what
    /// everything is unless the model says otherwise.
    #[serde(skip_serializing_if = "Multiplicity::is_one")]
    pub multiplicity: Multiplicity,
    /// Whether the whole owns it (`part`) or refers to it (`ref part`).
    /// Said only of what a whole can be made of: a value is owned by
    /// nobody.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub composite: Option<bool>,
    /// Whether the collection keeps its order, and whether it may hold
    /// the same thing twice: a list, a set, or neither. Said only of
    /// collections, since one of a thing is in no order and is not two
    /// of anything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordered: Option<bool>,
    /// Whether it may hold the same thing twice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unique: Option<bool>,
    /// Which way a parameter goes: `in`, `out` or `inout`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    /// The `= value` it was declared with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Given>,
    /// The inherited feature it redefines, qualified: the same thing
    /// said again, not a second one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redefines: Option<String>,
    /// What the model says it is for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}

/// How many of it there are.
#[derive(PartialEq, Serialize)]
pub struct Multiplicity {
    /// The fewest there may be.
    pub lower: i64,
    /// `null` is `*`: any number of them.
    pub upper: Option<i64>,
}

impl Multiplicity {
    /// One of the thing, which is what everything is unless the model
    /// says otherwise -- and so what is not worth saying.
    fn is_one(&self) -> bool {
        self.lower == 1 && self.upper == Some(1)
    }
}

/// A value the model declared, whether or not this toolchain understood
/// it.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Given {
    /// A literal, as JSON: a number, a string, a boolean.
    Literal(serde_json::Value),
    /// Anything else, as the model wrote it.
    Expression(String),
}

/// One `@name { ... }` a definition carries, as the model wrote it.
#[derive(Serialize)]
pub struct Annotation {
    /// What is being said, by the name of the metadata definition:
    /// `rust`, `Safety`, whatever the model declared.
    pub of: String,
    /// What it sets, by property name.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub says: std::collections::BTreeMap<String, Given>,
}

/// One variant of a variation.
#[derive(Serialize)]
pub struct Variant {
    /// What the model calls it.
    pub name: String,
    /// What it carries, where it carries something.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub typed_by: Option<String>,
    /// What the model says it is for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}

/// One transition of a state machine.
#[derive(Serialize)]
pub struct Transition {
    /// What the model calls it.
    pub name: String,
    /// The state it leaves.
    pub from: String,
    /// The state it arrives in.
    pub to: String,
    /// What it waits for, where it waits for something.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<Trigger>,
    /// The condition, as the model wrote it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guard: Option<String>,
    /// Whether the model says something happens on the way. What it is
    /// is the reader's to write.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub effect: bool,
}

/// The event a transition waits for, and what it carries.
#[derive(Serialize)]
pub struct Trigger {
    /// The name the payload is bound to.
    pub name: String,
    /// What the payload is.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub typed_by: Option<String>,
}

/// One behaviour a definition carries out. What it performs is named
/// rather than described again: the behaviour is a definition of its
/// own wherever the model declares it.
#[derive(Serialize)]
pub struct Performed {
    /// What the model calls it here.
    pub name: String,
    /// The behaviour performed, qualified.
    #[serde(rename = "of", skip_serializing_if = "Option::is_none")]
    pub performed: Option<String>,
    /// What the model says it is for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}

/// One step of an action.
#[derive(Serialize)]
pub struct Step {
    /// What the model calls it.
    pub name: String,
    /// What it performs.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub typed_by: Option<String>,
    /// The steps that must finish before it starts.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
}

/// Two of a definition's parts, joined.
#[derive(Serialize)]
pub struct Connection {
    /// What the model calls it, where it calls it anything: `connect a
    /// to b;` names nothing, and the joining is the point.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// What kind of connection it is, qualified, where the model says.
    #[serde(rename = "of", skip_serializing_if = "Option::is_none")]
    pub connected_by: Option<String>,
    /// The two sides, as dotted paths from the definition.
    pub from: String,
    /// The other side.
    pub to: String,
    /// What the model says it is for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}

/// One thing passing from one place to another.
#[derive(Serialize)]
pub struct Flow {
    /// Where it comes from, as a dotted path from the definition.
    pub from: String,
    /// Where it goes.
    pub to: String,
}

/// The plan for everything under `roots`.
pub fn of(ws: &mut Workspace, roots: &[ElementId]) -> Plan {
    let mut definitions = Vec::new();
    let mut constants = Vec::new();
    for &root in roots {
        for id in ws.model().descendants(root) {
            if ws.model().name(id).is_none() {
                continue;
            }
            if let Some(shape) = shape_of(ws, id) {
                definitions.push(definition(ws, id, shape));
                continue;
            }
            if declared_by_a_package(ws, id) {
                let named = ws.qualified_name_of(id);
                let mut it = feature(ws, id);
                it.name = named;
                constants.push(it);
            }
        }
    }
    // Only the files the plan is about: a model checked without its
    // library reports every reference into the library as unresolved,
    // and none of those is a hole in what was planned.
    let planned: std::collections::HashSet<usize> = roots
        .iter()
        .filter_map(|&root| ws.element_file(root))
        .collect();
    let mut missed: Vec<String> = ws
        .unresolved()
        .iter()
        .filter(|it| planned.contains(&it.file))
        .map(|it| it.name.clone())
        .collect();
    missed.dedup();
    let broken: Vec<String> = ws
        .findings(&[])
        .syntax
        .iter()
        .filter(|it| planned.contains(&it.file))
        .map(|it| format!("{}: {}", ws.file_name(it.file), it.what))
        .collect();
    Plan {
        checked: Checked {
            ok: missed.is_empty() && broken.is_empty(),
            syntax: broken,
            unresolved: missed,
        },
        definitions,
        constants,
    }
}

/// Whether a usage is a package's own rather than a definition's: a
/// constant of the model, and named from the root by everything that
/// reaches for it.
fn declared_by_a_package(ws: &Workspace, id: ElementId) -> bool {
    let model = ws.model();
    if !is_feature_kind(model.kind(id)) {
        return false;
    }
    model
        .owner(id)
        .is_some_and(|owner| model.kind(owner).is_a(ElementKind::Package))
}

/// What a definition is in code, or nothing where it is not a
/// definition at all.
fn shape_of(ws: &mut Workspace, id: ElementId) -> Option<&'static str> {
    // asked before the walk over supertypes below, which wants the
    // workspace to itself
    let bare = !ws
        .model()
        .owned(id)
        .iter()
        .any(|&child| is_feature_kind(ws.model().kind(child)));
    let under = primitive_of(ws, id).is_some();
    let model = ws.model();
    Some(match model.kind(id) {
        ElementKind::EnumerationDefinition => "enumeration",
        ElementKind::StateDefinition => "state machine",
        ElementKind::CalculationDefinition | ElementKind::ConstraintDefinition => "function",
        ElementKind::ActionDefinition => "behaviour",
        // `attribute def Millis :> Integer;` is a primitive under
        // another name; as a record it would be an empty class.
        ElementKind::AttributeDefinition if under && bare => "value",
        // A port is a place a thing connects and a requirement is a
        // question asked of a model; `record` covered both and said
        // less than the metaclass beside it.
        ElementKind::PortDefinition | ElementKind::InterfaceDefinition => "port",
        ElementKind::RequirementDefinition => "requirement",
        ElementKind::PartDefinition
        | ElementKind::ItemDefinition
        | ElementKind::AttributeDefinition
        | ElementKind::ConnectionDefinition => {
            let variation = model
                .owned(id)
                .iter()
                .any(|&child| model.member_role(child) == Some(Role::Variant));
            match (variation, model.is_abstract(id)) {
                (true, _) => "variation",
                (_, true) => "abstract",
                _ => "record",
            }
        }
        // KerML declares the same things in its own words -- half the
        // standard library is written that way -- and planning it as
        // nothing read as "there is nothing here to write".
        //
        // Asked after the SysML kinds, not instead: a `part def` is a
        // `Structure` too and has more to say for itself.
        kind if kind.is_a(ElementKind::Function) => "function",
        kind if kind.is_a(ElementKind::Behavior) => "behaviour",
        kind if kind.is_a(ElementKind::DataType) && under && bare => "value",
        kind if kind.is_a(ElementKind::Classifier) => match model.is_abstract(id) {
            true => "abstract",
            false => "record",
        },
        _ => return None,
    })
}

fn definition(ws: &mut Workspace, id: ElementId, shape: &'static str) -> Definition {
    let specializes = declared_parents(ws, id);
    let owned = ws.model().owned(id).to_vec();
    let says_nothing_by_being_abstract = matches!(shape, "enumeration" | "variation");
    Definition {
        of: ws.qualified_name_of(id),
        short_name: ws
            .model()
            .maybe(id, "declaredShortName")
            .and_then(Value::as_str)
            .map(str::to_string),
        kind: ws.model().kind(id).name(),
        shape,
        primitive: primitive_of(ws, id),
        documentation: ws.documentation_of(id),
        annotations: annotations(ws, &owned),
        // An enumeration and a variation are abstract by being what they
        // are, so saying it again says nothing and reads as "write an
        // abstract base for this".
        is_abstract: !says_nothing_by_being_abstract && ws.model().is_abstract(id),
        specializes,
        features: {
            let mine: Vec<ElementId> = owned
                .iter()
                .copied()
                .filter(|&child| is_feature(ws, child))
                .collect();
            mine.into_iter().map(|child| feature(ws, child)).collect()
        },
        values: owned
            .iter()
            .filter(|&&child| ws.model().kind(child) == ElementKind::EnumerationUsage)
            .filter_map(|&child| ws.model().name(child).map(str::to_string))
            .collect(),
        subject: {
            let asked = owned
                .iter()
                .copied()
                .find(|&child| ws.model().member_role(child) == Some(Role::Subject));
            asked.map(|child| feature(ws, child))
        },
        requires: constraints(ws, &owned, Role::Require),
        assumes: constraints(ws, &owned, Role::Assume),
        variants: {
            // an enumeration's members are variants too, and saying them
            // twice over is two lists to keep in step
            let mine: Vec<ElementId> = match shape == "enumeration" {
                true => Vec::new(),
                false => owned
                    .iter()
                    .copied()
                    .filter(|&child| {
                        ws.model().member_role(child) == Some(Role::Variant)
                            && ws.model().name(child).is_some()
                    })
                    .collect(),
            };
            mine.into_iter().map(|child| variant(ws, child)).collect()
        },
        states: owned
            .iter()
            .filter(|&&child| ws.model().kind(child) == ElementKind::StateUsage)
            .filter_map(|&child| ws.model().name(child).map(str::to_string))
            .collect(),
        initial: initial_state(ws, &owned),
        on_entry: acting(ws, &owned, Role::Entry),
        during: acting(ws, &owned, Role::Do),
        on_exit: acting(ws, &owned, Role::Exit),
        transitions: owned
            .iter()
            .filter_map(|&child| transition(ws, child))
            .collect(),
        returns: {
            let result = owned
                .iter()
                .copied()
                .find(|&child| ws.model().member_role(child) == Some(Role::Return));
            result.map(|child| feature(ws, child))
        },
        expression: expression_of(ws, id),
        performs: {
            let mine: Vec<ElementId> = owned
                .iter()
                .copied()
                .filter(|&child| {
                    ws.model().kind(child) == ElementKind::PerformActionUsage
                        && (ws.model().name(child).is_some()
                            || sysml_model::redefined(ws.model(), child).is_some())
                })
                .collect();
            mine.into_iter().map(|child| performed(ws, child)).collect()
        },
        steps: {
            // an entry, do or exit action is the machinery of a state
            // machine rather than a step of a behaviour, and is said
            // where it belongs
            let mine: Vec<ElementId> = owned
                .iter()
                .copied()
                .filter(|&child| {
                    ws.model().kind(child) == ElementKind::ActionUsage
                        && !matches!(
                            ws.model().member_role(child),
                            Some(Role::Entry | Role::Do | Role::Exit)
                        )
                })
                .collect();
            mine.into_iter()
                .filter_map(|child| step(ws, child, &owned))
                .collect()
        },
        flows: owned.iter().filter_map(|&child| flow(ws, child)).collect(),
        connections: {
            let mine: Vec<ElementId> = owned
                .iter()
                .copied()
                .filter(|&child| {
                    ws.model().kind(child).is_a(ElementKind::ConnectionUsage)
                        && !ws.model().kind(child).is_a(ElementKind::FlowUsage)
                })
                .collect();
            mine.into_iter()
                .filter_map(|child| connection(ws, child))
                .collect()
        },
    }
}

/// Whether a member is one of the definition's features rather than
/// part of its machinery: a state, a transition, a flow and a
/// documentation comment are all owned members and none of them is a
/// field.
fn is_feature(ws: &Workspace, child: ElementId) -> bool {
    let model = ws.model();
    // A usage that only narrows an inherited one is written without a
    // name of its own -- `attribute :>> mass = 1200.0;` -- and borrows
    // the name it redefines. Read as nameless it is dropped, and what
    // the model says a car's mass is goes unsaid.
    if model.name(child).is_none() && sysml_model::redefined(model, child).is_none() {
        return false;
    }
    if matches!(
        model.member_role(child),
        Some(
            Role::Variant
                | Role::Return
                | Role::Entry
                | Role::Do
                | Role::Exit
                | Role::Subject
                | Role::Require
                | Role::Assume
        )
    ) {
        return false;
    }
    is_feature_kind(model.kind(child))
}

/// Whether composition is a question worth asking of a metaclass. A
/// value is not something its owner is made of, however it is written,
/// so saying so of every attribute and every parameter is a column of
/// `false` that means nothing.
fn ownable(kind: ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::PartUsage
            | ElementKind::ItemUsage
            | ElementKind::PortUsage
            | ElementKind::OccurrenceUsage
    )
}

/// The metaclasses that are a definition's fields rather than its
/// machinery.
fn is_feature_kind(kind: ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::AttributeUsage
            | ElementKind::PartUsage
            | ElementKind::ItemUsage
            | ElementKind::PortUsage
            | ElementKind::ReferenceUsage
            | ElementKind::OccurrenceUsage
            // KerML's own word for all of those: `feature radius : Real;`
            | ElementKind::Feature
    )
}

fn feature(ws: &mut Workspace, child: ElementId) -> Feature {
    let typed = ws.model().type_of(child).or_else(|| {
        sysml_model::redefined(ws.model(), child).and_then(|it| ws.model().type_of(it))
    });
    let (lower, upper) = bounds(ws, child);
    let borrowed = sysml_model::redefined(ws.model(), child).and_then(|it| ws.model().name(it));
    Feature {
        name: ws
            .model()
            .name(child)
            .or(borrowed)
            .unwrap_or_default()
            .to_string(),
        kind: ws.model().kind(child).name(),
        typed_by: typed.map(|ty| ws.qualified_name_of(ty)),
        primitive: typed.and_then(|ty| primitive_of(ws, ty)),
        multiplicity: Multiplicity { lower, upper },
        // `declared_flag` rather than `flag`: silence about ordering or
        // uniqueness is the specification's answer, not `false`, and a
        // generator reading it off silence writes a list for a set.
        // An end is a feature that says it is one -- dropped as "not a
        // field", `connection def Pipe` planned as an empty record.
        end: ws.model().declared_flag(child, "isEnd"),
        composite: ownable(ws.model().kind(child))
            .then(|| ws.model().declared_flag(child, "isComposite")),
        ordered: (upper != Some(1)).then(|| ws.model().declared_flag(child, "isOrdered")),
        unique: (upper != Some(1)).then(|| ws.model().declared_flag(child, "isUnique")),
        direction: ws.model().direction(child).map(str::to_string),
        default: given(ws, child),
        redefines: redefines(ws, child),
        documentation: ws.documentation_of(child),
    }
}

/// The declared multiplicity, as two numbers. Everything is one of
/// itself unless the model says otherwise.
fn bounds(ws: &Workspace, usage: ElementId) -> (i64, Option<i64>) {
    use sysml_model::Bound;
    let Some(declared) = sysml_model::declared_multiplicity(ws.model(), usage) else {
        return (1, Some(1));
    };
    // A bound written as anything but a literal -- `[n]`, naming a
    // feature -- says nothing this can put a number to, so it says any
    // number of them rather than one, which is the reading that cannot
    // be mistaken for a model that said nothing at all.
    let number = |end: Option<Bound>| match end {
        Some(Bound::Exactly(n)) => Some(n),
        _ => None,
    };
    // `[4]` is a bound on its own, which is both ends at once
    if declared.bound.is_some() {
        let exactly = number(declared.bound);
        return (exactly.unwrap_or(0), exactly);
    }
    (number(declared.lower).unwrap_or(0), number(declared.upper))
}

/// The standard library primitive a type bottoms out in, walking what it
/// specializes. `ISQ::MassValue` is a `Real` five specializations up.
fn primitive_of(ws: &mut Workspace, ty: ElementId) -> Option<&'static str> {
    let mut seen = std::collections::HashSet::new();
    let mut queue = vec![ty];
    while let Some(next) = queue.pop() {
        if !seen.insert(next) {
            continue;
        }
        if let Some(found) = ws.model().name(next).and_then(sysml_model::primitive) {
            return Some(found);
        }
        queue.extend(ws.supertypes(next));
    }
    None
}

/// The `= value` a usage was declared with.
fn given(ws: &Workspace, usage: ElementId) -> Option<Given> {
    use sysml_model::{Declared, Literal};
    Some(match sysml_model::declared_value(ws.model(), usage)? {
        Declared::Text(text) => Given::Expression(text),
        Declared::Literal(Literal::Real(real)) => Given::Literal(serde_json::json!(real)),
        Declared::Literal(Literal::Int(int)) => Given::Literal(serde_json::json!(int)),
        Declared::Literal(Literal::Bool(flag)) => Given::Literal(serde_json::json!(flag)),
        Declared::Literal(Literal::String(text)) => Given::Literal(serde_json::json!(text)),
    })
}

/// What an inherited feature this one redefines is called.
fn redefines(ws: &mut Workspace, usage: ElementId) -> Option<String> {
    let redefined = sysml_model::redefined(ws.model(), usage)?;
    Some(ws.qualified_name_of(redefined))
}

/// What the model itself declares a definition specializes.
///
/// [`Workspace::supertypes`] walks the whole way up and includes what
/// the library implies: the right answer to "what kind of thing is
/// this", the wrong one to "what does it inherit from", where the
/// parent is one name among several with nothing to mark it.
fn declared_parents(ws: &mut Workspace, id: ElementId) -> Vec<String> {
    let model = ws.model();
    let written: Vec<ElementId> = model
        .owned(id)
        .iter()
        .copied()
        .filter(|&child| !model.flag(child, "isImplied"))
        .filter(|&child| model.kind(child) == ElementKind::Subclassification)
        .filter_map(|child| model.maybe(child, "superclassifier")?.as_id())
        .collect();
    written
        .into_iter()
        .map(|up| ws.qualified_name_of(up))
        .collect()
}

/// What the model annotates an element with. A metadata usage sets its
/// properties by redefining them -- `:>> path = "crate::hal::Gpio";` --
/// so each is read off the redefinition and the value beside it.
fn annotations(ws: &mut Workspace, owned: &[ElementId]) -> Vec<Annotation> {
    let carried: Vec<ElementId> = owned
        .iter()
        .copied()
        .filter(|&child| ws.model().kind(child) == ElementKind::MetadataUsage)
        .collect();
    carried
        .into_iter()
        .filter_map(|usage| {
            let named = ws.model().type_of(usage)?;
            let says = ws
                .model()
                .owned(usage)
                .iter()
                .copied()
                .filter_map(|setting| {
                    let redefined = sysml_model::redefined(ws.model(), setting)?;
                    let name = ws.model().name(redefined)?.to_string();
                    Some((name, given(ws, setting)?))
                })
                .collect();
            Some(Annotation {
                of: ws.qualified_name_of(named),
                says,
            })
        })
        .collect()
}

/// What a requirement demands or assumes, as the model wrote it.
fn constraints(ws: &Workspace, owned: &[ElementId], role: Role) -> Vec<String> {
    let model = ws.model();
    owned
        .iter()
        .copied()
        .filter(|&child| model.member_role(child) == Some(role))
        .filter_map(|constraint| {
            model
                .owned(constraint)
                .iter()
                .copied()
                .find(|&it| model.kind(it) == ElementKind::Expression)
                .and_then(|it| sysml_model::expression_text(model, it))
        })
        .collect()
}

/// The state a machine starts in. Written `entry; then dark;` -- an
/// entry action with a succession out of it -- so it is neither the
/// first state declared nor anything a state says of itself.
fn initial_state(ws: &Workspace, owned: &[ElementId]) -> Option<String> {
    let model = ws.model();
    let entry = owned
        .iter()
        .copied()
        .find(|&child| model.member_role(child) == Some(Role::Entry))?;
    owned.iter().copied().find_map(|other| {
        if !model.kind(other).is_a(ElementKind::SuccessionAsUsage) {
            return None;
        }
        let ends: Vec<ElementId> = model
            .owned(other)
            .iter()
            .filter(|&&end| model.kind(end) == ElementKind::Feature)
            .filter_map(|&end| sysml_model::end_reaches(model, end).last().copied())
            .collect();
        let (from, to) = (*ends.first()?, *ends.get(1)?);
        (from == entry)
            .then(|| model.name(to))
            .flatten()
            .map(str::to_string)
    })
}

/// The action a state machine performs in one of the three places it can
/// perform one, by the name it is called or the name of what it does.
fn acting(ws: &Workspace, owned: &[ElementId], role: Role) -> Option<String> {
    let model = ws.model();
    let acted = owned
        .iter()
        .copied()
        .find(|&child| model.member_role(child) == Some(role))?;
    model.name(acted).map(str::to_string).or_else(|| {
        model
            .type_of(acted)
            .and_then(|ty| model.name(ty))
            .map(str::to_string)
    })
}

/// The result expression of a calculation or constraint, as written.
fn expression_of(ws: &Workspace, id: ElementId) -> Option<String> {
    sysml_model::result_expression_text(ws.model(), id)
}

fn performed(ws: &mut Workspace, child: ElementId) -> Performed {
    // `perform action :>> pause { ... }` narrows an inherited one and is
    // written with no name and no type of its own: both are borrowed
    // from what it redefines, the way a feature borrows them
    let redefines = sysml_model::redefined(ws.model(), child);
    let of = ws
        .model()
        .type_of(child)
        .or_else(|| redefines.and_then(|it| ws.model().type_of(it)));
    let borrowed = redefines.and_then(|it| ws.model().name(it));
    Performed {
        name: ws
            .model()
            .name(child)
            .or(borrowed)
            .expect("picked by having a name, its own or a borrowed one")
            .to_string(),
        performed: of.map(|it| ws.qualified_name_of(it)),
        documentation: ws.documentation_of(child),
    }
}

fn variant(ws: &mut Workspace, child: ElementId) -> Variant {
    let typed = ws.model().type_of(child);
    Variant {
        name: ws
            .model()
            .name(child)
            .expect("picked by having a name")
            .to_string(),
        typed_by: typed.map(|ty| ws.qualified_name_of(ty)),
        documentation: ws.documentation_of(child),
    }
}

fn transition(ws: &mut Workspace, usage: ElementId) -> Option<Transition> {
    let model = ws.model();
    if model.kind(usage) != ElementKind::TransitionUsage {
        return None;
    }
    let name = model.name(usage)?.to_string();
    // A transition is not a connector: what it relates it relates
    // through the `Succession` it owns, and that is where its ends are.
    let relates = model
        .owned(usage)
        .iter()
        .copied()
        .find(|&child| model.kind(child).is_a(ElementKind::SuccessionAsUsage))
        .unwrap_or(usage);
    let ends: Vec<ElementId> = model
        .owned(relates)
        .iter()
        .filter(|&&child| model.kind(child) == ElementKind::Feature)
        .filter_map(|&child| sysml_model::end_reaches(model, child).last().copied())
        .collect();
    let [from, to] = ends[..] else { return None };
    let on = match model.get(usage, "triggerAction") {
        Some(Value::RefList(triggers)) => triggers.first().and_then(|&accept| {
            let waits = sysml_model::payload_parameter(model, accept)?;
            let typed = model.type_of(waits);
            Some(Trigger {
                name: model.name(waits)?.to_string(),
                typed_by: typed.map(|ty| ws.qualified_name_of(ty)),
            })
        }),
        _ => None,
    };
    let model = ws.model();
    let guard = match model.get(usage, "guardExpression") {
        Some(Value::RefList(guards)) => guards
            .first()
            .and_then(|&it| sysml_model::expression_text(model, it)),
        _ => None,
    };
    let effect =
        matches!(model.get(usage, "effectAction"), Some(Value::RefList(list)) if !list.is_empty());
    Some(Transition {
        name,
        from: model.name(from)?.to_string(),
        to: model.name(to)?.to_string(),
        on,
        guard,
        effect,
    })
}

fn step(ws: &mut Workspace, child: ElementId, beside: &[ElementId]) -> Option<Step> {
    let typed = ws.model().type_of(child);
    let name = ws.model().name(child)?.to_string();
    // `first a then b` is a succession the definition owns, so what a
    // step waits for is read off its neighbours rather than off itself
    let successions: Vec<ElementId> = beside
        .iter()
        .copied()
        .filter(|&other| ws.model().kind(other).is_a(ElementKind::SuccessionAsUsage))
        .collect();
    let model = ws.model();
    let after: Vec<String> = successions
        .into_iter()
        .filter_map(|other| {
            let ends: Vec<ElementId> = model
                .owned(other)
                .iter()
                .filter(|&&end| model.kind(end) == ElementKind::Feature)
                .filter_map(|&end| sysml_model::end_reaches(model, end).last().copied())
                .collect();
            let (before, next) = (*ends.first()?, *ends.get(1)?);
            (next == child)
                .then(|| model.name(before))
                .flatten()
                .map(str::to_string)
        })
        .collect();
    Some(Step {
        name,
        typed_by: typed.map(|ty| ws.qualified_name_of(ty)),
        after,
    })
}

/// The two sides a connection joins, and what it is called.
///
/// A flow is a connection too, and is carried on its own as what passes
/// rather than as what is joined, so it is left out here.
fn connection(ws: &mut Workspace, child: ElementId) -> Option<Connection> {
    let of = ws.model().type_of(child);
    let model = ws.model();
    let ends = joined(model, child)?;
    let name = model.name(child).map(str::to_string);
    Some(Connection {
        name,
        connected_by: of.map(|it| ws.qualified_name_of(it)),
        from: ends.0,
        to: ends.1,
        documentation: ws.documentation_of(child),
    })
}

/// The two ends of a connector, as the model names them: the members it
/// marks as ends, which is not the same as the features it owns --
/// `connect a.b to c.d` owns two, and neither is found by metaclass.
fn joined(model: &sysml_model::Model, child: ElementId) -> Option<(String, String)> {
    let ends: Vec<String> = model
        .owned(child)
        .iter()
        .filter(|&&end| model.declared_flag(end, "isEnd"))
        .filter_map(|&end| {
            let reached = sysml_model::end_reaches(model, end);
            let named: Vec<&str> = reached.iter().filter_map(|&it| model.name(it)).collect();
            (!named.is_empty()).then(|| named.join("."))
        })
        .collect();
    let [from, to] = &ends[..] else { return None };
    Some((from.clone(), to.clone()))
}

fn flow(ws: &Workspace, child: ElementId) -> Option<Flow> {
    let model = ws.model();
    if !model.kind(child).is_a(ElementKind::FlowUsage) {
        return None;
    }
    let (from, to) = joined(model, child)?;
    Some(Flow { from, to })
}

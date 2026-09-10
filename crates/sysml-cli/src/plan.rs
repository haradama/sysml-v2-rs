//! What a model implies for code, in no language in particular.
//!
//! `generate_rust` answers this question in Rust and hands back Rust.
//! Somebody writing the same model in Python, Kotlin or C gets nothing
//! from that, and asking a language model to read the SysML itself puts
//! it back to guessing at exactly the things it cannot guess: whether
//! `wheels : Wheel[4]` is an array of four or a list of any number,
//! whether a `part` is owned or referred to, what `ISQ::MassValue`
//! bottoms out in, which of a `variation`'s variants there are.
//!
//! So the answer is stated once, as data, and whoever writes the code --
//! a person, or a model with a language this toolchain has never heard
//! of -- reads it rather than the model. Nothing here decides anything
//! about a language: no identifier is spelled, no container is named, no
//! type is mapped. It says what the model says, resolved, with the
//! things a code generator must ask about made explicit.
//!
//! It lives here because the two things that read it -- the MCP server
//! and `sysml plan` -- live here. A second crate reading it would be the
//! reason to move it into one of its own.

use serde::Serialize;

use sysml_model::{ElementId, ElementKind, Role, Value};
use sysml_semantics::Workspace;

/// Everything a generator would have to write.
#[derive(Serialize)]
pub struct Plan {
    /// Each definition the model declares, in declaration order.
    pub definitions: Vec<Definition>,
    /// What a package declares outright rather than inside a definition:
    /// `attribute ledPinNumber : Integer = 13;`. These are not
    /// definitions and were not planned, so a default that named one --
    /// `attribute pin = ledPinNumber;` -- named something the plan did
    /// not contain, and whoever read it had to invent the number.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub constants: Vec<Feature>,
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
    /// A model says things about a definition that this toolchain has no
    /// opinion about, and the commonest of them is where the thing
    /// already exists: `@code { :>> writtenIn = "rust"; :>> path =
    /// "crate::hal::Gpio"; }` says
    /// this port *is* a type somebody has written, and the actions whose
    /// paths begin the same way are its methods. Read without it, a port
    /// with no features is an empty record and the actions belong to
    /// nothing -- which is the model's own account of what connects
    /// them, dropped.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// Whether the model forbids instances of it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub is_abstract: bool,
    /// What the model declares it specializes, qualified, and only
    /// that: the direct parent rather than everything above it, and not
    /// the standard library specialization every definition gets whether
    /// or not it asks. A reader handed the whole closure has to work out
    /// which of them is the parent, and cannot where the answer is a
    /// library name it was told to ignore.
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
    /// The steps of an action, in the order the model puts them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
    /// What passes between them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub flows: Vec<Flow>,
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
    /// Whether the whole owns it (`part`) or merely refers to it
    /// (`ref part`) -- which decides ownership, copying and lifetime in
    /// every language that has an opinion about them. Said only of what
    /// a whole can be made of: an attribute is a value and a value is
    /// owned by nobody.
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
    Plan {
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
        // An attribute definition with a primitive under it and nothing
        // of its own is that primitive under another name:
        // `attribute def Millis :> Integer;`. Written as a record it
        // becomes an empty class holding nothing, which is a thing the
        // model does not have.
        ElementKind::AttributeDefinition if under && bare => "value",
        // A port is a place a thing connects, not a thing with fields,
        // and a requirement is a question asked of a model rather than
        // anything to build. `record` covered all five of these, which
        // made the field that says what to write in code the less
        // informative of the two.
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
                    .filter(|&child| ws.model().member_role(child) == Some(Role::Variant))
                    .collect(),
            };
            mine.into_iter()
                .filter_map(|child| variant(ws, child))
                .collect()
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
    if model.name(child).is_none() && redefined(model, child).is_none() {
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
    )
}

fn feature(ws: &mut Workspace, child: ElementId) -> Feature {
    let typed = ws
        .model()
        .type_of(child)
        .or_else(|| redefined(ws.model(), child).and_then(|it| ws.model().type_of(it)));
    let (lower, upper) = bounds(ws, child);
    let borrowed = redefined(ws.model(), child).and_then(|it| ws.model().name(it));
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
        // `declared_flag` rather than `flag`: a usage that says nothing
        // about ordering or uniqueness is not a usage that is neither,
        // it is one the specification answers for -- and a generator
        // that reads `unique: false` off silence writes a list where the
        // model meant a set.
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
    let model = ws.model();
    let Some(Value::Ref(range)) = model.get(usage, "multiplicity") else {
        return (1, Some(1));
    };
    let bound = |name: &str| -> Option<Option<i64>> {
        let Some(Value::Ref(bound)) = model.maybe(*range, name) else {
            return None;
        };
        if model.kind(*bound) == ElementKind::LiteralInfinity {
            return Some(None);
        }
        match model.maybe(*bound, "value") {
            Some(Value::Int(int)) => Some(Some(*int)),
            _ => None,
        }
    };
    // `[4]` is a bound on its own, which is both ends at once
    if let Some(exactly) = bound("bound") {
        return (exactly.unwrap_or(0), exactly);
    }
    (
        bound("lowerBound").flatten().unwrap_or(0),
        bound("upperBound").unwrap_or(None),
    )
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
        if let Some(name) = ws.model().name(next) {
            let found = match name {
                "Real" => Some("Real"),
                "Integer" => Some("Integer"),
                "Natural" => Some("Natural"),
                "Positive" => Some("Positive"),
                "Boolean" => Some("Boolean"),
                "String" => Some("String"),
                _ => None,
            };
            if found.is_some() {
                return found;
            }
        }
        queue.extend(ws.supertypes(next));
    }
    None
}

/// The `= value` a usage was declared with.
fn given(ws: &Workspace, usage: ElementId) -> Option<Given> {
    let model = ws.model();
    let membership = model
        .owned(usage)
        .iter()
        .copied()
        .find(|&child| model.kind(child) == ElementKind::FeatureValue)?;
    let Some(Value::Ref(expression)) = model.maybe(membership, "value") else {
        return None;
    };
    let literal = match model.maybe(*expression, "value") {
        Some(Value::Real(real)) => serde_json::json!(real),
        Some(Value::Int(int)) => serde_json::json!(int),
        Some(Value::Bool(flag)) => serde_json::json!(flag),
        Some(Value::String(text)) => serde_json::json!(text),
        _ => return written(model, *expression).map(Given::Expression),
    };
    Some(Given::Literal(literal))
}

/// What an inherited feature this one redefines is called.
fn redefines(ws: &mut Workspace, usage: ElementId) -> Option<String> {
    let redefined = redefined(ws.model(), usage)?;
    Some(ws.qualified_name_of(redefined))
}

/// What the model itself declares a definition specializes.
///
/// [`Workspace::supertypes`] walks the whole way up and includes what
/// the library implies, which is the right answer to "what is this a
/// kind of" and the wrong one to "what does this inherit from": every
/// definition specializes something of the library's whether it says so
/// or not, and the parent is then one name among several with nothing to
/// mark it.
fn declared_parents(ws: &mut Workspace, id: ElementId) -> Vec<String> {
    let model = ws.model();
    let written: Vec<ElementId> = model
        .owned(id)
        .iter()
        .copied()
        .filter(|&child| !model.flag(child, "isImplied"))
        .filter_map(|child| match model.kind(child) {
            ElementKind::Subclassification => model.maybe(child, "superclassifier")?.as_id(),
            ElementKind::Subsetting => model.maybe(child, "subsettedFeature")?.as_id(),
            _ => None,
        })
        .collect();
    written
        .into_iter()
        .map(|up| ws.qualified_name_of(up))
        .collect()
}

/// What the model annotates an element with.
///
/// A metadata usage sets its properties by redefining them --
/// `:>> path = "crate::hal::Gpio";` -- so what it says is read off the
/// redefinition and the value beside it.
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
                    let redefined = redefined(ws.model(), setting)?;
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
                .and_then(|it| written(model, it))
        })
        .collect()
}

/// The inherited feature a usage redefines.
fn redefined(model: &sysml_model::Model, usage: ElementId) -> Option<ElementId> {
    model.owned(usage).iter().copied().find_map(|child| {
        (model.kind(child) == ElementKind::Redefinition)
            .then(|| model.redefined_feature(child))
            .flatten()
    })
}

/// The state a machine starts in.
///
/// The model writes it as `entry; then dark;` -- an entry action with a
/// succession out of it -- so it is neither the first state declared nor
/// anything a state says about itself.
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
        match ends[..] {
            [from, to] if from == entry => model.name(to).map(str::to_string),
            _ => None,
        }
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
    let model = ws.model();
    model
        .owned(id)
        .iter()
        .copied()
        .filter(|&child| {
            model.kind(child) == ElementKind::Expression
                && model.member_role(child) == Some(Role::Result)
        })
        .find_map(|child| written(model, child))
}

/// The text the model wrote for an expression, off the textual
/// representation the builder keeps beside it.
fn written(model: &sysml_model::Model, expression: ElementId) -> Option<String> {
    model.owned(expression).iter().find_map(|&child| {
        (model.kind(child) == ElementKind::TextualRepresentation)
            .then(|| model.maybe(child, "body"))
            .flatten()?
            .as_str()
            .map(str::to_string)
    })
}

fn variant(ws: &mut Workspace, child: ElementId) -> Option<Variant> {
    let typed = ws.model().type_of(child);
    Some(Variant {
        name: ws.model().name(child)?.to_string(),
        typed_by: typed.map(|ty| ws.qualified_name_of(ty)),
        documentation: ws.documentation_of(child),
    })
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
        Some(Value::RefList(guards)) => guards.first().and_then(|&it| written(model, it)),
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
    let mut after = Vec::new();
    for &other in beside {
        let model = ws.model();
        if !model.kind(other).is_a(ElementKind::SuccessionAsUsage) {
            continue;
        }
        let ends: Vec<ElementId> = model
            .owned(other)
            .iter()
            .filter(|&&end| model.kind(end) == ElementKind::Feature)
            .filter_map(|&end| sysml_model::end_reaches(model, end).last().copied())
            .collect();
        if let [before, next] = ends[..] {
            if next == child {
                if let Some(earlier) = model.name(before) {
                    after.push(earlier.to_string());
                }
            }
        }
    }
    Some(Step {
        name,
        typed_by: typed.map(|ty| ws.qualified_name_of(ty)),
        after,
    })
}

fn flow(ws: &Workspace, child: ElementId) -> Option<Flow> {
    let model = ws.model();
    if !model.kind(child).is_a(ElementKind::FlowUsage) {
        return None;
    }
    // The ends of a flow are the members it marks as ends, which is not
    // the same as the plain features it owns: `flow a.b to c.d` owns
    // two, and reading them by metaclass finds neither.
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
    Some(Flow {
        from: from.clone(),
        to: to.clone(),
    })
}

//! What a model implies for code, in no language in particular.
//!
//! `sysml rustgen` answers this in Rust and hands back Rust. Whoever
//! writes Python or C gets nothing from that, and reading the SysML
//! instead means guessing at what it cannot see: whether `wheels :
//! Wheel[4]` is an array or a list, whether a `part` is owned or
//! referred to, what `ISQ::MassValue` bottoms out in.
//!
//! So it is stated once, as data. Nothing here decides anything about a
//! language: no identifier is spelled, no container named, no type
//! mapped. A [`Plan`] is what `sysml plan` prints and what `sysml
//! codegen` hands a generator on standard input -- a program for any
//! language that reads the JSON and writes the code, found on `PATH` the
//! way Doxygen finds a filter. The mirror of [`crate::surface`], which
//! takes code in any language the other way; [`crate::generate`] is the
//! generator that ships, and reads the model itself.
//!
//! ```
//! let mut ws = sysml_semantics::Workspace::new();
//! let file = ws.add_file("car.sysml", "part def Car { attribute mass : Real; }\n");
//! ws.resolve_all();
//! let roots = ws.file_roots(file).to_vec();
//! let plan = sysml_rust::plan::of(&mut ws, &roots);
//! assert_eq!(plan.definitions[0].of, "Car");
//! assert_eq!(plan.definitions[0].shape, "record");
//! // and back from the JSON a generator reads
//! let json = serde_json::to_string(&plan).unwrap();
//! let read: sysml_rust::plan::Plan = serde_json::from_str(&json).unwrap();
//! assert_eq!(read, plan);
//! ```

use serde::{Deserialize, Serialize};

use sysml_model::{ElementId, ElementKind, Role, Value};
use sysml_semantics::Workspace;

/// Everything a generator would have to write.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constants: Vec<Feature>,
}

/// Whether the model resolves, and what did not.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Checked {
    /// Whether every name in the planned files found something, and
    /// every one of them parsed.
    pub ok: bool,
    /// What the parser could not read. A file that does not parse is
    /// missing whole declarations rather than one type, so a plan built
    /// from it says nothing about what is not there.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub syntax: Vec<String>,
    /// The names that did not, as the model wrote them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved: Vec<String>,
}

/// One definition, and what shape it has in code.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Definition {
    /// Its qualified name, as the model spells it.
    pub of: String,
    /// The short name it also answers to, where it has one:
    /// `requirement def <'S.2'> PerceptiblePeriod` is `S.2` to everything
    /// that refers to it, and a reader given only the long name cannot
    /// follow what points at it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_name: Option<String>,
    /// The metaclass, for a reader that knows SysML.
    pub kind: String,
    /// What it is in code, for one that does not: `record`, `value`,
    /// `enumeration`, `variation`, `abstract`, `port`, `requirement`,
    /// `state machine`, `function` or `behaviour`.
    pub shape: String,
    /// What the definition itself bottoms out in, where it is a value
    /// with a primitive under it. `attribute def Millis :> Integer;` is
    /// an integer with a name, and writing an empty class for it is
    /// writing something the model does not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primitive: Option<String>,
    /// What the model says it is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    /// What the model annotates it with, verbatim.
    ///
    /// Most usefully where the thing already exists: `@code { :>>
    /// writtenIn = "rust"; :>> path = "crate::hal::Gpio"; }` says this
    /// port *is* a written type, and the actions whose paths begin the
    /// same way are its methods. Without it they belong to nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// Whether the model forbids instances of it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_abstract: bool,
    /// What the model declares it specializes, qualified: the direct
    /// parent, not everything above it and not the library
    /// specialization every definition gets whether it asks or not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub specializes: Vec<String>,
    /// Its own features: attributes, parts, ports, parameters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<Feature>,
    /// The members of an enumeration.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    /// What a requirement is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Feature>,
    /// What a requirement demands, as the model wrote it. Without these
    /// a requirement arrives as a parameter list and a sentence, and
    /// whoever writes the check invents it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    /// What it takes as given before it demands anything.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assumes: Vec<String>,
    /// The variants of a variation: exactly one of these, not all of
    /// them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<Variant>,
    /// The states of a state definition, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<String>,
    /// The one it starts in, which the model writes as `entry; then x;`
    /// and is not the same as the first one declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial: Option<String>,
    /// What happens on the way in, throughout, and on the way out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_entry: Option<String>,
    /// What it does for as long as it is in the machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub during: Option<String>,
    /// What it does on the way out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_exit: Option<String>,
    /// What moves it between them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transitions: Vec<Transition>,
    /// What a calculation or behaviour hands back, where it declares
    /// one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returns: Option<Feature>,
    /// What a calculation or constraint works out, as the model wrote
    /// it. Translating it is the reader's; this toolchain does not
    /// pretend to have done it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    /// The behaviours it carries out, which is what makes a definition
    /// something that *does* anything: a part that performs an action
    /// is written, in a language with methods, as a method.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub performs: Vec<Performed>,
    /// The steps of an action, in the order the model puts them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
    /// What follows what among them, the guards of a `decide`'s ways out
    /// included: the order and the branching the steps' own
    /// [`after`](Step::after) lists cannot say.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub successions: Vec<Succession>,
    /// What passes between them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flows: Vec<Flow>,
    /// What is joined to what inside it. `connect pump to tank;` is the
    /// whole of why the two parts are there together.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<Connection>,
}

/// One feature of a definition, with everything a generator must ask
/// about it and nothing about how any language would write it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Feature {
    /// What the model calls it. A `return : Real;` has no name and does
    /// not need one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// The metaclass: `AttributeUsage`, `PartUsage`, `PortUsage`, and so
    /// on. What kind of thing it is, which decides more than its type
    /// does -- a part is a thing the whole is made of, a port is a place
    /// it connects.
    pub kind: String,
    /// What types it, qualified.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub typed_by: Option<String>,
    /// Whether it is one of the two sides a connection relates rather
    /// than something the definition holds. `connection def Pipe { end
    /// source : Pump; end target : Tank; }` is a pipe between a pump and
    /// a tank, not a pipe with a pump inside it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub end: bool,
    /// What that type bottoms out in, where it bottoms out in one of the
    /// standard library's primitives: `Real`, `Integer`, `Natural`,
    /// `Positive`, `Boolean`, `String`. `ISQ::MassValue` is a `Real` --
    /// the number of [`unit`](Self::unit)s it holds -- and no amount of
    /// reading its name says so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primitive: Option<String>,
    /// What that number counts, where the type is a scalar quantity:
    /// `kg`, `m/s`, `kg*m^2/s^3`. Always the SI coherent unit, whatever
    /// unit the model wrote a value in, so a generator that writes the
    /// number alone writes the same number for `2500 [g]` and `2.5
    /// [kg]`. Read off the dimension the library gives the quantity, and
    /// absent for one that counts in ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// How many of it there are. Absent is one of it, which is what
    /// everything is unless the model says otherwise.
    #[serde(default, skip_serializing_if = "Multiplicity::is_one")]
    pub multiplicity: Multiplicity,
    /// Whether the whole owns it (`part`) or refers to it (`ref part`).
    /// Said only of what a whole can be made of: a value is owned by
    /// nobody.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composite: Option<bool>,
    /// Whether the collection keeps its order, and whether it may hold
    /// the same thing twice: a list, a set, or neither. Said only of
    /// collections, since one of a thing is in no order and is not two
    /// of anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordered: Option<bool>,
    /// Whether it may hold the same thing twice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unique: Option<bool>,
    /// Which way a parameter goes: `in`, `out` or `inout`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    /// The `= value` it was declared with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Given>,
    /// The inherited feature it redefines, qualified: the same thing
    /// said again, not a second one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redefines: Option<String>,
    /// What the model says it is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}

/// How many of it there are.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Multiplicity {
    /// The fewest there may be.
    pub lower: i64,
    /// `null` is `*`: any number of them.
    pub upper: Option<i64>,
}

impl Multiplicity {
    /// One of the thing, which is what everything is unless the model
    /// says otherwise -- and so what is not worth saying.
    pub fn is_one(&self) -> bool {
        self.lower == 1 && self.upper == Some(1)
    }
}

impl Default for Multiplicity {
    /// One of it: what a plan that says nothing means.
    fn default() -> Multiplicity {
        Multiplicity {
            lower: 1,
            upper: Some(1),
        }
    }
}

/// The JSON schema of a [`Plan`], for whoever writes a generator.
#[cfg(feature = "schema")]
pub fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Plan)).expect("a schema is JSON")
}

/// A value the model declared, whether or not this toolchain understood
/// it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Given {
    /// A literal, as JSON: a number, a string, a boolean.
    Literal(serde_json::Value),
    /// Anything else, as the model wrote it.
    Expression(String),
}

/// One `@name { ... }` a definition carries, as the model wrote it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Annotation {
    /// What is being said, by the name of the metadata definition:
    /// `rust`, `Safety`, whatever the model declared.
    pub of: String,
    /// What it sets, by property name.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub says: std::collections::BTreeMap<String, Given>,
}

/// One variant of a variation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Variant {
    /// What the model calls it.
    pub name: String,
    /// What it carries, where it carries something.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub typed_by: Option<String>,
    /// What the model says it is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}

/// One transition of a state machine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Transition {
    /// What the model calls it -- or, for one it leaves unnamed,
    /// `{from}_to_{to}`, the name `sysml rustgen` gives it too, numbered
    /// `_2`, `_3` where another transition already answers to that.
    pub name: String,
    /// Whether [`name`](Self::name) is that derived one rather than the
    /// model's: nothing in the model refers to it by it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unnamed: bool,
    /// The state it leaves.
    pub from: String,
    /// The state it arrives in.
    pub to: String,
    /// What it waits for, where it waits for something.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<Trigger>,
    /// The condition, as the model wrote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<String>,
    /// Whether the model says something happens on the way. What it is
    /// is the reader's to write.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub effect: bool,
}

/// The event a transition waits for, and what it carries.
///
/// `accept s : Sig` waits for a `Sig` and binds it to `s`; `accept Go`
/// waits for a `Go` and binds it to nothing; `accept when level > 3`
/// waits for a condition to become true, and `accept after 5 [s]` /
/// `accept at t` for a time -- neither of those carries anything.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Trigger {
    /// What kind of event: `signal` (something arrives), `when` (a
    /// change: the condition becomes true), `after` (a relative time
    /// passes) or `at` (an absolute time is reached).
    #[serde(default = "signal")]
    pub kind: String,
    /// The name the payload is bound to, where the model binds it to
    /// one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// What the payload is.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub typed_by: Option<String>,
    /// The condition or the time of a `when`, `after` or `at`, as the
    /// model wrote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
}

fn signal() -> String {
    "signal".to_string()
}

/// One behaviour a definition carries out. What it performs is named
/// rather than described again: the behaviour is a definition of its
/// own wherever the model declares it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Performed {
    /// What the model calls it here.
    pub name: String,
    /// The behaviour performed, qualified.
    #[serde(rename = "of", default, skip_serializing_if = "Option::is_none")]
    pub performed: Option<String>,
    /// What the model says it is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}

/// One step of an action: a subaction, or one of the control nodes and
/// structured actions that decide which subaction runs when.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Step {
    /// What the model calls it -- or, for one it leaves unnamed, its
    /// [`kind`](Self::kind), numbered `_2`, `_3` clear of the other
    /// steps beside it.
    pub name: String,
    /// Whether [`name`](Self::name) is that derived one rather than the
    /// model's.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unnamed: bool,
    /// What kind of step: `action` (a subaction), `perform`, `accept`,
    /// `send`, `assign`, `terminate`; the control nodes `decide` (one way
    /// out is taken, chosen by the guards of its
    /// [`successions`](Definition::successions)), `merge` (any one way
    /// in is enough), `fork` (every way out is taken) and `join` (every
    /// way in must arrive); and the structured `if`, `while`, `loop` and
    /// `for`, whose [`body`](Self::body) is a block of steps of its own.
    #[serde(default = "action")]
    pub kind: String,
    /// What it performs.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub typed_by: Option<String>,
    /// The steps a succession leads from into this one, guarded or not:
    /// every one of them a step of the same block. Which of them must
    /// finish -- all, for an action or a `join`; the one that arrives,
    /// for a `merge` -- is the step's kind's to say, and which guard
    /// opens which way the [`successions`](Definition::successions)'.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
    /// The condition of an `if` or a `while`, as the model wrote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    /// The `until` a loop stops at, checked after each pass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// What an `if` does when its condition holds, or a loop does each
    /// pass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Block>,
    /// What an `if` does otherwise. An `else if` is a block holding the
    /// one `if` step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otherwise: Option<Block>,
}

fn action() -> String {
    "action".to_string()
}

/// The steps of the body of an `if` or a loop, and how they follow one
/// another: the same as a definition says of its own.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Block {
    /// What the body is called, where the model names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its steps, in the order the model puts them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
    /// What follows what among them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub successions: Vec<Succession>,
    /// What passes between them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flows: Vec<Flow>,
}

/// One step following another: `first a then b`, or -- out of a
/// `decide` -- `if guard then b`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Succession {
    /// The step it leads from, by its [`name`](Step::name).
    pub from: String,
    /// The step it leads to.
    pub to: String,
    /// What must hold for it to be taken, as the model wrote it. A way
    /// out of a `decide` with none is its `else`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<String>,
}

/// Two of a definition's parts, joined.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Connection {
    /// What the model calls it, where it calls it anything: `connect a
    /// to b;` names nothing, and the joining is the point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// What kind of connection it is, qualified, where the model says.
    #[serde(rename = "of", default, skip_serializing_if = "Option::is_none")]
    pub connected_by: Option<String>,
    /// The two sides, as dotted paths from the definition.
    pub from: String,
    /// The other side.
    pub to: String,
    /// What the model says it is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}

/// One thing passing from one place to another.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
        // A metadata definition is the vocabulary an annotation is
        // written in -- `metadata def code { attribute path : String; }`
        // -- and a conjugated port is the other face of one the model
        // declared, implied beside it and named `~Port`. Neither is a
        // thing code is written for, and each was planned as a record:
        // a generator wrote a class `code`, and a class it could not
        // even name.
        kind if kind.is_a(ElementKind::Metaclass)
            || kind == ElementKind::ConjugatedPortDefinition =>
        {
            return None
        }
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
    // a part's `perform` is what it does, said in `performs`; an
    // action's is one of its steps
    let block = block(
        ws,
        id,
        ws.model().kind(id).is_a(ElementKind::ActionDefinition),
    );
    let says_nothing_by_being_abstract = matches!(shape, "enumeration" | "variation");
    Definition {
        of: ws.qualified_name_of(id),
        short_name: ws
            .model()
            .maybe(id, "declaredShortName")
            .and_then(Value::as_str)
            .map(str::to_string),
        kind: ws.model().kind(id).name().to_string(),
        shape: shape.to_string(),
        primitive: primitive_of(ws, id).map(str::to_string),
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
        transitions: transitions(ws, &owned, shape == "state machine"),
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
        steps: block.steps,
        successions: block.successions,
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
    let typed = crate::generate::held_type(ws.model(), child);
    let (lower, upper) = bounds(ws, child);
    let borrowed = sysml_model::redefined(ws.model(), child).and_then(|it| ws.model().name(it));
    Feature {
        name: ws
            .model()
            .name(child)
            .or(borrowed)
            .unwrap_or_default()
            .to_string(),
        kind: ws.model().kind(child).name().to_string(),
        typed_by: typed.map(|ty| ws.qualified_name_of(ty)),
        primitive: typed
            .and_then(|ty| primitive_of(ws, ty))
            .map(str::to_string),
        unit: typed
            .filter(|&ty| crate::quantity::is_scalar_quantity(ws.model(), ty))
            .and_then(|ty| crate::quantity::si_unit(ws.model(), ty)),
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
/// specializes. A scalar quantity is a `Real`, though not by
/// specializing one: `ISQ::MassValue` specializes `NumericalValue`,
/// which is above `Real`, and *has* a `num : Real` -- the number of SI
/// coherent units it is.
fn primitive_of(ws: &mut Workspace, ty: ElementId) -> Option<&'static str> {
    if crate::quantity::is_scalar_quantity(ws.model(), ty) {
        return Some("Real");
    }
    let mut seen = std::collections::HashSet::new();
    let mut queue = vec![ty];
    while let Some(next) = queue.pop() {
        if !seen.insert(next) {
            continue;
        }
        if let Some(found) = sysml_model::library_primitive(ws.model(), next) {
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
///
/// What it redefines may be a redefinition itself, declaring no name and
/// answering to the one it borrows -- `attribute :>> c = 2;` in `Sub` is
/// `Sub::c` -- which the qualified name reads off the model as `?`.
fn redefines(ws: &mut Workspace, usage: ElementId) -> Option<String> {
    let model = ws.model();
    let redefined = sysml_model::redefined(model, usage)?;
    if model.name(redefined).is_some() {
        return Some(ws.qualified_name_of(redefined));
    }
    let name = model.effective_name(redefined)?.to_string();
    let owner = model
        .owner(redefined)
        .map(|owner| ws.qualified_name_of(owner))
        .filter(|owner| !owner.is_empty());
    Some(owner.map_or(name.clone(), |owner| format!("{owner}::{name}")))
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

/// The state a machine starts in, by name. See
/// [`crate::generate::initial_state`], which the generated `initial()`
/// answers from too, so the plan and the Rust cannot disagree.
fn initial_state(ws: &Workspace, owned: &[ElementId]) -> Option<String> {
    let model = ws.model();
    crate::generate::initial_state(model, owned)
        .and_then(|state| model.name(state))
        .map(str::to_string)
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

/// The transitions of a state definition, each with a name: the model's,
/// or `{from}_to_{to}` for one it leaves unnamed -- numbered clear of the
/// names the model gave, which keep theirs -- as `sysml rustgen` calls it.
///
/// Outside a state machine an unnamed transition is a guarded succession
/// -- `if flag then logA;` out of a `decide` -- and is planned as one of
/// the [`successions`](Definition::successions), not as a transition.
///
/// A transition written inside a state's body -- `state off { accept Go
/// then on; }` -- names only where it goes, and leaves from the state it
/// is written in. It was left out of the plan, as though the machine had
/// no way out of `off`; it is planned with that state as its source.
fn transitions(ws: &mut Workspace, owned: &[ElementId], machine: bool) -> Vec<Transition> {
    let mut written: Vec<(ElementId, Option<ElementId>)> =
        owned.iter().map(|&child| (child, None)).collect();
    if machine {
        let model = ws.model();
        for &state in owned {
            if model.kind(state) != ElementKind::StateUsage || model.name(state).is_none() {
                continue;
            }
            written.extend(
                model
                    .owned(state)
                    .iter()
                    .filter(|&&child| model.kind(child) == ElementKind::TransitionUsage)
                    .map(|&child| (child, Some(state))),
            );
        }
    }
    let mut planned: Vec<Transition> = written
        .into_iter()
        .filter_map(|(child, inside)| transition(ws, child, inside))
        .filter(|it| machine || !it.unnamed)
        .collect();
    let mut taken: std::collections::HashSet<String> = planned
        .iter()
        .filter(|it| !it.unnamed)
        .map(|it| it.name.clone())
        .collect();
    for it in planned.iter_mut().filter(|it| it.unnamed) {
        let base = format!("{}_to_{}", it.from, it.to);
        let mut name = base.clone();
        let mut at = 1;
        while !taken.insert(name.clone()) {
            at += 1;
            name = format!("{base}_{at}");
        }
        it.name = name;
    }
    planned
}

/// One transition; `inside` is the state whose body it is written in,
/// which is where it leaves from when it names only where it goes.
fn transition(
    ws: &mut Workspace,
    usage: ElementId,
    inside: Option<ElementId>,
) -> Option<Transition> {
    let model = ws.model();
    if model.kind(usage) != ElementKind::TransitionUsage {
        return None;
    }
    // named, where it is unnamed, once its ends are known
    let named = model.name(usage).map(str::to_string);
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
        .filter(|&&child| crate::connector_end(model, child))
        .filter_map(|&child| sysml_model::end_reaches(model, child).last().copied())
        .collect();
    let (from, to) = match (&ends[..], inside) {
        (&[from, to], _) => (from, to),
        (&[to], Some(state)) => (state, to),
        _ => return None,
    };
    let accept = match model.get(usage, "triggerAction") {
        Some(Value::RefList(triggers)) => triggers.first().copied(),
        _ => None,
    };
    let on = accept.and_then(|accept| trigger(ws, accept));
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
        unnamed: named.is_none(),
        name: named.unwrap_or_default(),
        from: model.name(from)?.to_string(),
        to: model.name(to)?.to_string(),
        on,
        guard,
        effect,
    })
}

/// What an `accept` waits for: what its payload parameter is typed by
/// and bound to, and -- for `when`, `after` and `at` -- the kind of event
/// and the expression the model wrote, which is the payload's value.
fn trigger(ws: &mut Workspace, accept: ElementId) -> Option<Trigger> {
    let model = ws.model();
    let waits = sysml_model::payload_parameter(model, accept)?;
    let typed = model.type_of(waits);
    let name = model.name(waits).unwrap_or_default().to_string();
    let invoked = model
        .owned(waits)
        .iter()
        .filter(|&&child| model.kind(child) == ElementKind::FeatureValue)
        .filter_map(|&value| model.maybe(value, "value").and_then(Value::as_id))
        .find(|&value| model.kind(value) == ElementKind::TriggerInvocationExpression);
    let (kind, expression) = match invoked {
        Some(invocation) => {
            let kind = model
                .maybe(invocation, "kind")
                .and_then(Value::as_str)?
                .to_string();
            // the text is the argument's, which a `when` holds behind a
            // reference to it: the outermost thing below that was written
            let written = sysml_model::expression_text(model, invocation).or_else(|| {
                model.descendants(invocation).into_iter().find_map(|below| {
                    sysml_model::expression_text(model, below).or_else(|| {
                        // a literal is held as its value, not its text
                        if !model.kind(below).name().starts_with("Literal") {
                            return None;
                        }
                        literal_text(model.maybe(below, "value")?)
                    })
                })
            });
            (kind, written)
        }
        None => (signal(), None),
    };
    Some(Trigger {
        kind,
        name,
        typed_by: typed.map(|ty| ws.qualified_name_of(ty)),
        expression,
    })
}

/// What kind of step a member of a behaviour is, where it is one at all.
fn step_kind(model: &sysml_model::Model, child: ElementId) -> Option<&'static str> {
    // a parameter is what the behaviour is handed, and an entry, do or
    // exit action the machinery of a state, said where it belongs
    if model.direction(child).is_some()
        || matches!(
            model.member_role(child),
            Some(Role::Entry | Role::Do | Role::Exit)
        )
    {
        return None;
    }
    Some(match model.kind(child) {
        ElementKind::ActionUsage => "action",
        ElementKind::PerformActionUsage => "perform",
        ElementKind::AcceptActionUsage => "accept",
        ElementKind::SendActionUsage => "send",
        ElementKind::AssignmentActionUsage => "assign",
        ElementKind::TerminateActionUsage => "terminate",
        ElementKind::DecisionNode => "decide",
        ElementKind::MergeNode => "merge",
        ElementKind::ForkNode => "fork",
        ElementKind::JoinNode => "join",
        ElementKind::IfActionUsage => "if",
        ElementKind::ForLoopActionUsage => "for",
        // `loop` holds an empty parameter where `while` holds its
        // condition
        ElementKind::WhileLoopActionUsage => match loop_clauses(model, child).first() {
            Some(&condition) if model.kind(condition).is_a(ElementKind::Expression) => "while",
            _ => "loop",
        },
        _ => return None,
    })
}

/// The clauses of an `if` or loop action, in the order the model holds
/// them: the condition, the body, and the `else` or the `until`.
fn loop_clauses(model: &sysml_model::Model, structured: ElementId) -> Vec<ElementId> {
    model
        .owned(structured)
        .iter()
        .copied()
        .filter(|&child| model.direction(child) == Some("in"))
        .collect()
}

/// The text of a condition clause.
fn clause_text(model: &sysml_model::Model, clause: ElementId) -> Option<String> {
    if !model.kind(clause).is_a(ElementKind::Expression) {
        return None;
    }
    sysml_model::expression_text(model, clause)
}

/// What a literal's value reads as in the text of a condition.
fn literal_text(value: &Value) -> Option<String> {
    value
        .as_real()
        .map(|it| it.to_string())
        .or_else(|| value.as_int().map(|it| it.to_string()))
        .or_else(|| value.as_bool().map(|it| it.to_string()))
        .or_else(|| value.as_str().map(|it| format!("{it:?}")))
}

/// The steps a behaviour, or one body inside it, is made of, and the
/// successions between them. Every name a succession or an `after`
/// carries is the name of a step listed beside it: one reaching outside
/// the block -- `first start then a`, or into another body -- orders
/// nothing listed here, and is left out rather than named as a step
/// that is not one.
fn block(ws: &mut Workspace, container: ElementId, performs_are_steps: bool) -> Block {
    let model = ws.model();
    let owned = model.owned(container).to_vec();
    let nodes: Vec<(ElementId, &'static str)> = owned
        .iter()
        .filter_map(|&child| Some((child, step_kind(model, child)?)))
        .filter(|&(_, kind)| performs_are_steps || kind != "perform")
        .collect();
    // the model's names first, so that a derived one steps round them
    let mut taken: std::collections::HashSet<String> = nodes
        .iter()
        .filter_map(|&(node, _)| model.effective_name(node).map(str::to_string))
        .collect();
    let names: Vec<(String, bool)> = nodes
        .iter()
        .map(|&(node, kind)| match model.effective_name(node) {
            Some(name) => (name.to_string(), false),
            None => {
                let mut name = kind.to_string();
                let mut at = 1;
                while !taken.insert(name.clone()) {
                    at += 1;
                    name = format!("{kind}_{at}");
                }
                (name, true)
            }
        })
        .collect();
    let named = |id: ElementId| {
        nodes
            .iter()
            .position(|&(node, _)| node == id)
            .map(|at| names[at].0.clone())
    };
    let mut successions = Vec::new();
    for &child in &owned {
        let kind = model.kind(child);
        // a guarded succession is a transition owning the succession it
        // guards
        let (relates, guard) = if kind == ElementKind::TransitionUsage {
            // the builder gives every transition its succession; one
            // without is read as the connector itself, which has no ends
            let relates = model
                .owned(child)
                .iter()
                .copied()
                .find(|&inner| model.kind(inner).is_a(ElementKind::SuccessionAsUsage))
                .unwrap_or(child);
            let guard = match model.get(child, "guardExpression") {
                Some(Value::RefList(guards)) => guards
                    .first()
                    .and_then(|&it| sysml_model::expression_text(model, it)),
                _ => None,
            };
            (relates, guard)
        } else if kind.is_a(ElementKind::SuccessionAsUsage) {
            (child, None)
        } else {
            continue;
        };
        let ends: Vec<ElementId> = model
            .owned(relates)
            .iter()
            .filter(|&&end| crate::connector_end(model, end))
            .filter_map(|&end| sysml_model::end_reaches(model, end).first().copied())
            .collect();
        let [from, to] = ends[..] else { continue };
        if let (Some(from), Some(to)) = (named(from), named(to)) {
            successions.push(Succession { from, to, guard });
        }
    }
    let mut steps = Vec::new();
    for (&(node, kind), (name, unnamed)) in nodes.iter().zip(names) {
        let mut after: Vec<String> = Vec::new();
        for succession in successions.iter().filter(|it| it.to == name) {
            if !after.contains(&succession.from) {
                after.push(succession.from.clone());
            }
        }
        steps.push(step(ws, node, kind, name, unnamed, after));
    }
    Block {
        name: None,
        flows: owned.iter().filter_map(|&child| flow(ws, child)).collect(),
        steps,
        successions,
    }
}

/// One step, the blocks of an `if` or a loop worked out with it.
fn step(
    ws: &mut Workspace,
    node: ElementId,
    kind: &'static str,
    name: String,
    unnamed: bool,
    after: Vec<String>,
) -> Step {
    let typed = ws.model().type_of(node);
    let mut step = Step {
        name,
        unnamed,
        kind: kind.to_string(),
        typed_by: typed.map(|ty| ws.qualified_name_of(ty)),
        after,
        condition: None,
        until: None,
        body: None,
        otherwise: None,
    };
    let model = ws.model();
    let clauses = loop_clauses(model, node);
    match kind {
        "if" | "while" | "loop" => {
            let [condition, body, rest @ ..] = clauses.as_slice() else {
                return step;
            };
            step.condition = clause_text(model, *condition);
            let rest = rest.first().copied();
            if kind == "if" {
                step.body = Some(clause_block(ws, *body));
                step.otherwise = rest.map(|it| clause_block(ws, it));
            } else {
                step.until = rest.and_then(|it| clause_text(model, it));
                step.body = Some(clause_block(ws, *body));
            }
        }
        // `for x in seq { ... }`: the sequence is the clause that is an
        // expression, the body the one that is an action
        "for" => {
            step.condition = clauses.iter().find_map(|&it| clause_text(model, it));
            let body = clauses
                .iter()
                .copied()
                .find(|&it| model.kind(it).is_a(ElementKind::ActionUsage));
            step.body = body.map(|it| clause_block(ws, it));
        }
        _ => {}
    }
    step
}

/// One body of an `if` or a loop as a block. An `else if` is an `if` of
/// its own, so its block is that one step.
fn clause_block(ws: &mut Workspace, clause: ElementId) -> Block {
    let model = ws.model();
    if model.kind(clause) == ElementKind::IfActionUsage {
        let name = model.name(clause).map(str::to_string);
        let unnamed = name.is_none();
        let only = step(
            ws,
            clause,
            "if",
            name.unwrap_or_else(|| "if".to_string()),
            unnamed,
            Vec::new(),
        );
        return Block {
            steps: vec![only],
            ..Block::default()
        };
    }
    let name = model.name(clause).map(str::to_string);
    let mut block = block(ws, clause, true);
    block.name = name;
    block
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

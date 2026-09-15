//! Rust from a resolved SysML model.
//!
//! Two kinds of thing come out of one pass over the model:
//!
//! **Data and structure.** A `part def`, `item def`, `attribute def` or
//! `port def` becomes a struct: attributes and composed parts become
//! fields (multiplicities as containers -- `[*]` a `Vec`, `[0..1]` an
//! `Option`, `[n]` an array), inherited features are flattened in with
//! redefinitions shadowing what they redefine, and a composition cycle is
//! broken with a `Box` at the edge that closes it. Declared values become
//! a `Default` implementation where every field has one. An `enum def` --
//! and a variation with `variant` members -- becomes an enum. An
//! `abstract` definition contributes its features to its subtypes but gets
//! no struct of its own.
//!
//! **Calculations.** A `calc def` becomes a function over its `in`
//! parameters and a `calc` usage of a struct a method over its fields.
//! Result expressions in the simple subset translate as written (numeric
//! literals keep their spelling, so mixed-type arithmetic surfaces as a
//! Rust type error, not a coercion); anything richer keeps its SysML text
//! behind a `todo!`. An `abstract calc def` declares no formula, so it
//! becomes a trait with one method and no default body: what the model
//! leaves open the compiler asks for.
//!
//! **Behaviour against existing APIs.** A port typed by a `port def`
//! carrying a `@code { ... }` binding becomes a generic parameter bound to
//! the real Rust trait, and each `perform`ed bound action a method
//! delegating through that port with the API's own signature: `async`,
//! `Result` and the receiver the binding states. A `state def` becomes a
//! state machine: an enum of states, an enum of the events its transitions
//! accept, a hooks trait carrying guards, effects and entry/exit
//! notifications, and a `step` function over the transition table.
//!
//! What has no generated shape is written into the output as a comment
//! rather than dropped. The output is deterministic and intended to be
//! committed next to the model; regenerating and diffing is the drift
//! check.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use sysml_model::{ElementId, ElementKind, Model, Role, Value};

use crate::binding;

/// What every generated struct and enum derives, where its fields let
/// it. `Default` joins them when the model gave every field a value.
const DERIVED: [&str; 3] = ["Debug", "Clone", "PartialEq"];
use crate::expr::{self, translate, translate_as, Numbers, Translated};

/// The Rust a model implies, and what the model left for a person to
/// write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Generated {
    /// The Rust the model implies, as one module.
    pub rust: String,
    /// Every place the generator stopped short, in the order it met
    /// them. The output says each of these in a comment too -- that is
    /// for whoever reads the file; this is for whoever has to fill the
    /// gap, and a work list is a poor thing to have to find by reading.
    pub open: Vec<Open>,
}

/// One thing the generator did not write, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Open {
    /// What kind of gap it is.
    pub kind: OpenKind,
    /// What in the model it is about, by the name the model gives it.
    pub sysml: String,
    /// What has to be written in Rust, where there is a name for it: the
    /// trait whose method is yours to implement.
    pub rust: Option<String>,
    /// Why the generator stopped short, in words a person can act on.
    pub why: String,
}

/// What kind of gap it is -- which decides who fills it and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenKind {
    /// A definition the model left abstract became a trait with no
    /// default body: the compiler will ask for it.
    Trait,
    /// An expression outside the translated subset kept the model's own
    /// words behind a `todo!`.
    Todo,
    /// A state machine's guards, effects and entry/exit notifications.
    Hooks,
    /// Something with no generated shape at all, said in a comment.
    Skipped,
}

impl OpenKind {
    /// The word for it, for an answer written as JSON.
    pub fn name(self) -> &'static str {
        match self {
            OpenKind::Trait => "trait",
            OpenKind::Todo => "todo",
            OpenKind::Hooks => "hooks",
            OpenKind::Skipped => "skipped",
        }
    }
}

/// What stops code generation outright (a model this generator cannot
/// write faithfully); everything smaller is a comment in the output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RustgenError {
    /// A performed action's binding names no port of the part.
    NoPortForAction {
        /// The part the port is declared on.
        part: String,
        /// The action performed over it.
        action: String,
        /// The Rust path the `@code` metadata binds it to.
        path: String,
    },
}

impl std::fmt::Display for RustgenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RustgenError::NoPortForAction { part, action, path } => write!(
                f,
                "`{part}` performs `{action}` bound to `{path}`, but no port of the part \
                 provides that API"
            ),
        }
    }
}

impl std::error::Error for RustgenError {}

/// Rust source for the definitions under `roots`.
pub fn generate(model: &Model, roots: &[ElementId]) -> Result<Generated, RustgenError> {
    let mut out = String::new();
    writeln!(
        out,
        "// generated by `sysml rustgen` -- DO NOT EDIT\n//\n\
         // Definitions of the model become structs and enums; ports bound to\n\
         // existing Rust APIs become generic parameters, `perform`ed actions\n\
         // methods, and state definitions state machines. What the model\n\
         // leaves abstract becomes a trait for you to implement -- alongside\n\
         // this file, not in it.\n\
         //\n\
         // An expression is spelled the way the model spells it, and a\n\
         // signature carries the parameters the model declares, so the lints\n\
         // that ask for a different spelling have nothing to say about a file\n\
         // nobody edits. What the model declares is written whether or not\n\
         // the caller reaches for it, which is the other thing a lint would\n\
         // otherwise ask about.\n\
         // On one line because that is where `rustfmt` puts it, and a\n\
         // generated file a reader never edits is still a file they run\n\
         // `cargo fmt` over: written the other way, the first thing this\n\
         // generator emits is the first thing that comes back as a diff.\n\
         #![allow(dead_code, clippy::manual_range_contains, clippy::too_many_arguments)]"
    )
    .expect("writing to a String cannot fail");

    let generator = Generator::collect(model, roots);
    for &def in &generator.order {
        generator.definition(def, &mut out)?;
    }
    for (def, why) in &generator.skipped {
        let name = model.name(*def).expect("collected named");
        generator.open(OpenKind::Skipped, name, None, why);
        writeln!(out, "\n// not generated: `{name}` -- {why}")
            .expect("writing to a String cannot fail");
    }
    generator.requirements(roots, &mut out);
    Ok(Generated {
        rust: out,
        open: generator.open.into_inner(),
    })
}

/// How one definition will be generated.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Struct,
    Enum,
    Variation,
    StateMachine,
    /// A `calc def` -- or a `constraint def`, which is one returning a
    /// boolean: a function over its `in` parameters.
    Calculation,
    /// An `action def`: a trait, since a behaviour is not something this
    /// generator can write.
    Action,
    /// Abstract: features flatten into subtypes, no item of its own.
    Flattened,
}

struct Generator<'a> {
    model: &'a Model,
    order: Vec<ElementId>,
    /// Definitions this generator has no shape for, in declaration
    /// order, and why, so that the output says so instead of passing
    /// over them.
    skipped: Vec<(ElementId, String)>,
    shapes: HashMap<ElementId, Shape>,
    /// Generated types that can answer `Default::default()`.
    defaultable: HashSet<ElementId>,
    /// Structs whose fields all support `Debug`/`Clone`/`PartialEq`.
    derivable: HashSet<ElementId>,
    /// Composition edges broken with a `Box` to keep structs finite.
    boxed: HashSet<(ElementId, ElementId)>,
    /// Per struct: its generic parameters and what each composed field
    /// passes along, so parts with API ports compose.
    plans: HashMap<ElementId, Plan>,
    /// Every generated type's fields, worked out once they stop
    /// changing. Empty until then.
    fields: HashMap<ElementId, Option<Vec<Field>>>,
    /// What the generator stopped short of writing. A cell because the
    /// pass writes through `&self`: the alternative is to thread a
    /// second `&mut` beside the output through every one of these
    /// methods, which says nothing the name of this field does not.
    open: RefCell<Vec<Open>>,
}

impl Generator<'_> {
    /// Remember a gap the output is about to say in a comment. The two
    /// are written together at every site so that neither can be added
    /// without the other.
    fn open(
        &self,
        kind: OpenKind,
        sysml: impl Into<String>,
        rust: Option<String>,
        why: impl Into<String>,
    ) {
        self.open.borrow_mut().push(Open {
            kind,
            sysml: sysml.into(),
            rust,
            why: why.into(),
        });
    }

    /// The body a calculation's formula becomes, and whether the model's own
    /// words had to be left inside a `todo!` because they are past the
    /// translated subset.
    ///
    /// A calculation is written twice -- as a free function and as a method on
    /// the type that owns one -- and this is the part that does not differ.
    /// What the model did not say is recorded as an opening at the same
    /// moment, so the list of what a person still has to write cannot fall out
    /// of step with the `todo!`s.
    fn formula_body(
        &self,
        clause: &Option<ValueClause>,
        translated: Option<expr::Translated>,
        sysml: impl Into<String>,
        rust: String,
    ) -> (String, bool) {
        let (body, unfit) = match (clause, translated) {
            (_, Some(translated)) => (translated.rust, false),
            (Some(ValueClause::Literal(rust)), None) => (rust.clone(), false),
            // the model's own words, and `{` in them is the model's, not
            // a placeholder for `todo!` to fill
            (Some(ValueClause::Text(text)), None) => (format!("todo!(\"{{}}\", {text:?})"), true),
            (None, None) => ("todo!()".to_string(), false),
        };
        if body.starts_with("todo!") {
            self.open(
                OpenKind::Todo,
                sysml,
                Some(rust),
                match clause {
                    Some(ValueClause::Text(text)) => {
                        format!("the formula is beyond the translated subset: `{text}`")
                    }
                    _ => "the model states no formula".to_string(),
                },
            );
        }
        (body, unfit)
    }
}

mod behavior;
mod claims;
mod plan;
mod ports;
mod shape;

/// The generic signature of one struct and how its fields use it.
#[derive(Default)]
struct Plan {
    /// Parameters in declaration order: `(name, trait bound)`.
    params: Vec<(String, String)>,
    /// The parameter names each composed usage instantiates its type with.
    arguments: HashMap<ElementId, Vec<String>>,
    /// Fields dropped because a composition cycle would need endless
    /// parameters, by name and note.
    dropped: HashMap<ElementId, String>,
}

impl<'a> Generator<'a> {}

/// What the model claims about its requirements: which features
/// satisfy each, and which verification cases answer for it, each with
/// the Rust path its binding names where it names one.
#[derive(Default)]
struct Claims {
    satisfiers: HashMap<ElementId, Vec<String>>,
    verifications: HashMap<ElementId, Vec<(String, Option<String>)>>,
}

/// A port of a part, once its type turned out to be a bound API trait.
struct Port {
    name: String,
    type_name: String,
    parameter: String,
    trait_path: String,
}

/// One data field of a generated struct.
#[derive(Clone)]
struct Field {
    name: String,
    /// The name Rust knows it by, which is `ident(name)` unless another
    /// field of the same struct got there first.
    spelled: String,
    usage: ElementId,
    ty: FieldType,
    container: Container,
    /// The element type sits behind a `Box`, breaking a composition cycle.
    boxed: bool,
    /// The declared `= value`, already rendered as Rust.
    default: Option<String>,
}

/// The names taken in one Rust namespace. What SysML keeps apart Rust
/// may not -- `fuelTank`, `fuel_tank` and `'fuel tank'` are three names
/// in a model and one in Rust -- so the second one to want a spelling
/// is numbered rather than written twice over.
#[derive(Default)]
struct Namespace(HashSet<String>);

impl Namespace {
    fn take(&mut self, wanted: String) -> String {
        let mut spelled = wanted.clone();
        let mut at = 1;
        while !self.0.insert(spelled.clone()) {
            at += 1;
            spelled = format!("{wanted}_{at}");
        }
        spelled
    }
}

#[derive(Clone)]
enum FieldType {
    Scalar(&'static str),
    Generated(ElementId),
    External(String),
}

/// What a multiplicity wraps the element type in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Container {
    One,
    Optional,
    Many,
    Array(i64),
}

/// The `= value` clause of a usage: a literal already rendered as Rust,
/// or the SysML text of an expression, still to translate.
enum ValueClause {
    Literal(String),
    Text(String),
}

/// One transition, ready to be written into the table.
struct Transition {
    name: String,
    source: String,
    target: String,
    event: String,
    /// The accepted payload: its parameter name and Rust type.
    payload: Option<(String, String)>,
    /// The guard's SysML text, for the hook's documentation.
    guard: Option<String>,
    effect: bool,
}

/// A compiled action body: the traits its parts demand of whoever
/// implements it, and the statements that perform them in order.
struct Body {
    supertraits: Vec<String>,
    lines: Vec<String>,
}

/// The subactions in an order the successions allow, or nothing where
/// they lead in a circle.
fn ordered(
    steps: &[ElementId],
    after: &HashMap<ElementId, Vec<ElementId>>,
) -> Option<Vec<ElementId>> {
    let mut order: Vec<ElementId> = Vec::new();
    let mut left: Vec<ElementId> = steps.to_vec();
    while !left.is_empty() {
        // whatever nothing left is waiting on can go next
        let next = left.iter().position(|&step| {
            !left
                .iter()
                .any(|&other| after.get(&other).is_some_and(|nexts| nexts.contains(&step)))
        })?;
        order.push(left.remove(next));
    }
    Some(order)
}

/// Which numeric types a calculation's parameters disagree over, if they
/// do. Rust combines none of them without a cast, and this generator
/// writes no cast, so a formula over two of them will not compile.
fn mixed_numerics(params: &[(String, String)]) -> Option<String> {
    let numeric: Vec<&(String, String)> = params
        .iter()
        .filter(|(_, ty)| matches!(ty.as_str(), "f64" | "i64" | "u64"))
        .collect();
    let mut kinds: Vec<&str> = numeric.iter().map(|(_, ty)| ty.as_str()).collect();
    kinds.sort_unstable();
    kinds.dedup();
    if kinds.len() < 2 {
        return None;
    }
    let named: Vec<String> = numeric
        .iter()
        .map(|(name, ty)| format!("`{name}` is {ty}"))
        .collect();
    Some(format!(
        "Mixed numeric parameters ({}). Rust combines none of these without a cast \
         and none is written, so a formula over more than one of them will not \
         compile: say them in one type in the model.",
        named.join(", ")
    ))
}

/// The return clause of a signature -- nothing at all where there is
/// nothing to return, since `-> ()` is noise, and noise clippy objects to.
fn returns_clause(returns: &str) -> String {
    if returns == "()" {
        String::new()
    } else {
        format!(" -> {returns}")
    }
}

/// Whether a Rust type is `Copy`, so that reading a value of it twice
/// costs nothing. What is not gets a `.clone()` wherever it is read
/// more than once. Three places asked this and had to agree.
fn copyable(ty: &str) -> bool {
    matches!(ty, "f64" | "i64" | "u64" | "bool")
}

/// A type under the container its multiplicity asks for.
fn contained(base: String, container: Container) -> String {
    match container {
        Container::One => base,
        Container::Optional => format!("Option<{base}>"),
        Container::Many => format!("Vec<{base}>"),
        Container::Array(n) => format!("[{base}; {n}]"),
    }
}

/// The declared multiplicity of a usage, as the container it implies.
fn multiplicity(model: &Model, usage: ElementId) -> Container {
    use sysml_model::Bound;
    let Some(declared) = sysml_model::declared_multiplicity(model, usage) else {
        return Container::One;
    };
    match (declared.bound, declared.lower, declared.upper) {
        (Some(Bound::Many), _, _) => Container::Many,
        // `[0]` is a multiplicity of nothing, which Rust spells as an
        // array of nothing; a `Vec` would claim it can hold more
        (Some(Bound::Exactly(n)), _, _) if n >= 0 => Container::Array(n),
        (None, Some(Bound::Exactly(0)), Some(Bound::Exactly(1))) => Container::Optional,
        // `[1..1]` is the multiplicity everything has by default, said
        // out loud -- one of the thing, not a collection of them
        (None, Some(Bound::Exactly(1)), Some(Bound::Exactly(1))) => Container::One,
        (None, Some(_), Some(_)) => Container::Many,
        _ => Container::Many,
    }
}

/// The `= value` a usage declared, as far as Rust can start from it: a
/// literal as itself, a closed simple expression (`2.0 * 3.0`) as
/// written, a name as whatever the feature it refers to starts from --
/// any other expression over other features has no place in
/// `Default::default()`.
fn default_of(model: &Model, usage: ElementId) -> Option<String> {
    match value_clause(model, usage)? {
        ValueClause::Literal(rust) => Some(rust),
        ValueClause::Text(text) => match translate_as(
            &text,
            match wants_float(model, usage) {
                true => Numbers::AsReals,
                false => Numbers::AsWritten,
            },
            &|_| None,
            &|_| None,
        ) {
            Some(translated) => Some(translated.rust),
            // A value that is only a name refers to a feature, and name
            // resolution wrote down which one, so the value to start
            // from is that feature's own. One step: what the feature it
            // names starts from has to be a value in itself, or there
            // is nothing here to write.
            None => match value_clause(model, referent_of(model, usage)?)? {
                ValueClause::Literal(rust) => Some(rust),
                ValueClause::Text(_) => None,
            },
        },
    }
}

/// The numbers a requirement states about itself, in the order it states
/// them -- the bounds of `attribute lowerBound : Millis = 100;` and its
/// like.
///
/// A verification is handed these rather than repeating them, so the
/// requirement stays the only place they are written: changing one changes
/// the call, and a verification that no longer fits stops compiling.
fn declared_values(model: &Model, requirement: ElementId) -> Vec<String> {
    model
        .owned(requirement)
        .iter()
        .filter(|&&child| model.kind(child) == ElementKind::AttributeUsage)
        .filter_map(|&child| default_of(model, child))
        .collect()
}

/// The feature a usage's value refers to, where the whole value is a
/// name -- `attribute pin : PinNumber = ledPinNumber;`.
fn referent_of(model: &Model, usage: ElementId) -> Option<ElementId> {
    let membership = model
        .owned(usage)
        .iter()
        .copied()
        .find(|&child| model.kind(child) == ElementKind::FeatureValue)?;
    let expression = model.maybe(membership, "value")?.as_id()?;
    model.maybe(expression, "referent").and_then(Value::as_id)
}

/// The trailing result expression of a calculation body, where one was
/// written, still to translate.
fn result_clause(model: &Model, element: ElementId) -> Option<ValueClause> {
    sysml_model::result_expression_text(model, element).map(ValueClause::Text)
}

/// The Rust scalar a SysML type stands for, where it stands for one.
/// Fields and parameters both ask this, so that one model type cannot
/// be a `f64` in a struct and something else in a signature.
fn scalar_of(name: &str) -> Option<&'static str> {
    Some(match sysml_model::primitive(name)? {
        "Real" => "f64",
        "Integer" => "i64",
        // `Positive` is a `Natural` the model has ruled zero out of;
        // Rust has no such integer that is also `Default`
        "Natural" | "Positive" => "u64",
        "Boolean" => "bool",
        _ => "String",
    })
}

/// Whether a usage is typed by something Rust spells as a float. SysML
/// writes `attribute shift : Real = 1670;` and means 1670.0; Rust reads
/// that literal as an integer and refuses it, so the point has to be
/// put back.
fn wants_float(model: &Model, usage: ElementId) -> bool {
    let Some(target) = model
        .type_of(usage)
        .or_else(|| redefined(model, usage).and_then(|it| model.type_of(it)))
    else {
        return false;
    };
    match binding(model, target).and_then(|bound| bound.get(binding::ITEM).cloned()) {
        Some(path) => path == "f32" || path == "f64",
        None => model.name(target).and_then(scalar_of) == Some("f64"),
    }
}

/// A real as Rust spells it. `{:?}` writes `inf` and `NaN`, which are
/// what a model overflowing a `f64` -- `1e999` -- arrives as and which
/// Rust does not read back as anything.
fn real_literal(real: f64) -> String {
    match real {
        _ if real.is_nan() => "f64::NAN".to_string(),
        f64::INFINITY => "f64::INFINITY".to_string(),
        f64::NEG_INFINITY => "f64::NEG_INFINITY".to_string(),
        _ => format!("{real:?}"),
    }
}

/// How a Rust type of `spelled` wants its whole numbers written.
fn numbers_of(spelled: Option<&str>) -> Numbers {
    match spelled {
        Some("f32" | "f64") => Numbers::AsReals,
        _ => Numbers::AsWritten,
    }
}

/// The `= value` clause of a usage, ready for translation.
fn value_clause(model: &Model, usage: ElementId) -> Option<ValueClause> {
    use sysml_model::{Declared, Literal};
    let rust = match sysml_model::declared_value(model, usage)? {
        Declared::Text(text) => return Some(ValueClause::Text(text)),
        Declared::Literal(Literal::Real(real)) => real_literal(real),
        Declared::Literal(Literal::Int(int)) if wants_float(model, usage) => format!("{int}.0"),
        Declared::Literal(Literal::Int(int)) => format!("{int}"),
        Declared::Literal(Literal::Bool(flag)) => format!("{flag}"),
        Declared::Literal(Literal::String(text)) => format!("{text:?}.to_string()"),
    };
    Some(ValueClause::Literal(rust))
}

// Read from `sysml-model`, where every generator's model walks live;
// re-exported so the modules of this one keep reading them through
// `super::*`.
pub(crate) use sysml_model::{expression_text, redefined};

/// The `@code { :>> name = value; ... }` pairs of one element, if it
/// carries a binding.
fn binding(model: &Model, element: ElementId) -> Option<HashMap<String, String>> {
    let mut out = HashMap::new();
    for &child in model.owned(element) {
        if model.kind(child) != ElementKind::MetadataUsage {
            continue;
        }
        // `@Safety { :>> level = "high"; }` has the shape of a binding
        // and means nothing of the sort; only the metadata definition
        // this crate owns says which item an element stands for
        if model.type_of(child).and_then(|def| model.name(def)) != Some(binding::DEF) {
            continue;
        }
        for &setting in model.owned(child) {
            let redefined = model.owned(setting).iter().copied().find_map(|rel| {
                if model.kind(rel) != ElementKind::Redefinition {
                    return None;
                }
                match model.maybe(rel, "redefinedFeature") {
                    Some(Value::Ref(target)) => Some(*target),
                    _ => None,
                }
            });
            let value = model.owned(setting).iter().copied().find_map(|part| {
                if model.kind(part) != ElementKind::FeatureValue {
                    return None;
                }
                match model.maybe(part, "value") {
                    Some(Value::Ref(literal)) => Some(*literal),
                    _ => None,
                }
            });
            let (Some(redefined), Some(value)) = (redefined, value) else {
                continue;
            };
            let Some(name) = model.name(redefined) else {
                continue;
            };
            let rendered = match model.maybe(value, "value") {
                Some(Value::String(text)) => text.clone(),
                Some(Value::Bool(flag)) => flag.to_string(),
                Some(Value::Int(int)) => int.to_string(),
                _ => continue,
            };
            out.insert(name.to_string(), rendered);
        }
    }
    // A binding to something written in another language is not this
    // generator's: `@code { language = "python"; item = "app.Tank"; }`
    // says a Python class exists, and reading it as a Rust path would
    // write a use of something no crate has.
    if out.get(binding::LANGUAGE).map(String::as_str) != Some(binding::RUST) {
        return None;
    }
    (!out.is_empty()).then_some(out)
}

/// The element's documentation text, if it has any, laid out as it was
/// written: a `doc /* ... */` spanning several lines keeps its lines, and
/// each of them loses the indentation and the `*` that line it up in the
/// model text -- neither of which belongs in a Rust doc comment.
fn documentation(model: &Model, element: ElementId) -> Option<String> {
    model.owned(element).iter().find_map(|&child| {
        if model.kind(child) != ElementKind::Documentation {
            return None;
        }
        let body = model.maybe(child, "body")?.as_str()?;
        let lines: Vec<&str> = body
            .lines()
            .map(|line| {
                let line = line.trim_start();
                let line = line.strip_prefix('*').unwrap_or(line);
                // one space after the `*` is the marker's own; any
                // further indentation the writer meant, so it stays
                let line = line.strip_prefix(' ').unwrap_or(line);
                line.trim_end()
            })
            .skip_while(|line| line.is_empty())
            .collect();
        let end = lines.iter().rposition(|line| !line.is_empty())? + 1;
        Some(lines[..end].join("\n"))
    })
}

/// Documentation as Rust doc comments, one line of `///` per line of the
/// original, each behind `indent` so a field's docs sit with the field.
fn doc_comment(doc: &str, indent: &str, out: &mut String) {
    for line in doc.lines() {
        if line.is_empty() {
            writeln!(out, "{indent}///").unwrap();
        } else {
            writeln!(out, "{indent}/// {line}").unwrap();
        }
    }
}

/// `<...>` unless there is nothing to put in it.
fn angle(list: &str) -> String {
    if list.is_empty() {
        String::new()
    } else {
        format!("<{list}>")
    }
}

/// `StockVisibility` -> `stock_visibility`: the case Rust asks of
/// everything but a type, and SysML of nothing.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (at, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if at > 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// `store` -> `Store`, for parameters, variants and event names.
fn camel(name: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for ch in name.chars() {
        if ch == '_' {
            upper = true;
        } else if upper {
            out.extend(ch.to_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// A definition's name with what owns it in front, for saying which of
/// two things with one name is meant.
fn qualified_of(model: &Model, def: ElementId) -> String {
    let mut parts = Vec::new();
    let mut at = Some(def);
    while let Some(id) = at {
        if let Some(name) = model.name(id) {
            parts.push(name.to_string());
        }
        at = model.owner(id);
    }
    parts.reverse();
    parts.join("::")
}

/// Whether Rust will accept a character inside an identifier.
///
/// Rust's rule is Unicode's XID, which `char::is_alphanumeric` is not:
/// it says yes to `²` and to the Arabic-Indic digits, and `x²` is not a
/// name Rust can spell. Letters and ASCII digits are the part of XID
/// this generator needs, and anything else is dropped.
fn spellable(ch: char) -> bool {
    ch.is_alphabetic() || ch.is_ascii_digit()
}

/// A SysML name as the name of a Rust *type*. An escaped name holds
/// whatever the modeller wrote -- `'Ideal Gas Parcel'` is one name in
/// SysML -- and Rust spells type names out of a much smaller alphabet,
/// so anything it cannot use joins the words up instead of ending the
/// name early.
fn type_ident(name: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for ch in name.chars() {
        if spellable(ch) {
            if upper {
                out.extend(ch.to_uppercase());
                upper = false;
            } else {
                out.push(ch);
            }
        } else {
            // a space or a hyphen joins two words rather than cutting one
            upper = !out.is_empty();
        }
    }
    if out.is_empty() {
        // a name Rust cannot spell one character of -- `'+'`, `''` --
        // still has to be called something, and `_` is not a name
        out.push_str("Unnamed");
    } else if out.starts_with(|ch: char| ch.is_ascii_digit()) {
        out.insert(0, '_');
    }
    if TAKEN_TYPES.contains(&out.as_str()) {
        out.push('_');
    }
    out
}

/// Type names a generated file cannot have, because it uses them for
/// something else. Rust's own keywords are here for the obvious reason;
/// so are the prelude names and primitives this generator writes -- a
/// `part def Vec` taking `Vec` would leave every `Vec<T>` in the file
/// naming a struct that has no parameter, and a `part def Default` the
/// same for every derive.
const TAKEN_TYPES: [&str; 61] = [
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "if", "impl", "in",
    "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "self", "Self", "static", "struct", "super", "trait", "true", "try", "type",
    "typeof", "unsafe", "unsized", "use", "virtual", "where", "while", "yield", "Box", "Vec",
    "Option", "String", "Default", "Result", "bool", "f64", "i64", "u64",
];

/// A SysML name as the Rust name of a field, parameter or method:
/// `spectralRadius` -> `spectral_radius`, since the model's own
/// convention is the one Rust keeps for types; anything Rust cannot
/// spell in an identifier becomes `_`; and a reserved word is taken raw,
/// or suffixed where no raw form exists.
pub(crate) fn ident(name: &str) -> String {
    /// Everything Rust has taken, including what it has only reserved.
    const RESERVED: [&str; 51] = [
        "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "do",
        "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "if", "impl", "in", "let",
        "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref", "return",
        "static", "struct", "trait", "true", "try", "type", "typeof", "unsafe", "unsized", "use",
        "virtual", "where", "while", "yield",
        // no raw form exists for these, so they take a suffix instead
        "crate", "self", "Self", "super",
    ];
    /// The keywords `r#` cannot rescue.
    const SUFFIXED: [&str; 4] = ["crate", "self", "Self", "super"];

    // an escaped name may hold anything at all -- `'provide transportation'`
    // is one name in SysML -- and Rust spells identifiers out of a much
    // smaller alphabet
    let name: String = snake(name)
        .chars()
        .map(|ch| if spellable(ch) || ch == '_' { ch } else { '_' })
        .collect();
    let name = if name.starts_with(|ch: char| ch.is_ascii_digit()) {
        format!("_{name}")
    } else if name.is_empty() || name == "_" {
        // `_` is a pattern, not a name, and a name Rust can spell no
        // part of still has to be called something
        "unnamed".to_string()
    } else {
        name
    };
    if SUFFIXED.contains(&name.as_str()) {
        format!("{name}_")
    } else if RESERVED.contains(&name.as_str()) {
        format!("r#{name}")
    } else {
        name
    }
}

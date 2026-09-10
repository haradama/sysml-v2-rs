//! What the plan says a model implies, and what it would say wrongly.
//!
//! Every assertion here is a thing a code generator would otherwise
//! guess at, and most of them were guessed at wrongly by the first
//! version of this: four wheels read as a list, a set read as a list, a
//! value the model states read as absent because the usage that states
//! it borrows its name, an integer with a name read as an empty class.

use std::process::Command;

use sysml_cli::plan::{self, Plan};
use sysml_semantics::Workspace;

const MODEL: &str = r#"
package Every {
    private import ScalarValues::*;

    enum def Gear {
        park;
        drive;
    }

    attribute def Millis :> Integer;

    part def Wheel;

    abstract part def Vehicle {
        attribute mass : Real;
    }

    part def Car :> Vehicle {
        attribute :>> mass = 1200.0;
        part wheels : Wheel[4];
        ref part spare : Wheel;
        attribute notes : String[0..*] ordered nonunique;
        attribute gear : Gear = Gear::park;
    }

    calc def Braking {
        in speed : Real;
        in factor : Real;
        return : Real;
        speed * factor
    }

    item def Sample;
    action def Read { out taken : Sample; }
    action def Store { in given : Sample; }
    action def Logging {
        action read : Read;
        action store : Store;
        first read then store;
        flow read.taken to store.given;
    }

    attribute ledPinNumber : Integer = 13;

    item def Tick;
    state def Lighting {
        entry; then off;
        state off;
        state on;
        transition lighting
            first off
            accept tick : Tick
            if true
            then on;
    }
}
"#;

/// The model, planned against the standard library -- which is where the
/// answer to "what does this bottom out in" lives.
fn planned() -> Plan {
    let mut ws = Workspace::new();
    for (name, text) in sysml_stdlib::FILES {
        ws.add_file(*name, text);
    }
    let file = ws.add_file("every.sysml", MODEL);
    ws.resolve_all();
    let roots = ws.file_roots(file).to_vec();
    plan::of(&mut ws, &roots)
}

fn about<'a>(plan: &'a Plan, of: &str) -> &'a plan::Definition {
    plan.definitions
        .iter()
        .find(|it| it.of == of)
        .unwrap_or_else(|| panic!("`{of}` is planned"))
}

fn feature<'a>(def: &'a plan::Definition, name: &str) -> &'a plan::Feature {
    def.features
        .iter()
        .find(|it| it.name == name)
        .unwrap_or_else(|| panic!("`{name}` is a feature of `{}`", def.of))
}

#[test]
fn a_multiplicity_is_two_numbers_and_not_a_container() {
    let plan = planned();
    let car = about(&plan, "Every::Car");

    // `[4]`: four of them, exactly -- an array, not a list
    let wheels = feature(car, "wheels");
    assert_eq!(wheels.multiplicity.lower, 4);
    assert_eq!(wheels.multiplicity.upper, Some(4));

    // `[0..*]`: any number
    let notes = feature(car, "notes");
    assert_eq!(notes.multiplicity.lower, 0);
    assert_eq!(notes.multiplicity.upper, None);

    // and everything else is one of itself, which is what everything is
    // unless the model says otherwise -- so it is not said at all
    assert_eq!(feature(car, "gear").multiplicity.lower, 1);
    assert_eq!(feature(car, "gear").multiplicity.upper, Some(1));
    let json = serde_json::to_value(&plan).unwrap();
    let gear = json["definitions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|it| it["of"] == "Every::Car")
        .map(|it| it["features"][4].clone())
        .unwrap();
    assert_eq!(gear["name"], "gear", "{gear}");
    assert!(gear["multiplicity"].is_null(), "one of it: {gear}");
    assert!(
        gear["ordered"].is_null(),
        "one of it is in no order: {gear}"
    );
    assert!(
        gear["composite"].is_null(),
        "a value is owned by nobody: {gear}"
    );
}

/// Ordering and uniqueness are the difference between a list and a set,
/// and a model that says nothing about them is not a model that means
/// neither: the specification answers for it.
#[test]
fn what_the_model_leaves_unsaid_the_specification_says() {
    let plan = planned();
    let car = about(&plan, "Every::Car");

    let wheels = feature(car, "wheels");
    assert_eq!(wheels.unique, Some(true), "unique unless said otherwise");
    assert_eq!(
        wheels.ordered,
        Some(false),
        "unordered unless said otherwise"
    );

    let notes = feature(car, "notes");
    assert_eq!(notes.ordered, Some(true), "`ordered` was written");
    assert_eq!(notes.unique, Some(false), "`nonunique` was written");
}

/// A part is what the whole is made of and a `ref part` is one it merely
/// points at -- which decides ownership in every language that has an
/// opinion about it. An attribute is referential either way: a value is
/// not something its owner is made of.
#[test]
fn what_is_owned_and_what_is_pointed_at() {
    let plan = planned();
    let car = about(&plan, "Every::Car");
    assert_eq!(feature(car, "wheels").composite, Some(true));
    assert_eq!(feature(car, "spare").composite, Some(false));
    // and of an attribute the question is not asked: a value is not
    // something its owner is made of, however it is written
    assert_eq!(feature(car, "notes").composite, None);
}

/// A usage that only narrows an inherited one is written with no name of
/// its own. Read as nameless it disappears, and what the model says a
/// car weighs goes unsaid.
#[test]
fn a_usage_that_borrows_its_name_is_still_a_feature() {
    let plan = planned();
    let car = about(&plan, "Every::Car");
    let mass = feature(car, "mass");
    assert_eq!(mass.redefines.as_deref(), Some("Every::Vehicle::mass"));
    assert_eq!(mass.primitive, Some("Real"), "the type comes with the name");
    assert!(
        matches!(&mass.default, Some(plan::Given::Literal(it)) if it.as_f64() == Some(1200.0)),
        "{:?}",
        mass.default.is_some()
    );
}

/// What a library type bottoms out in is a question about the library,
/// and no amount of reading the name answers it.
#[test]
fn a_type_says_what_it_is_underneath() {
    let plan = planned();
    let car = about(&plan, "Every::Car");
    assert_eq!(feature(car, "notes").primitive, Some("String"));
    assert_eq!(feature(car, "mass").primitive, Some("Real"));
    // an enumeration is not a primitive, and saying it was would be
    // worse than saying nothing
    assert_eq!(feature(car, "gear").primitive, None);

    // and a definition that is a primitive under another name is that,
    // rather than a record with nothing in it
    let millis = about(&plan, "Every::Millis");
    assert_eq!(millis.shape, "value");
    assert_eq!(millis.primitive, Some("Integer"));
    // and what it specializes is what the model wrote, not the whole way
    // up: `Base::DataValue` is above `ScalarValues::Integer` and is not
    // what this definition says
    assert_eq!(millis.specializes, ["ScalarValues::Integer"]);
}

/// A declared value is a literal where it is one and the model's own
/// words where it is not. Translating the words is the reader's.
#[test]
fn a_declared_value_is_handed_over_as_it_was_written() {
    let plan = planned();
    let car = about(&plan, "Every::Car");
    assert!(
        matches!(&feature(car, "gear").default, Some(plan::Given::Expression(it)) if it == "Gear::park"),
    );
}

#[test]
fn an_enumeration_is_its_values_and_says_them_once() {
    let plan = planned();
    let gear = about(&plan, "Every::Gear");
    assert_eq!(gear.shape, "enumeration");
    assert_eq!(gear.values, ["park", "drive"]);
    // the members are variants too, and two lists of one thing are two
    // lists to keep in step
    assert!(gear.variants.is_empty());
    // an enumeration is abstract by being one, so saying so again reads
    // as "write an abstract base for this"
    assert!(!gear.is_abstract);
}

#[test]
fn a_function_says_what_it_takes_and_what_it_gives_back() {
    let plan = planned();
    let braking = about(&plan, "Every::Braking");
    assert_eq!(braking.shape, "function");
    assert_eq!(braking.expression.as_deref(), Some("speed * factor"));
    let taken: Vec<&str> = braking.features.iter().map(|it| it.name.as_str()).collect();
    assert_eq!(taken, ["speed", "factor"]);
    assert!(braking
        .features
        .iter()
        .all(|it| it.direction.as_deref() == Some("in")));
    let given = braking.returns.as_ref().expect("a declared return");
    assert_eq!(given.primitive, Some("Real"));
}

/// What order the steps happen in, and what passes between them: two
/// different relationships, and a behaviour that reads only one of them
/// is a behaviour written wrong.
#[test]
fn a_behaviour_says_its_order_and_its_dataflow() {
    let plan = planned();
    let logging = about(&plan, "Every::Logging");
    assert_eq!(logging.shape, "behaviour");
    let names: Vec<&str> = logging.steps.iter().map(|it| it.name.as_str()).collect();
    assert_eq!(names, ["read", "store"]);
    assert_eq!(logging.steps[0].after, Vec::<String>::new());
    assert_eq!(logging.steps[1].after, ["read"]);
    assert_eq!(logging.flows.len(), 1, "{:?}", logging.flows.len());
    assert_eq!(logging.flows[0].from, "read.taken");
    assert_eq!(logging.flows[0].to, "store.given");
}

/// A state machine starts somewhere, and where is written as `entry;
/// then x;` -- neither the first state declared nor anything a state
/// says about itself.
#[test]
fn a_state_machine_says_where_it_starts() {
    let plan = planned();
    let lighting = about(&plan, "Every::Lighting");
    assert_eq!(lighting.shape, "state machine");
    assert_eq!(lighting.states, ["off", "on"]);
    assert_eq!(lighting.initial.as_deref(), Some("off"));

    let [only] = &lighting.transitions[..] else {
        panic!("one transition, {}", lighting.transitions.len());
    };
    assert_eq!(only.from, "off");
    assert_eq!(only.to, "on");
    assert_eq!(only.guard.as_deref(), Some("true"));
    let waits = only.on.as_ref().expect("it accepts something");
    assert_eq!(waits.name, "tick");
    assert_eq!(waits.typed_by.as_deref(), Some("Every::Tick"));
}

/// An abstract definition is one to inherit from rather than to build,
/// and that is what the shape says.
#[test]
fn what_cannot_be_built_says_so() {
    let plan = planned();
    let vehicle = about(&plan, "Every::Vehicle");
    assert_eq!(vehicle.shape, "abstract");
    assert!(vehicle.is_abstract);
    // the parent, and only the parent: the library specialization every
    // definition gets whether it asks or not is not something to write
    assert_eq!(about(&plan, "Every::Car").specializes, ["Every::Vehicle"]);
}

/// Only the model's own definitions are planned. Nobody is generating
/// the standard library, and a plan that carried it would be sixty
/// thousand entries of which none is the caller's.
#[test]
fn the_library_is_answered_from_and_not_planned() {
    let plan = planned();
    assert!(
        plan.definitions
            .iter()
            .all(|it| it.of.starts_with("Every::")),
        "{:?}",
        plan.definitions
            .iter()
            .map(|it| it.of.as_str())
            .filter(|it| !it.starts_with("Every::"))
            .collect::<Vec<_>>()
    );
}

/// The command line answers the same thing, for whoever is not speaking
/// the protocol: JSON to work from, and a line each to read.
#[test]
fn the_command_line_hands_over_the_same_plan() {
    let dir = std::env::temp_dir().join("sysml-cli-plan");
    std::fs::create_dir_all(&dir).unwrap();
    let at = dir.join("every.sysml");
    std::fs::write(&at, MODEL).unwrap();

    let sysml = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_sysml"))
            .args(args)
            .output()
            .unwrap()
    };

    let out = sysml(&["--format", "json", "plan", at.to_str().unwrap()]);
    assert!(out.status.success());
    let said: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let planned = said["definitions"].as_array().expect("definitions");
    let car = planned
        .iter()
        .find(|it| it["of"] == "Every::Car")
        .expect("the model's own definitions");
    assert_eq!(car["features"][0]["name"], "mass", "{car}");

    // and a person gets a line each rather than a page of JSON
    let out = sysml(&["plan", at.to_str().unwrap()]);
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains("Every::Car -- record"), "{said}");
    assert!(said.contains("Every::Millis -- value"), "{said}");

    // a path that is not there is said so, rather than planned as
    // nothing at all
    let out = sysml(&["plan", "/nowhere/model.sysml"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("cannot read"),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A requirement arrives with what it demands, not only with what it is
/// about: it is the one kind of definition whose whole content used to
/// come through as an English sentence, leaving whoever wrote the check
/// to invent the logic -- and, in one case, to invent a number that
/// appeared nowhere in the plan.
#[test]
fn a_requirement_says_what_it_demands() {
    let mut ws = Workspace::new();
    for (name, text) in sysml_stdlib::FILES {
        ws.add_file(*name, text);
    }
    let file = ws.add_file(
        "req.sysml",
        "package R {\n\
         \tprivate import ScalarValues::*;\n\
         \tpart def Lamp { attribute period : Integer = 500; }\n\
         \trequirement def <'S.2'> Perceptible {\n\
         \t\tsubject lamp : Lamp;\n\
         \t\tattribute lowerBound : Integer = 100;\n\
         \t\trequire constraint { lamp.period >= lowerBound }\n\
         \t}\n\
         }\n",
    );
    ws.resolve_all();
    let roots = ws.file_roots(file).to_vec();
    let plan = plan::of(&mut ws, &roots);
    let it = about(&plan, "R::Perceptible");

    assert_eq!(it.shape, "requirement");
    // what everything that refers to it calls it
    assert_eq!(it.short_name.as_deref(), Some("S.2"));
    assert_eq!(it.requires, ["lamp.period >= lowerBound"]);
    let subject = it.subject.as_ref().expect("what it is about");
    assert_eq!(subject.name, "lamp");
    assert_eq!(subject.typed_by.as_deref(), Some("R::Lamp"));
    // the subject is not one of its fields, and neither is the
    // constraint
    let named: Vec<&str> = it.features.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(named, ["lowerBound"]);
}

/// What a package declares outright is planned too. A default that names
/// one -- `attribute pin = ledPinNumber;` -- named something the plan did
/// not contain, and the reader put a guessed constant at the middle of
/// the model.
#[test]
fn what_a_package_declares_outright_is_planned() {
    let plan = planned();
    let [constant] = &plan.constants[..] else {
        panic!("one constant, {}", plan.constants.len());
    };
    assert_eq!(constant.name, "Every::ledPinNumber", "named from the root");
    assert_eq!(constant.primitive, Some("Integer"));
    assert!(matches!(&constant.default, Some(plan::Given::Literal(it)) if it.as_i64() == Some(13)),);
}

/// A port is a place a thing connects and not a thing with fields, and
/// the shape says which -- `record` covered a struct, an interface, a
/// message, a value and a predicate alike.
#[test]
fn a_port_is_not_a_record() {
    let mut ws = Workspace::new();
    for (name, text) in sysml_stdlib::FILES {
        ws.add_file(*name, text);
    }
    let file = ws.add_file("p.sysml", "package P { port def Gpio; item def Sample; }\n");
    ws.resolve_all();
    let roots = ws.file_roots(file).to_vec();
    let plan = plan::of(&mut ws, &roots);
    assert_eq!(about(&plan, "P::Gpio").shape, "port");
    assert_eq!(about(&plan, "P::Sample").shape, "record");
}

/// Documentation arrives as prose. What the parser keeps is the inside
/// of the comment, margin and all, and a generator handed that writes
/// the asterisks into a docstring.
#[test]
fn documentation_comes_without_its_comment() {
    let mut ws = Workspace::new();
    let file = ws.add_file(
        "d.sysml",
        "package D {\n\
         \tpart def Lamp {\n\
         \t\tdoc\n\
         \t\t/*\n\
         \t\t * Two states, and the lamp follows\n\
         \t\t * which one is current.\n\
         \t\t */\n\
         \t}\n\
         }\n",
    );
    ws.resolve_all();
    let roots = ws.file_roots(file).to_vec();
    let plan = plan::of(&mut ws, &roots);
    assert_eq!(
        about(&plan, "D::Lamp").documentation.as_deref(),
        Some("Two states, and the lamp follows\nwhich one is current."),
    );
}

/// What a model annotates a definition with is what the model says
/// about it, and dropping it drops the only account there is of how some
/// definitions relate.
///
/// A port with no features reads as an empty record and the actions read
/// as belonging to nothing -- and the model said, of all four of them,
/// which existing type and which of its methods they stand for.
#[test]
fn what_the_model_annotates_a_definition_with_is_handed_over() {
    let mut ws = Workspace::new();
    let file = ws.add_file(
        "hal.sysml",
        "package Hal {\n\
         \tmetadata def code {\n\
         \t\tattribute writtenIn : ScalarValues::String;\n\
         \t\tattribute path : ScalarValues::String;\n\
         \t}\n\
         \tport def Gpio {\n\
         \t\t@code {\n\
         \t\t\t:>> writtenIn = \"rust\";\n\
         \t\t\t:>> path = \"crate::hal::Gpio\";\n\
         \t\t}\n\
         \t}\n\
         }\n",
    );
    ws.resolve_all();
    let roots = ws.file_roots(file).to_vec();
    let plan = plan::of(&mut ws, &roots);
    let gpio = about(&plan, "Hal::Gpio");
    let [said] = &gpio.annotations[..] else {
        panic!("one annotation, {}", gpio.annotations.len());
    };
    assert_eq!(said.of, "Hal::code");
    assert!(
        matches!(said.says.get("path"), Some(plan::Given::Literal(it)) if it == "crate::hal::Gpio"),
        "{:?}",
        said.says.keys().collect::<Vec<_>>()
    );
    // and which language it is about, which is what makes one notation
    // serve them all
    assert!(matches!(said.says.get("writtenIn"), Some(plan::Given::Literal(it)) if it == "rust"),);
}

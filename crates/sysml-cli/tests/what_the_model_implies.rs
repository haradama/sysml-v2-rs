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

    variation part def Drive {
        variant part electric : Vehicle;
        variant part petrol : Vehicle;
    }

    part def Car :> Vehicle {
        attribute :>> mass = 1200.0;
        attribute towing : Boolean = false;
        part wheels : Wheel[4];
        attribute howMany : Integer = 2;
        part spares : Wheel[howMany];
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
        first off then on;
        entry; then off;
        state off;
        state on;
        transition lighting
            first off
            accept tick : Tick
            if true
            then on;
        transition darkening
            first on
            then off;
    }
}
"#;

/// The model, planned against the standard library -- which is where the
/// answer to "what does this bottom out in" lives.
/// The binary, for the tests that are about what it hands over.
fn sysml(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sysml"))
        .args(args)
        .output()
        .unwrap()
}

/// A model on disk, under a directory of its own.
fn written(dir: &str, name: &str, text: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let at = dir.join(name);
    std::fs::write(&at, text).unwrap();
    at
}

/// What the plan says about one definition, or a failure naming it.
fn said<'a>(plan: &'a serde_json::Value, of: &str) -> &'a serde_json::Value {
    plan["definitions"]
        .as_array()
        .expect("a plan has definitions")
        .iter()
        .find(|it| it["of"] == of)
        .unwrap_or_else(|| panic!("`{of}` is planned: {plan}"))
}

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

    // `[n]`, naming a feature: any number of them, since a plan that
    // said one of them would be read as a model that said nothing
    let spares = feature(car, "spares");
    assert_eq!(spares.multiplicity.lower, 0);
    assert_eq!(spares.multiplicity.upper, None);

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
        .and_then(|it| {
            it["features"]
                .as_array()?
                .iter()
                .find(|it| it["name"] == "gear")
                .cloned()
        })
        .expect("a car has a gear");
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

    let [lighting_up, darkening] = &lighting.transitions[..] else {
        panic!("two transitions, {}", lighting.transitions.len());
    };
    assert_eq!(lighting_up.from, "off");
    assert_eq!(lighting_up.to, "on");
    assert_eq!(lighting_up.guard.as_deref(), Some("true"));
    let waits = lighting_up.on.as_ref().expect("it accepts something");
    assert_eq!(waits.name, "tick");
    assert_eq!(waits.typed_by.as_deref(), Some("Every::Tick"));

    // and one that waits for nothing and asks nothing, which is a
    // transition a machine takes as soon as it can
    assert_eq!(darkening.from, "on");
    assert_eq!(darkening.to, "off");
    assert!(darkening.on.is_none(), "it waits for nothing");
    assert!(darkening.guard.is_none(), "it asks nothing");
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
    let at = written("sysml-cli-plan", "every.sysml", MODEL);

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

    // and so is a library that is not there, which is a different path
    // and used to be a plan answered as if the library had been read
    let out = Command::new(env!("CARGO_BIN_EXE_sysml"))
        .args(["plan", at.to_str().unwrap()])
        .env("SYSML_LIBRARY_PATH", "/nowhere/library")
        .output()
        .unwrap();
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

/// A variation is a choice between its variants -- one of them, not all
/// of them -- which is a sum type in every language that has one and a
/// tagged union in the rest.
#[test]
fn a_variation_is_its_variants() {
    let plan = planned();
    let drive = about(&plan, "Every::Drive");
    assert_eq!(drive.shape, "variation");
    let named: Vec<&str> = drive.variants.iter().map(|it| it.name.as_str()).collect();
    assert_eq!(named, ["electric", "petrol"]);
    assert_eq!(
        drive.variants[0].typed_by.as_deref(),
        Some("Every::Vehicle")
    );
    // it is abstract by being a variation, so saying so again reads as
    // "write an abstract base for this"
    assert!(!drive.is_abstract);
}

/// Every kind of literal a model can declare arrives as itself rather
/// than as the words for it.
#[test]
fn a_literal_of_any_kind_arrives_as_a_value() {
    let plan = planned();
    let car = about(&plan, "Every::Car");
    assert!(
        matches!(&feature(car, "towing").default, Some(plan::Given::Literal(it)) if it == false),
        "a boolean",
    );
    assert!(
        matches!(&feature(car, "mass").default, Some(plan::Given::Literal(it)) if it == 1200.0),
        "a real",
    );
}

/// KerML declares the same things in its own words, and a model written
/// in it used to plan as nothing at all.
///
/// An empty answer reads as "there is nothing here to write", which is
/// a wrong answer rather than a missing one -- and half the standard
/// library, along with anything written below the systems layer, is
/// KerML.
#[test]
fn kerml_is_planned_in_the_same_words_as_sysml() {
    let at = written(
        "sysml-cli-plan-kerml",
        "shapes.kerml",
        "package Shapes {\n\
         \tabstract class Shape {\n\
         \t\tfeature name : ScalarValues::String;\n\
         \t}\n\
         \tstruct Circle :> Shape {\n\
         \t\tfeature radius : ScalarValues::Real;\n\
         \t}\n\
         \tassoc Touching {\n\
         \t\tend feature one : Circle;\n\
         \t\tend feature other : Circle;\n\
         \t}\n\
         }\n",
    );
    let out = sysml(&["--format", "json", "plan", at.to_str().unwrap()]);
    let plan: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();

    let shape = said(&plan, "Shapes::Shape");
    assert_eq!(shape["kind"], "Class");
    assert_eq!(shape["shape"], "abstract", "{shape}");
    assert_eq!(shape["features"][0]["name"], "name");
    assert_eq!(shape["features"][0]["primitive"], "String");

    let circle = said(&plan, "Shapes::Circle");
    assert_eq!(circle["shape"], "record");
    assert_eq!(circle["specializes"][0], "Shapes::Shape");

    // an association's ends are what it holds -- it relates its two
    // sides and has nothing else -- and each says that it is an end, so
    // a reader writes a pair of references rather than a record with a
    // copy of each side inside it
    let touching = said(&plan, "Shapes::Touching");
    let ends = touching["features"].as_array().expect("two ends");
    assert_eq!(ends.len(), 2, "{touching}");
    assert!(ends.iter().all(|it| it["end"] == true), "{touching}");
    assert_eq!(ends[0]["type"], "Shapes::Circle");
}

/// What a definition performs is what makes it something that *does*
/// anything, and it used to go missing: a part with three performed
/// actions planned as three fields and no way to use them.
#[test]
fn what_a_definition_performs_is_said() {
    let at = written(
        "sysml-cli-plan-performs",
        "does.sysml",
        "package Doing {\n\
         \taction def Wait { in millis : ScalarValues::Integer; }\n\
         \tpart def Clock {\n\
         \t\tperform action pause : Wait {\n\
         \t\t\tdoc /* until the next tick */\n\
         \t\t}\n\
         \t}\n\
         \tpart def PreciseClock :> Clock {\n\
         \t\tperform action :>> pause {\n\
         \t\t\tdoc /* more often */\n\
         \t\t}\n\
         \t}\n\
         }\n",
    );
    let out = sysml(&["--format", "json", "plan", at.to_str().unwrap()]);
    let plan: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(plan["checked"]["ok"], true, "{plan}");
    let clock = said(&plan, "Doing::Clock");
    let [performs] = clock["performs"].as_array().unwrap().as_slice() else {
        panic!("one performed action: {clock}");
    };
    assert_eq!(performs["name"], "pause");
    assert_eq!(
        performs["of"], "Doing::Wait",
        "so its parameters can be read"
    );
    assert_eq!(performs["documentation"], "until the next tick");

    // a subtype that narrows it writes neither name nor type, and both
    // are borrowed from what it redefines -- the way a feature written
    // `attribute :>> mass = 1200.0;` borrows `mass`
    let precise = said(&plan, "Doing::PreciseClock");
    let [narrowed] = precise["performs"].as_array().unwrap().as_slice() else {
        panic!("one performed action: {precise}");
    };
    assert_eq!(narrowed["name"], "pause");
    assert_eq!(narrowed["of"], "Doing::Wait");
    assert_eq!(narrowed["documentation"], "more often");
}

/// A plan is only as good as the model behind it, and says so.
///
/// A feature whose type resolved to nothing is planned with no type at
/// all. `generate_rust` refuses such a model outright; the plan hands
/// over what it has -- somebody writing a model is entitled to see what
/// it implies so far -- and says what is missing rather than letting a
/// reader write an untyped field and never learn the model was
/// misspelt.
#[test]
fn a_plan_says_whether_the_model_behind_it_resolves() {
    let at = written(
        "sysml-cli-plan-typo",
        "typo.sysml",
        "package P { part def Tank { attribute fuel : MassValu; } }\n",
    );
    let out = sysml(&["--format", "json", "plan", at.to_str().unwrap()]);
    let plan: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(plan["checked"]["ok"], false, "{plan}");
    assert_eq!(plan["checked"]["unresolved"][0], "MassValu", "{plan}");
    // the feature is there, and what types it is not
    let tank = said(&plan, "P::Tank");
    assert_eq!(tank["features"][0]["name"], "fuel");
    assert!(tank["features"][0]["type"].is_null(), "{tank}");
    // and the exit code says not to build from it yet
    assert!(!out.status.success());

    // and a file that does not parse is a different failure and a worse
    // one: it is missing whole declarations rather than one type, so a
    // plan that called it sound would be claiming the model contains
    // what the parser never read
    let unclosed = written(
        "sysml-cli-plan-typo",
        "unclosed.sysml",
        "package R { part def Tank;\n",
    );
    let out = sysml(&["--format", "json", "plan", unclosed.to_str().unwrap()]);
    let plan: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(plan["checked"]["ok"], false, "{plan}");
    assert!(
        plan["checked"]["syntax"][0]
            .as_str()
            .is_some_and(|it| it.contains("unclosed.sysml")),
        "{plan}"
    );
    assert!(!out.status.success());

    // a model that resolves says so, and says nothing else about it
    let sound = written(
        "sysml-cli-plan-typo",
        "sound.sysml",
        "package Q { part def Tank { attribute fuel : ISQ::MassValue; } }\n",
    );
    let out = sysml(&["--format", "json", "plan", sound.to_str().unwrap()]);
    let plan: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(plan["checked"]["ok"], true, "{plan}");
    assert!(plan["checked"]["unresolved"].is_null(), "{plan}");
    assert!(plan["checked"]["syntax"].is_null(), "{plan}");
    assert!(out.status.success());
}

/// Files that hold no definition say so rather than printing nothing.
///
/// Silence reads as "there is nothing here to write", which is a wrong
/// answer where the truth is that the model has no definitions -- a
/// package of imports, or a file whose declarations are all next door.
#[test]
fn a_model_that_implies_no_code_says_so() {
    let at = written("sysml-cli-plan-empty", "empty.sysml", "package Empty;\n");
    let out = sysml(&["plan", at.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("no definitions"),
        "{:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// A connection is a thing between two others, and its ends are what it
/// holds.
///
/// Dropping them as "not fields" left `connection def Pipe { end source
/// : Pump; end target : Tank; }` planned as an empty record -- a pipe
/// between nothing and nothing.
#[test]
fn a_connection_carries_the_two_sides_it_relates() {
    let at = written(
        "sysml-cli-plan-connection",
        "plumbing.sysml",
        "package Plumbing {\n\
         \tpart def Pump;\n\
         \tpart def Tank;\n\
         \tconnection def Pipe {\n\
         \t\tend source : Pump;\n\
         \t\tend target : Tank;\n\
         \t}\n\
         }\n",
    );
    let out = sysml(&["--format", "json", "plan", at.to_str().unwrap()]);
    let plan: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let pipe = said(&plan, "Plumbing::Pipe");
    let ends = pipe["features"].as_array().expect("two ends");
    assert_eq!(ends.len(), 2, "{pipe}");
    assert_eq!(ends[0]["name"], "source");
    assert_eq!(ends[0]["type"], "Plumbing::Pump");
    assert_eq!(ends[0]["end"], true, "not something the pipe is made of");
    assert_eq!(ends[1]["type"], "Plumbing::Tank");
}

/// What is joined to what inside a definition.
///
/// A definition's parts are not a bag: `connect pump to tank;` is the
/// whole of why the two are there together, and a reader given the parts
/// and not the wiring writes a record whose fields have nothing to do
/// with each other.
#[test]
fn what_is_joined_to_what_is_said() {
    let at = written(
        "sysml-cli-plan-wiring",
        "works.sysml",
        "package Works {\n\
         \tpart def Pump;\n\
         \tpart def Tank;\n\
         \tconnection def Pipe { end source : Pump; end target : Tank; }\n\
         \tpart def Plant {\n\
         \t\tpart pump : Pump;\n\
         \t\tpart tank : Tank;\n\
         \t\tconnect pump to tank;\n\
         \t\tconnection feed : Pipe connect pump to tank;\n\
         \t}\n\
         }\n",
    );
    let out = sysml(&["--format", "json", "plan", at.to_str().unwrap()]);
    let plan: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let plant = said(&plan, "Works::Plant");
    let [plain, named] = plant["connections"].as_array().unwrap().as_slice() else {
        panic!("two connections: {plant}");
    };

    // `connect a to b;` names nothing, and the joining is the point
    assert_eq!(plain["from"], "pump");
    assert_eq!(plain["to"], "tank");
    assert!(plain["name"].is_null(), "{plain}");
    assert!(plain["of"].is_null(), "{plain}");

    // and one that says what kind of joining it is says so
    assert_eq!(named["name"], "feed");
    assert_eq!(named["of"], "Works::Pipe");
    assert_eq!(named["from"], "pump");
    assert_eq!(named["to"], "tank");
}

/// To a person, the same two failures are said in words, since a plan
/// read off a terminal has no `checked` block to look at.
#[test]
fn a_person_is_told_why_a_plan_is_not_to_be_built_from() {
    let typo = written(
        "sysml-cli-plan-said",
        "typo.sysml",
        "package P { part def Tank { attribute fuel : MassValu; } }\n",
    );
    let out = sysml(&["plan", typo.to_str().unwrap()]);
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        said.contains("1 name(s) in this model resolve to nothing"),
        "{said}"
    );
    assert!(said.contains("`sysml check` says where"), "{said}");

    let unclosed = written(
        "sysml-cli-plan-said",
        "unclosed.sysml",
        "package R { part def Tank;\n",
    );
    let out = sysml(&["plan", unclosed.to_str().unwrap()]);
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("does not parse: "), "{said}");
    assert!(said.contains("unclosed.sysml"), "{said}");
}

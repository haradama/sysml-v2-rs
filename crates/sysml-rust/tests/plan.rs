//! A plan, out as JSON and back: what a generator written in Rust reads
//! is what this crate wrote, with nothing lost to what the JSON left
//! unsaid.

use sysml_rust::plan::{Multiplicity, Plan};
use sysml_semantics::Workspace;

const MODEL: &str = "package P {\n\
    \tprivate import ScalarValues::*;\n\
    \tattribute limit : Integer = 3;\n\
    \tenum def Gear { park; drive; }\n\
    \tpart def Wheel;\n\
    \tpart def Car {\n\
    \t\tattribute mass : Real = 1200.0;\n\
    \t\tpart wheels : Wheel[4];\n\
    \t\tattribute gear : Gear = Gear::park;\n\
    \t}\n\
    \tstate def Lighting {\n\
    \t\tentry; then off;\n\
    \t\tstate off;\n\
    \t\tstate on;\n\
    \t\ttransition lighting first off then on;\n\
    \t}\n\
    \tport def Gpio;\n\
    \tmetadata def code { attribute path : String; }\n\
    }\n";

fn planned() -> Plan {
    let mut ws = Workspace::new();
    for (name, text) in sysml_stdlib::FILES {
        ws.add_file(*name, text);
    }
    let file = ws.add_file("p.sysml", MODEL);
    ws.resolve_all();
    let roots = ws.file_roots(file).to_vec();
    sysml_rust::plan::of(&mut ws, &roots)
}

#[test]
fn what_is_written_is_what_is_read_back() {
    let plan = planned();
    assert!(plan.checked.ok, "{:?}", plan.checked);
    let json = serde_json::to_string_pretty(&plan).unwrap();
    // what a plan says nothing about is not written, and reads back as
    // nothing said
    assert!(
        !json.contains("\"multiplicity\": {\n        \"lower\": 1,\n        \"upper\": 1"),
        "{json}"
    );
    assert!(!json.contains("\"is_abstract\": false"), "{json}");
    let read: Plan = serde_json::from_str(&json).unwrap();
    assert_eq!(read, plan);
    // the shapes are the reader's to match on
    let car = read
        .definitions
        .iter()
        .find(|it| it.of == "P::Car")
        .expect("the car is planned");
    assert_eq!(car.shape, "record");
    assert_eq!(car.kind, "PartDefinition");
    let wheels = car
        .features
        .iter()
        .find(|it| it.name == "wheels")
        .expect("the wheels are a feature");
    assert_eq!(
        wheels.multiplicity,
        Multiplicity {
            lower: 4,
            upper: Some(4)
        }
    );
    let mass = car.features.iter().find(|it| it.name == "mass").unwrap();
    assert_eq!(mass.primitive.as_deref(), Some("Real"));
    assert_eq!(mass.multiplicity, Multiplicity::default());
    assert!(mass.multiplicity.is_one());
    // and what was left out of the JSON reads back as the default
    let bare: Plan =
        serde_json::from_str(r#"{ "checked": { "ok": true }, "definitions": [] }"#).unwrap();
    assert!(bare.definitions.is_empty());
    assert!(bare.constants.is_empty());
    assert!(format!("{bare:?}").contains("checked"));
    assert_eq!(bare.clone(), bare);
}

/// A metadata definition is the vocabulary an annotation is written in,
/// and a conjugated port is implied beside every port the model
/// declares; both were planned as records, so a generator wrote a class
/// `code` and a class `~Gpio`.
#[test]
fn what_is_not_code_is_not_planned() {
    let plan = planned();
    let planned: Vec<&str> = plan.definitions.iter().map(|it| it.of.as_str()).collect();
    assert!(planned.contains(&"P::Gpio"), "{planned:?}");
    assert!(!planned.iter().any(|it| it.contains('~')), "{planned:?}");
    assert!(!planned.contains(&"P::code"), "{planned:?}");
}

#[cfg(feature = "schema")]
#[test]
fn the_schema_says_what_a_plan_is() {
    let schema = sysml_rust::plan::schema();
    let properties = schema["properties"].as_object().unwrap();
    for field in ["checked", "definitions", "constants"] {
        assert!(properties.contains_key(field), "{field} is in the schema");
    }
    let text = schema.to_string();
    for word in [
        "state machine",
        "transitions",
        "multiplicity",
        "primitive",
        "specializes",
        "performs",
    ] {
        assert!(text.contains(word), "{word} is in the schema");
    }
}

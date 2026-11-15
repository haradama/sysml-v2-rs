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
        "successions",
        "decide",
        "after",
        "until",
    ] {
        assert!(text.contains(word), "{word} is in the schema");
    }
}

fn plan_of(text: &str) -> Plan {
    let mut ws = Workspace::new();
    for (name, text) in sysml_stdlib::FILES {
        ws.add_file(*name, text);
    }
    let file = ws.add_file("p.sysml", text);
    ws.resolve_all();
    let roots = ws.file_roots(file).to_vec();
    sysml_rust::plan::of(&mut ws, &roots)
}

/// A transition the model leaves unnamed is a transition still, called
/// what `sysml rustgen` calls it; and one waiting on a signal it binds
/// to no name, on a change or on a time says what it waits for.
#[test]
fn every_transition_is_planned_with_what_triggers_it() {
    let plan = plan_of(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tattribute def Go;\n\
         \tattribute def Sig { attribute level : Integer; }\n\
         \tstate def M {\n\
         \t\tattribute lvl : Integer = 1;\n\
         \t\tentry; then a;\n\
         \t\tstate a; state b; state c;\n\
         \t\ttransition first a accept Go then b;\n\
         \t\ttransition a_to_b first b accept s : Sig then c;\n\
         \t\ttransition first b accept when lvl > 3 then c;\n\
         \t\ttransition first c accept after 5 [SI::s] then a;\n\
         \t\ttransition first c accept at 7 then a;\n\
         \t}\n\
         }\n",
    );
    assert!(plan.checked.ok, "{:?}", plan.checked);
    let machine = plan.definitions.iter().find(|it| it.of == "S::M").unwrap();
    let json = serde_json::to_string(&machine.transitions).unwrap();
    type Said<'a> = (&'a str, bool, &'a str, &'a str, &'a str, Option<&'a str>);
    let said: Vec<Said> = machine
        .transitions
        .iter()
        .map(|it| {
            let on = it.on.as_ref().expect(&json);
            (
                it.name.as_str(),
                it.unnamed,
                on.kind.as_str(),
                on.name.as_str(),
                on.typed_by.as_deref().unwrap_or(""),
                on.expression.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        said,
        [
            // the model's own `a_to_b` keeps its name, as in rustgen
            ("a_to_b_2", true, "signal", "", "S::Go", None),
            ("a_to_b", false, "signal", "s", "S::Sig", None),
            ("b_to_c", true, "when", "", "", Some("lvl > 3")),
            ("c_to_a", true, "after", "", "", Some("5 [SI::s]")),
            ("c_to_a_2", true, "at", "", "", Some("7")),
        ],
        "{json}"
    );
    // and what an older plan wrote still reads
    let old: sysml_rust::plan::Trigger =
        serde_json::from_str(r#"{ "name": "s", "type": "S::Sig" }"#).unwrap();
    assert_eq!(old.kind, "signal");
}

/// A transition written inside a state's body -- `state off { accept Go
/// then on; }` -- names only where it goes, and leaves from the state it
/// is written in. It was left out of the plan, so a generator reading it
/// wrote a machine with no way out of `off`.
#[test]
fn a_transition_inside_a_state_leaves_from_that_state() {
    let plan = plan_of(
        "package S {\n\
         \tattribute def Go;\n\
         \tstate def Lamp {\n\
         \t\tentry; then off;\n\
         \t\tstate off { accept Go then on; }\n\
         \t\tstate on { transition dim first on accept Go then off; }\n\
         \t}\n\
         }\n",
    );
    assert!(plan.checked.ok, "{:?}", plan.checked);
    let machine = plan
        .definitions
        .iter()
        .find(|it| it.of == "S::Lamp")
        .unwrap();
    let json = serde_json::to_string(&machine.transitions).unwrap();
    let said: Vec<(&str, bool, &str, &str, &str)> = machine
        .transitions
        .iter()
        .map(|it| {
            (
                it.name.as_str(),
                it.unnamed,
                it.from.as_str(),
                it.to.as_str(),
                it.on
                    .as_ref()
                    .and_then(|on| on.typed_by.as_deref())
                    .unwrap_or(""),
            )
        })
        .collect();
    assert_eq!(
        said,
        [
            ("off_to_on", true, "off", "on", "S::Go"),
            ("dim", false, "on", "off", "S::Go"),
        ],
        "{json}"
    );
}

/// A behaviour's order is more than its subactions: the control nodes
/// and the structured actions are steps too, a `decide`'s guards are on
/// its successions, and every name an `after` or a succession carries is
/// a step the plan lists.
#[test]
fn a_behaviour_plans_its_control_nodes_guards_and_bodies() {
    let plan = plan_of(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \taction def Measure { out reading : Real; }\n\
         \taction def Log { in reading : Real; }\n\
         \taction def R {\n\
         \t\tin flag : Boolean;\n\
         \t\tfirst start then measure;\n\
         \t\taction measure : Measure;\n\
         \t\tfirst measure then d;\n\
         \t\tdecide d;\n\
         \t\t\tif flag then logA;\n\
         \t\t\telse logB;\n\
         \t\taction logA : Log;\n\
         \t\taction logB : Log;\n\
         \t\tfirst logA then m;\n\
         \t\tfirst logB then m;\n\
         \t\tmerge m;\n\
         \t\tfirst m then f;\n\
         \t\tfork f;\n\
         \t\twhile flag { action again : Measure; }\n\
         \t\tif flag { action more : Measure; } else if not flag { action less : Measure; }\n\
         \t\tloop action forever { action x : Measure; } until flag;\n\
         \t}\n\
         }\n",
    );
    assert!(plan.checked.ok, "{:?}", plan.checked);
    let r = plan.definitions.iter().find(|it| it.of == "S::R").unwrap();
    let json = serde_json::to_string_pretty(&r).unwrap();
    let kinds: Vec<(&str, &str)> = r
        .steps
        .iter()
        .map(|it| (it.name.as_str(), it.kind.as_str()))
        .collect();
    assert_eq!(
        kinds,
        [
            ("measure", "action"),
            ("d", "decide"),
            ("logA", "action"),
            ("logB", "action"),
            ("m", "merge"),
            ("f", "fork"),
            ("while", "while"),
            ("if", "if"),
            ("loop", "loop"),
        ],
        "{json}"
    );
    // the decision's ways out, guard and else
    let out_of_d: Vec<(&str, Option<&str>)> = r
        .successions
        .iter()
        .filter(|it| it.from == "d")
        .map(|it| (it.to.as_str(), it.guard.as_deref()))
        .collect();
    assert_eq!(out_of_d, [("logA", Some("flag")), ("logB", None)], "{json}");
    // nothing named that is not listed: `start` is the library's
    let listed: Vec<&str> = r.steps.iter().map(|it| it.name.as_str()).collect();
    for step in &r.steps {
        for before in &step.after {
            assert!(listed.contains(&before.as_str()), "{before}: {json}");
        }
    }
    for succession in &r.successions {
        assert!(listed.contains(&succession.from.as_str()), "{json}");
        assert!(listed.contains(&succession.to.as_str()), "{json}");
    }
    let m = r.steps.iter().find(|it| it.name == "m").unwrap();
    assert_eq!(m.after, ["logA", "logB"]);
    // the structured actions carry their conditions and bodies
    let looped = r.steps.iter().find(|it| it.kind == "while").unwrap();
    assert_eq!(looped.condition.as_deref(), Some("flag"));
    assert_eq!(looped.body.as_ref().unwrap().steps[0].name, "again");
    let branch = r.steps.iter().find(|it| it.kind == "if").unwrap();
    assert_eq!(branch.body.as_ref().unwrap().steps[0].name, "more");
    let otherwise = &branch.otherwise.as_ref().unwrap().steps[0];
    assert_eq!(otherwise.kind, "if", "{json}");
    assert_eq!(otherwise.condition.as_deref(), Some("not flag"));
    assert_eq!(otherwise.body.as_ref().unwrap().steps[0].name, "less");
    let forever = r.steps.iter().find(|it| it.kind == "loop").unwrap();
    assert_eq!(forever.condition, None);
    assert_eq!(forever.until.as_deref(), Some("flag"));
    assert_eq!(forever.body.as_ref().unwrap().steps[0].name, "x");
    // a guarded succession is no transition of a behaviour
    assert!(r.transitions.is_empty(), "{json}");
    // and the JSON reads back
    let read: Plan = serde_json::from_str(&serde_json::to_string(&plan).unwrap()).unwrap();
    assert_eq!(read, plan);
    // an older step, with no kind, is an action
    let old: sysml_rust::plan::Step = serde_json::from_str(r#"{ "name": "a" }"#).unwrap();
    assert_eq!(old.kind, "action");
}

/// A redefinition of a redefinition names the one it redefines by the
/// name that one answers to, not by the `?` of a name it never declared.
#[test]
fn what_is_redefined_is_named_even_where_it_borrows_its_name() {
    let plan = plan_of(
        "package C {\n\
         \tprivate import ScalarValues::*;\n\
         \tpart def Concrete { attribute c : Integer = 1; }\n\
         \tpart def Sub :> Concrete { attribute :>> c = 2; }\n\
         \tpart def Sub2 :> Sub { attribute c2 :>> c; }\n\
         }\n",
    );
    let redefines = |of: &str, name: &str| {
        plan.definitions
            .iter()
            .find(|it| it.of == of)
            .and_then(|it| it.features.iter().find(|it| it.name == name))
            .and_then(|it| it.redefines.clone())
    };
    assert_eq!(redefines("C::Sub", "c").as_deref(), Some("C::Concrete::c"));
    assert_eq!(redefines("C::Sub2", "c2").as_deref(), Some("C::Sub::c"));
}

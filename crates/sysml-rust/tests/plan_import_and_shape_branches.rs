//! Branches of the plan, the rustdoc importer and the Rust shapes that
//! only an unusual input reaches: each test builds that input and says
//! what comes out.

use sysml_rust::plan::{Plan, Step};
use sysml_semantics::Workspace;

fn workspace(text: &str) -> (Workspace, Vec<sysml_model::ElementId>) {
    let mut ws = Workspace::new();
    for (name, text) in sysml_stdlib::FILES {
        ws.add_file(*name, text);
    }
    let file = ws.add_file("p.sysml", text);
    ws.resolve_all();
    let roots = ws.file_roots(file).to_vec();
    (ws, roots)
}

fn plan_of(text: &str) -> Plan {
    let (mut ws, roots) = workspace(text);
    sysml_rust::plan::of(&mut ws, &roots)
}

fn rust_of(text: &str) -> sysml_rust::Generated {
    let (ws, roots) = workspace(text);
    sysml_rust::generate(ws.model(), &roots).unwrap()
}

fn kinds(steps: &[Step]) -> Vec<(&str, &str)> {
    steps
        .iter()
        .map(|it| (it.name.as_str(), it.kind.as_str()))
        .collect()
}

/// Every kind of step a behaviour can be made of is planned under the
/// word a generator reads, and what a loop or an `if` is made of is read
/// from the clauses the model holds -- the sequence of a `for`, a
/// literal condition of any type.
#[test]
fn every_kind_of_step_is_planned_by_its_word() {
    let plan = plan_of(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \titem def Sig;\n\
         \tpart def T;\n\
         \tpart tgt : T;\n\
         \tattribute v : Integer;\n\
         \taction def R {\n\
         \t\tin flag : Boolean;\n\
         \t\tin seq : Integer[*];\n\
         \t\taccept s : Sig;\n\
         \t\tsend new Sig() to tgt;\n\
         \t\tassign v := 1;\n\
         \t\tfork f;\n\
         \t\tjoin j;\n\
         \t\tterminate;\n\
         \t\tfor i in seq { action a; }\n\
         \t\twhile 3 { action w1; }\n\
         \t\twhile 2.5 { action w2; }\n\
         \t\twhile true { action w3; }\n\
         \t\twhile \"s\" { action w4; }\n\
         \t\tif flag { action z; }\n\
         \t}\n\
         }\n",
    );
    assert!(plan.checked.ok, "{:?}", plan.checked);
    let r = plan.definitions.iter().find(|it| it.of == "S::R").unwrap();
    assert_eq!(
        kinds(&r.steps),
        [
            ("accept", "accept"),
            ("send", "send"),
            ("assign", "assign"),
            ("f", "fork"),
            ("j", "join"),
            ("terminate", "terminate"),
            ("for", "for"),
            ("while", "while"),
            ("while_2", "while"),
            ("while_3", "while"),
            ("while_4", "while"),
            ("if", "if"),
        ]
    );
    let conditions: Vec<Option<&str>> = r
        .steps
        .iter()
        .filter(|it| it.kind == "for" || it.kind == "while")
        .map(|it| it.condition.as_deref())
        .collect();
    assert_eq!(
        conditions,
        [
            Some("seq"),
            Some("3"),
            Some("2.5"),
            Some("true"),
            Some("\"s\"")
        ]
    );
    let looped = r.steps.iter().find(|it| it.kind == "for").unwrap();
    assert_eq!(
        kinds(&looped.body.as_ref().unwrap().steps),
        [("a", "action")]
    );
}

/// An `if` or a loop that holds no condition and no body is a step with
/// nothing said about either, and a transition that relates nothing is
/// not planned: a generator reads no half of one.
#[test]
fn a_step_or_transition_with_nothing_to_say_is_planned_bare() {
    let plan = plan_of(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \taction def R {\n\
         \t\tin flag : Boolean;\n\
         \t\ttransition t;\n\
         \t\ttransition first a then b;\n\
         \t\tif flag;\n\
         \t\twhile flag;\n\
         \t\tloop;\n\
         \t\taction a;\n\
         \t\taction b;\n\
         \t}\n\
         \tstate def L {\n\
         \t\tstate a;\n\
         \t\tstate b;\n\
         \t\ttransition t;\n\
         \t\ttransition first a;\n\
         \t\ttransition first a then b then a;\n\
         \t}\n\
         }\n",
    );
    let r = plan.definitions.iter().find(|it| it.of == "S::R").unwrap();
    for step in r.steps.iter().filter(|it| it.unnamed) {
        assert!(step.condition.is_none() && step.body.is_none(), "{step:?}");
        assert!(step.until.is_none() && step.otherwise.is_none(), "{step:?}");
    }
    assert_eq!(
        kinds(&r.steps),
        [
            ("if", "if"),
            ("while", "while"),
            ("loop", "loop"),
            ("a", "action"),
            ("b", "action")
        ]
    );
    assert_eq!(r.successions.len(), 1);
    let state = plan.definitions.iter().find(|it| it.of == "S::L").unwrap();
    assert!(state.transitions.is_empty(), "{:?}", state.transitions);
}

/// What a `when` waits for is the expression the model wrote; a literal
/// is held as its value, not its text, and is planned as the text it was.
#[test]
fn a_transition_waiting_on_a_literal_says_what_it_waits_for() {
    let plan = plan_of(
        "package S {\n\
         \tstate def L {\n\
         \t\tstate a;\n\
         \t\tstate b;\n\
         \t\ttransition first a accept when 3 then b;\n\
         \t\ttransition first a accept when 2.5 then b;\n\
         \t\ttransition first a accept when true then b;\n\
         \t\ttransition first a accept when \"x\" then b;\n\
         \t\ttransition first a then b;\n\
         \t}\n\
         }\n",
    );
    let l = plan.definitions.iter().find(|it| it.of == "S::L").unwrap();
    let waits: Vec<Option<&str>> = l
        .transitions
        .iter()
        .map(|it| {
            it.on
                .as_ref()
                .map(|on| on.expression.as_deref().unwrap_or(""))
        })
        .collect();
    assert_eq!(
        waits,
        [Some("3"), Some("2.5"), Some("true"), Some("\"x\""), None]
    );
}

/// A transition written inside a state's body names only where it goes;
/// it leaves from the state it is written in.
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
    let lamp = plan
        .definitions
        .iter()
        .find(|it| it.of == "S::Lamp")
        .unwrap();
    let said: Vec<(&str, &str, &str)> = lamp
        .transitions
        .iter()
        .map(|it| (it.name.as_str(), it.from.as_str(), it.to.as_str()))
        .collect();
    assert_eq!(said, [("off_to_on", "off", "on"), ("dim", "on", "off")]);
}

/// A declared value on a field a cycle forces behind a `Box`, or on one
/// whose multiplicity is no number, is not written in the container's
/// form: the field starts at its type's default and the generator says so
/// where it cannot.
#[test]
fn a_declared_value_on_a_boxed_field_or_an_unnumbered_one_is_left_open() {
    let generated = rust_of(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tpart def A {\n\
         \t\tattribute s : Integer = 2;\n\
         \t\tpart b : A = s;\n\
         \t\tpart c : A[0..1] = s;\n\
         \t}\n\
         \tpart def Counted {\n\
         \t\tattribute n : Integer = 2;\n\
         \t\tattribute some : Integer[n] = 3;\n\
         \t\tattribute upto : Integer[n..*] = 3;\n\
         \t}\n\
         }\n",
    );
    let rust = &generated.rust;
    assert!(rust.contains("b: Default::default(),"), "{rust}");
    assert!(rust.contains("c: Some(Box::new(2)),"), "{rust}");
    assert!(rust.contains("some: Default::default(),"), "{rust}");
    assert!(rust.contains("upto: Default::default(),"), "{rust}");
    for name in ["some", "upto"] {
        assert!(
            generated
                .open
                .iter()
                .any(|open| open.why.contains(&format!("`{name}` starts"))
                    && open.why.contains("lower bound is no number")),
            "{name}: {:?}",
            generated.open
        );
    }
}

/// A member a nearer definition answers to by name is not handed down
/// again from the general: one method, the nearer one's.
#[test]
fn a_member_a_nearer_one_answers_to_is_not_handed_down_twice() {
    let generated = rust_of(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \taction def Go { in x : Real; }\n\
         \tabstract part def Base { perform action go : Go; }\n\
         \tpart def Derived :> Base { perform action go : Go; }\n\
         }\n",
    );
    let rust = &generated.rust;
    let derived = &rust[rust.find("pub struct Derived").expect("Derived")..];
    assert_eq!(derived.matches("fn go(").count(), 1, "{rust}");
}

fn import(json: &str) -> String {
    sysml_rust::rustdoc_to_sysml(json, None).unwrap().sysml
}

/// A method with no name is nothing to import, and one in a generic
/// `impl` is listed rather than bound.
#[test]
fn an_impl_of_nameless_and_generic_methods_is_skipped_aloud() {
    let json = r#"{
        "root": "r",
        "index": {
            "r": { "name": "krate", "inner": { "module": { "items": ["s", "g"] } } },
            "s": { "name": "Plain", "inner": { "struct": { "generics": { "params": [] },
                "kind": { "plain": { "fields": [] } }, "impls": ["own"] } } },
            "own": { "inner": { "impl": { "trait": null, "generics": { "params": [] },
                "items": ["anon", "named"] } } },
            "anon": { "visibility": "public", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [ [ "self", { "borrowed_ref": { "is_mutable": false } } ] ], "output": null },
                "header": { "is_async": false } } } },
            "named": { "name": "ping", "visibility": "public", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [ [ "self", { "borrowed_ref": { "is_mutable": false } } ] ], "output": null },
                "header": { "is_async": false } } } },
            "g": { "name": "Open", "inner": { "struct": { "generics": { "params": [] },
                "kind": { "plain": { "fields": [] } }, "impls": ["wide"] } } },
            "wide": { "inner": { "impl": { "trait": null,
                "generics": { "params": [ { "name": "T", "kind": { "type": {} } } ] },
                "items": ["spread"] } } },
            "spread": { "name": "spread", "visibility": "public", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [ [ "self", { "borrowed_ref": { "is_mutable": false } } ] ], "output": null },
                "header": { "is_async": false } } } }
        }
    }"#;
    let sysml = import(json);
    assert!(sysml.contains("action def Ping {"), "{sysml}");
    assert!(
        sysml.contains("//   Open::spread -- in a generic impl"),
        "{sysml}"
    );
    assert!(!sysml.contains("action def Spread"), "{sysml}");
}

/// A guarded succession in a behaviour is a transition owning the
/// succession it guards: the plan orders the steps by it, with the guard,
/// and a transition that names no succession orders nothing.
#[test]
fn a_guarded_succession_orders_the_steps_it_joins() {
    let plan = plan_of(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \taction def R {\n\
         \t\tin flag : Boolean;\n\
         \t\taction a;\n\
         \t\taction b;\n\
         \t\taction c;\n\
         \t\ttransition g first a if flag then b;\n\
         \t\ttransition u;\n\
         \t\ttransition first b;\n\
         \t\ttransition then c;\n\
         \t}\n\
         }\n",
    );
    let r = plan.definitions.iter().find(|it| it.of == "S::R").unwrap();
    let ordered: Vec<(&str, &str, Option<&str>)> = r
        .successions
        .iter()
        .map(|it| (it.from.as_str(), it.to.as_str(), it.guard.as_deref()))
        .collect();
    assert_eq!(ordered[0], ("a", "b", Some("flag")), "{ordered:?}");
    let b = r.steps.iter().find(|it| it.name == "b").unwrap();
    assert_eq!(b.after, ["a"]);
    let g = r.transitions.iter().find(|it| it.name == "g").unwrap();
    assert_eq!((g.from.as_str(), g.to.as_str()), ("a", "b"));
    assert_eq!(g.guard.as_deref(), Some("flag"));
}

/// A condition written as a literal, or a loop ending `until` one, is
/// planned as the literal; an `if` carries both of its bodies.
#[test]
fn an_if_and_a_loop_say_what_they_test_and_run() {
    let plan = plan_of(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \taction def R {\n\
         \t\tin flag : Boolean;\n\
         \t\tif 3 { action p; } else { action q; }\n\
         \t\tloop { action x; } until 3;\n\
         \t\tloop { action y; } until true;\n\
         \t\twhile flag { action k; } until 2.5;\n\
         \t\tfor i in 1..3 { action m; }\n\
         \t}\n\
         }\n",
    );
    let r = plan.definitions.iter().find(|it| it.of == "S::R").unwrap();
    let said: Vec<(&str, Option<&str>, Option<&str>)> = r
        .steps
        .iter()
        .map(|it| {
            (
                it.kind.as_str(),
                it.condition.as_deref(),
                it.until.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        said,
        [
            ("if", Some("3"), None),
            ("loop", None, Some("3")),
            ("loop", None, Some("true")),
            ("while", Some("flag"), Some("2.5")),
            ("for", Some("1..3"), None),
        ]
    );
    let first = &r.steps[0];
    assert_eq!(
        kinds(&first.body.as_ref().unwrap().steps),
        [("p", "action")]
    );
    assert_eq!(
        kinds(&first.otherwise.as_ref().unwrap().steps),
        [("q", "action")]
    );
}

/// A `when` on something that is no written expression -- `null`, `*` --
/// says it waits on a change and nothing about what; a signal with a
/// guard and an effect says both.
#[test]
fn a_trigger_with_no_expression_and_a_guarded_effect_are_planned() {
    let plan = plan_of(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tattribute def Go;\n\
         \tstate def L {\n\
         \t\tattribute flag : Boolean;\n\
         \t\tstate a;\n\
         \t\tstate b;\n\
         \t\ttransition first a accept when null then b;\n\
         \t\ttransition first a accept when * then b;\n\
         \t\ttransition first a accept Go if flag then b;\n\
         \t\ttransition first a accept Go if flag do action eff then b;\n\
         \t}\n\
         }\n",
    );
    let l = plan.definitions.iter().find(|it| it.of == "S::L").unwrap();
    let said: Vec<(&str, Option<&str>, bool)> = l
        .transitions
        .iter()
        .map(|it| {
            (
                it.on.as_ref().unwrap().kind.as_str(),
                it.guard.as_deref(),
                it.effect,
            )
        })
        .collect();
    assert_eq!(
        said,
        [
            ("when", None, false),
            ("when", None, false),
            ("signal", Some("flag"), false),
            ("signal", Some("flag"), true),
        ]
    );
    assert!(l.transitions[0].on.as_ref().unwrap().expression.is_none());
    assert!(l.transitions[1].on.as_ref().unwrap().expression.is_none());
}

/// A succession written out with its keyword relates the steps as one
/// written `first a then b` does.
#[test]
fn a_succession_written_out_orders_the_steps_it_joins() {
    let plan = plan_of(
        "package S {\n\
         \taction def B {\n\
         \t\taction a;\n\
         \t\taction b;\n\
         \t\taction c;\n\
         \t\tsuccession s first a then b;\n\
         \t\tsuccession first b then c;\n\
         \t}\n\
         \tstate def M {\n\
         \t\tstate x;\n\
         \t\tstate y;\n\
         \t\tsuccession first x then y;\n\
         \t\ttransition t first x then y;\n\
         \t}\n\
         }\n",
    );
    let b = plan.definitions.iter().find(|it| it.of == "S::B").unwrap();
    let ordered: Vec<(&str, &str)> = b
        .successions
        .iter()
        .map(|it| (it.from.as_str(), it.to.as_str()))
        .collect();
    assert_eq!(ordered, [("a", "b"), ("b", "c")], "{b:?}");
}

/// What a transition written in a state's body relates is read from the
/// ends it names: one end is where it goes, and one that names no end at
/// all is no transition.
#[test]
fn transitions_written_in_a_state_body_leave_from_it() {
    let plan = plan_of(
        "package S {\n\
         \tattribute def Go;\n\
         \tstate def M {\n\
         \t\tstate a { transition then b; }\n\
         \t\tstate b { transition t accept Go then a; }\n\
         \t\tstate c { transition first c; }\n\
         \t\tstate d { transition t2; }\n\
         \t}\n\
         }\n",
    );
    let m = plan.definitions.iter().find(|it| it.of == "S::M").unwrap();
    let said: Vec<(&str, &str, &str)> = m
        .transitions
        .iter()
        .map(|it| (it.name.as_str(), it.from.as_str(), it.to.as_str()))
        .collect();
    assert_eq!(
        said,
        [("a_to_b", "a", "b"), ("t", "b", "a"), ("c_to_c", "c", "c")]
    );
}

/// An `impl` the index does not hold, or a method of one it does not, is
/// nothing to read, and what is there is read all the same.
#[test]
fn what_the_index_does_not_hold_is_passed_over() {
    let json = r#"{
        "root": "r",
        "index": {
            "r": { "name": "krate", "inner": { "module": { "items": ["s"] } } },
            "s": { "name": "Plain", "inner": { "struct": { "generics": { "params": [] },
                "kind": { "plain": { "fields": [] } }, "impls": ["gone", "own"] } } },
            "own": { "inner": { "impl": { "trait": null, "generics": { "params": [] },
                "items": ["missing", "named"] } } },
            "named": { "name": "ping", "visibility": "public", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [ [ "self", { "borrowed_ref": { "is_mutable": false } } ] ], "output": null },
                "header": { "is_async": false } } } }
        }
    }"#;
    let sysml = import(json);
    assert!(sysml.contains("action def Ping {"), "{sysml}");
}

/// A formula reads a value through the fields of generated structs, one
/// value at each step: through a field that is a number, or to a field
/// that holds several, there is no value to read, and the calculation is
/// left to write.
#[test]
fn a_formula_through_what_is_no_one_struct_is_left_to_write() {
    let generated = rust_of(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tpart def P {\n\
         \t\tattribute mass : ISQ::MassValue;\n\
         \t\tattribute xs : Real[*];\n\
         \t\tcalc through : Real { mass.num }\n\
         \t\tcalc several : Real { xs }\n\
         \t}\n\
         }\n",
    );
    let rust = &generated.rust;
    for name in ["through", "several"] {
        let at = rust.find(&format!("fn {name}(")).expect(name);
        let body = &rust[at..];
        let body = body[body.find('{').unwrap() + 1..].trim_start();
        assert!(body.starts_with("todo!"), "{name}: {rust}");
    }
}

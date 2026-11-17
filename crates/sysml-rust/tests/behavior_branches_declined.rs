//! What behaviour generation declines, and says so: a transition that
//! leaves the machine, a step no body performs, a condition that is no
//! `bool`, a call whose name is two things, and an API held twice.

const SCALARS: &str = "package ScalarValues {\n\
    \tabstract datatype Boolean;\n\
    \tabstract datatype String;\n\
    \tabstract datatype Real;\n\
    \tabstract datatype Integer;\n\
    \tabstract datatype Natural;\n\
    \tabstract datatype Positive;\n\
    \tabstract datatype Complex;\n}\n";

/// A miniature bound API package, the shape `sysml import-rust` writes.
const API: &str = "package Api {\n\
    \tprivate import ScalarValues::*;\n\
    \tmetadata def code {\n\
    \t\tattribute writtenIn : String;\n\
    \t\tattribute path : String;\n\
    \t\tattribute module : String;\n\
    \t\tattribute takesSelf : String;\n\
    \t\tattribute isAsync : Boolean;\n\
    \t\tattribute isFallible : Boolean;\n\
    \t\tattribute capabilities : String;\n\
    \t}\n\
    \tport def Store { @code { :>> writtenIn = \"rust\"; :>> path = \"fake::Store\"; } }\n\
    \tport def Metrics { @code { :>> writtenIn = \"rust\"; :>> path = \"fake::Metrics\"; } }\n\
    \taction def Ping { @code { :>> writtenIn = \"rust\"; :>> path = \"fake::Store::ping\"; :>> takesSelf = \"&self\"; :>> isAsync = false; :>> isFallible = false; } }\n\
    \taction def Consume { @code { :>> writtenIn = \"rust\"; :>> path = \"fake::Store::consume\"; :>> takesSelf = \"self\"; } }\n\
    \taction def Orphan { @code { :>> writtenIn = \"rust\"; :>> path = \"elsewhere::Api::orphan\"; :>> takesSelf = \"&self\"; } }\n\
    \taction def Unbound;\n\
    \taction def Fuzzy { @code { :>> writtenIn = \"rust\"; :>> path = \"fake::Store::fuzzy\"; :>> takesSelf = \"&self\"; } in blob : Plain; }\n\
    \taction def Purge { @code { :>> writtenIn = \"rust\"; :>> path = \"fake::Store::purge\"; :>> takesSelf = \"&self\"; :>> isFallible = true; } out error : String; }\n\
    \taction def Shuffle { @code { :>> writtenIn = \"rust\"; :>> path = \"fake::Store::shuffle\"; :>> takesSelf = \"&self\"; } inout buffer : String; }\n\
    \taction def Vanish { @code { :>> writtenIn = \"rust\"; :>> path = \"fake::Store::vanish\"; :>> takesSelf = \"&self\"; :>> isFallible = true; } out result : String; }\n\
    \taction def Count { @code { :>> writtenIn = \"rust\"; :>> path = \"fake::Store::count\"; :>> takesSelf = \"&self\"; :>> isFallible = false; } out result : Real; }\n\
    \taction def Watch { @code { :>> writtenIn = \"rust\"; :>> path = \"fake::Store::watch\"; :>> takesSelf = \"&mut self\"; :>> isAsync = true; :>> isFallible = true; } in sku : String; out result : Payload; out error : String; }\n\
    \titem def Payload { @code { :>> writtenIn = \"rust\"; :>> path = \"fake::Payload\"; } }\n\
    \tport def Plain;\n}\n";

fn generated(system: &str) -> sysml_rust::Generated {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file("scalars.kerml", SCALARS);
    ws.add_file("api.sysml", API);
    let file = ws.add_file("system.sysml", system);
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "the test model must resolve");
    let roots = ws.file_roots(file).to_vec();
    sysml_rust::generate(ws.model(), &roots).unwrap()
}

fn generate(system: &str) -> String {
    generated(system).rust
}

const STEPS: &str = "\tprivate import ScalarValues::*;\n\
     \taction def Measure { out reading : Real; }\n";

/// An action def `Run` with one subaction `m1`, then `body`.
fn run(body: &str) -> String {
    format!(
        "package S {{\n{STEPS}\
         \tpart tgt;\n\
         \taction def Run {{\n\
         \t\tin flag : Boolean;\n\
         \t\tin n : Integer;\n\
         \t\tin seq : Integer[*];\n\
         \t\tattribute v : Integer;\n\
         \t\taction m1 : Measure;\n\
         {body}\
         \t}}\n\
         }}\n"
    )
}

/// Why `Run` is left to the implementor, as its doc comment says it.
#[track_caller]
fn why_not_performed(body: &str) -> String {
    let rust = generate(&run(body));
    rust.lines()
        .find_map(|line| line.strip_prefix("/// Not performed here: "))
        .unwrap_or_else(|| panic!("`Run` was performed:\n{rust}"))
        .to_string()
}

#[test]
fn a_transition_inside_a_state_that_goes_nowhere_is_said_not_dropped() {
    let generated = generated(
        "package S {\n\
         \tstate def M {\n\
         \t\tstate a { then a; }\n\
         \t\tstate b { transition t1 first b then outside; }\n\
         \t\tstate c { transition then b; }\n\
         \t}\n\
         \tstate outside;\n\
         }\n",
    );
    let rust = &generated.rust;
    for state in ["a", "b"] {
        assert!(
            rust.contains(&format!(
                "// not generated: a transition inside state `{state}` -- its target is no state of this machine\n"
            )),
            "{rust}"
        );
        assert!(
            generated.open.iter().any(|open| open.sysml == state
                && open.why == "a transition inside it -- its target is no state of this machine"),
            "{:?}",
            generated.open
        );
    }
    // `c` names where it goes, which is a state of the machine: the one
    // transition that is a row of the table
    assert!(!rust.contains("inside state `c`"), "{rust}");
    assert!(rust.contains("(MState::C, MEvent::CToB) => {"), "{rust}");
}

#[test]
fn a_guard_may_call_a_calculation_and_not_a_name_that_is_two() {
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tprivate import A::*;\n\
         \tpackage A { calc def Widen { in x : Real; return : Real = x * 2.0; } }\n\
         \tpackage B { calc def Widen { in x : Real; return : Real = x * 3.0; } }\n\
         \tcalc def Hot { in x : Real; return : Boolean = x > 3.0; }\n\
         \tattribute def Sig { attribute level : Real; }\n\
         \tstate def M {\n\
         \t\tstate a; state b;\n\
         \t\ttransition t first a accept s : Sig if Widen(s.level) > 1.0 do action e { attribute q : Real = B::Widen(2.0); } then b;\n\
         \t\ttransition v first b accept s : Sig if Widen(s.level) > 1.0 then a;\n\
         \t\ttransition u first b accept s : Sig if Hot(s.level) and s.level > 1.0 then a;\n\
         \t}\n\
         }\n",
    );
    // `Hot` is one function, and the guard calls it
    assert!(
        rust.contains(
            "fn guard_u(&self, s: &Sig) -> bool {\n        hot(s.level) && (s.level > 1.0)\n    }"
        ),
        "{rust}"
    );
    // on its own, `Widen` is the one function
    assert!(
        rust.contains("fn guard_v(&self, s: &Sig) -> bool {\n        widen(s.level) > 1.0\n    }"),
        "{rust}"
    );
    // `Widen` is two things within the one transition (`A::Widen` in its
    // guard, `B::Widen` in its effect): the text alone cannot say which
    // call is which, so the guard stays the permissive default
    assert!(
        rust.contains("fn guard_t(&self, s: &Sig) -> bool {\n        true\n    }"),
        "{rust}"
    );
}

#[test]
fn a_calc_reads_into_the_parameter_it_is_given() {
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tattribute def Pt { attribute x : Real; }\n\
         \tpart def P { calc c : Real { in p : Pt; p.x } }\n\
         }\n",
    );
    assert!(
        rust.contains("pub fn c(&self, p: Pt) -> f64 {\n        p.x\n    }"),
        "{rust}"
    );
}

#[test]
fn whole_numbers_written_out_stay_whole_beside_each_other() {
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tpart def Pt {\n\
         \t\tattribute y : Integer = 1 + 2;\n\
         \t\tattribute z : Real = 1 + 2;\n\
         \t\tcalc k : Integer { 1 + 2 }\n\
         \t}\n\
         }\n",
    );
    assert!(rust.contains("            y: 1 + 2,\n"), "{rust}");
    assert!(rust.contains("            z: 1.0 + 2.0,\n"), "{rust}");
    assert!(
        rust.contains("pub fn k(&self) -> i64 {\n        1 + 2\n    }"),
        "{rust}"
    );
}

#[test]
fn an_api_held_twice_is_not_called_on_either() {
    let generated = generated(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tprivate import Api::*;\n\
         \titem def Gauge { @code { :>> writtenIn = \"rust\"; :>> path = \"fake::Gauge\"; } }\n\
         \taction def Total { @code { :>> writtenIn = \"rust\"; :>> path = \"fake::Gauge::total\"; :>> takesSelf = \"&self\"; :>> isAsync = false; :>> isFallible = false; } out result : Real; }\n\
         \tpart def Twin { item a : Gauge; item b : Gauge; perform action total : Total; }\n\
         }\n",
    );
    assert!(
        !generated.rust.contains("self.a.total()"),
        "{}",
        generated.rust
    );
    assert!(
        generated.open.iter().any(|open| open.why
            == "`fake::Gauge::total` is called on a `fake::Gauge`, and `Twin` holds more than one, so which is not said"),
        "{:?}",
        generated.open
    );
}

#[test]
fn an_else_if_is_gathered_with_the_steps_inside_it() {
    let rust = generate(&run(
        "\t\tif flag { action a : Measure; } else if flag { action b : Measure; } else { action c : Measure; }\n",
    ));
    assert!(
        rust.contains(concat!(
            "        let m1 = self.measure();\n",
            "        if flag {\n",
            "            let a = self.measure();\n",
            "        } else if flag {\n",
            "            let b = self.measure();\n",
            "        } else {\n",
            "            let c = self.measure();\n",
            "        }\n",
        )),
        "{rust}"
    );
}

#[test]
fn a_decision_with_two_ways_out_and_no_guard_is_not_performed() {
    let why = why_not_performed(
        "\t\taction x : Measure; action y : Measure; action z : Measure;\n\
         \t\tfirst m1 then d;\n\
         \t\tdecide d;\n\
         \t\t\tif flag then x;\n\
         \t\t\telse y;\n\
         \t\t\telse z;\n",
    );
    assert_eq!(
        why,
        "`d` has more than one way out with no guard, and a decision takes one."
    );
}

#[test]
fn an_if_or_a_loop_without_what_it_needs_is_not_performed() {
    assert_eq!(
        why_not_performed("\t\tif flag;\n"),
        "an `if` with no condition or no body."
    );
    assert_eq!(
        why_not_performed("\t\tif flag { action a : Measure; } else { action b : Measure; } else { action c : Measure; }\n"),
        "an `if` with more than one `else`."
    );
    assert_eq!(
        why_not_performed("\t\twhile flag;\n"),
        "a loop with no body."
    );
    assert_eq!(
        why_not_performed("\t\tloop { action a : Measure; } else { action b : Measure; }\n"),
        "a loop whose `until` is no expression."
    );
}

#[test]
fn a_condition_that_is_no_bool_is_not_performed() {
    for condition in ["3", "n + 1", "\"s\"", "seq", "(flag)"] {
        assert_eq!(
            why_not_performed(&format!(
                "\t\twhile {condition} {{ action a : Measure; }}\n"
            )),
            format!(
                "the condition `{condition}` is past what this generator translates into a `bool`."
            ),
        );
    }
    assert_eq!(
        why_not_performed("\t\tif n + 1 { action a : Measure; }\n"),
        "the condition `n + 1` is past what this generator translates into a `bool`."
    );
    assert_eq!(
        why_not_performed("\t\tloop action l { action a : Measure; } until n;\n"),
        "the condition `n` is past what this generator translates into a `bool`."
    );
}

#[test]
fn a_step_no_body_performs_is_named_for_what_it_is() {
    for (step, what) in [
        ("accept s : Boolean;", "a step of it is an `accept`"),
        ("assign v := 1;", "a step of it is an assignment"),
        (
            "for i in seq { action a : Measure; }",
            "a step of it is a `for` loop",
        ),
        ("terminate;", "a step of it is a `terminate`"),
        ("send n to tgt;", "a step of it is a `send`"),
        ("state s;", "`s` is a StateUsage"),
    ] {
        assert_eq!(
            why_not_performed(&format!("\t\t{step}\n")),
            format!("{what} -- no generated body performs one."),
            "{step}"
        );
    }
}

#[test]
fn a_transition_with_no_end_of_its_own_is_no_row_of_the_table() {
    let generated = generated(
        "package S {\n\
         \tstate def M {\n\
         \t\tstate a; state b;\n\
         \t\ttransition t1 first a accept Go then b;\n\
         \t\ttransition t5;\n\
         \t\tstate d {\n\
         \t\t\tstate inner;\n\t\t\ttransition then outside;\n\t\t\ttransition then inner;\n\
         \t\t\ttransition t4 accept Go;\n\
         \t\t}\n\
         \t}\n\
         \tstate outside;\n\
         \tattribute def Go;\n\
         }\n",
    );
    let rust = &generated.rust;
    // the one that goes somewhere is a row; the two that name no state of
    // the machine are not, and the one written inside `d` is said, once
    assert!(rust.contains("(MState::A, MEvent::T1) => {"), "{rust}");
    assert!(!rust.contains("T4") && !rust.contains("T5"), "{rust}");
    assert_eq!(
        rust.matches("// not generated: a transition inside state `d`")
            .count(),
        1,
        "{rust}"
    );
}

#[test]
fn a_guard_of_whole_numbers_written_out_is_one_comparison() {
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tattribute def Sig { attribute level : Integer; }\n\
         \tstate def M {\n\
         \t\tstate a; state b;\n\
         \t\ttransition t first a accept s : Sig if 1 + 2 > 2 then b;\n\
         \t}\n\
         }\n",
    );
    assert!(
        rust.contains("fn guard_t(&self, s: &Sig) -> bool {\n        (1 + 2) > 2\n    }"),
        "{rust}"
    );
}

#[test]
fn a_transition_naming_one_state_inside_a_state_is_a_row_when_that_is_a_state_of_the_machine() {
    let rust = generate(
        "package S {\n\
         \tstate def M {\n\
         \t\tstate a; state b;\n\
         \t\ttransition t6 first a;\n\
         \t\tstate d { transition t7 first a; transition t8 first outside; }\n\
         \t}\n\
         \tstate outside;\n\
         }\n",
    );
    // one end, written inside `d`: the row leaves `d` for that end when it
    // is a state of the machine, and is said and left out when it is not;
    // written outside any state, one end is no row at all
    assert!(rust.contains("(MState::D, MEvent::T7) => {"), "{rust}");
    assert!(!rust.contains("T6") && !rust.contains("T8"), "{rust}");
    assert!(
        rust.contains("// not generated: a transition inside state `d` -- its target is no state of this machine\n"),
        "{rust}"
    );
}

/// A behaviour made of several steps of one definition is still made of
/// that definition once, and what that one is made of is looked through
/// for the behaviour above it.
#[test]
fn steps_of_one_definition_twice_over_are_looked_through_once() {
    let rust = generate(
        "package S {\n\
         \taction def Top { action b : Mid; }\n\
         \taction def Mid { action c1 : Leaf; action c2 : Leaf; }\n\
         \taction def Leaf { action d : Base; }\n\
         \taction def Base;\n\
         }\n",
    );
    assert!(!rust.contains("Not performed here"), "{rust}");
}

/// A decision whose one way out has no guard always goes that way.
#[test]
fn a_decision_with_only_an_else_always_takes_it() {
    let rust = generate(&run("\t\taction x : Measure;\n\
         \t\tfirst m1 then d;\n\
         \t\tdecide d;\n\
         \t\t\telse x;\n"));
    assert!(
        rust.contains("        {\n            let x = self.measure();\n        }\n"),
        "{rust}"
    );
}

const READS: &str = "\taction def Use { in value : Real; }\n";

/// A step reads what another produces once that one has run; read from
/// one on the other way out of a decision, nothing has.
#[test]
fn a_step_that_reads_what_another_way_produces_is_not_performed() {
    let why = why_not_performed(&format!(
        "\t\taction x : Use;\n\
         \t\taction y : Measure;\n\
         \t\tfirst m1 then d;\n\
         \t\tdecide d;\n\
         \t\t\tif flag then x;\n\
         \t\t\telse y;\n\
         \t\tflow from y.reading to x.value;\n{READS}"
    ));
    assert_eq!(
        why,
        "`x.value` reads what `y` produces, which nothing runs before it."
    );
}

/// Nor can a step read what is produced only on one way through a
/// decision it does not itself sit on.
#[test]
fn a_step_that_reads_what_one_branch_produced_is_not_performed() {
    let why = why_not_performed(&format!(
        "\t\taction x : Use;\n\
         \t\taction y : Measure;\n\
         \t\taction z : Measure;\n\
         \t\tfirst m1 then d;\n\
         \t\tdecide d;\n\
         \t\t\tif flag then y;\n\
         \t\t\telse z;\n\
         \t\tfirst y then mg;\n\
         \t\tfirst z then mg;\n\
         \t\tmerge mg;\n\
         \t\tfirst mg then x;\n\
         \t\tflow from y.reading to x.value;\n{READS}"
    ));
    assert_eq!(
        why,
        "`x.value` reads what `y` produces only inside a branch or a loop `x` is not in."
    );
}

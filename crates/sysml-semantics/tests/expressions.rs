//! Names written inside expressions.
//!
//! A constraint body, the result of a `calc`, the value after `=` -- the
//! names in them are references like any other, and until they were
//! resolved a typo in one was a model the toolchain called sound.

use sysml_semantics::Workspace;

fn resolved(text: &str) -> Vec<String> {
    let mut ws = Workspace::new();
    let file = ws.add_file("e.sysml", text);
    ws.resolve_files(&[file]);
    ws.unresolved().iter().map(|u| u.name.clone()).collect()
}

#[test]
fn a_name_in_a_value_a_body_or_a_constraint_is_a_reference() {
    let unresolved = resolved(
        "package P {\n\
         \tattribute def Number;\n\
         \tpart def Thing { attribute size : Number; }\n\
         \tattribute shared : Number;\n\
         \tpart t : Thing;\n\
         \tattribute good : Number = shared;\n\
         \tattribute chained : Number = t.size;\n\
         \tattribute bad : Number = notDeclared;\n\
         \tattribute deep : Number = t.notAMember;\n\
         \trequirement def R {\n\
         \t\tsubject s : Thing;\n\
         \t\trequire constraint { s.size > 0 }\n\
         \t\trequire constraint { s.absent > 0 }\n\
         \t}\n\
         \tcalc def C { in n : Number; n + shared }\n\
         \tcalc def D { in n : Number; n + missingHere }\n\
         }\n",
    );
    assert_eq!(
        unresolved,
        ["notDeclared", "t::notAMember", "s::absent", "missingHere",]
    );
}

#[test]
fn what_an_expression_names_elsewhere_is_left_alone() {
    // Three shapes whose names are not looked up here: an argument names
    // a parameter of what is called, a body expression declares its own,
    // and a step off a call has no namespace to be a member of.
    let unresolved = resolved(
        "package P {\n\
         \tattribute def Number;\n\
         \tattribute list : Number;\n\
         \tcalc def F { in arg : Number; arg }\n\
         \tattribute named : Number = F(arg = 1);\n\
         \tattribute picked : Number = list->select { in each; each > 0 };\n\
         \tattribute filtered : Number = list.?{ in each; each > 0 };\n\
         \tattribute stepped : Number = F(1).whatever;\n\
         \tattribute positional : Number = F(list);\n\
         }\n",
    );
    assert!(unresolved.is_empty(), "{unresolved:?}");
}

#[test]
fn a_named_connection_is_something_the_model_can_name() {
    // `connection k connect a to b` used to read `k` as a reference, so
    // the connection had no name and nothing could reach what it holds.
    let unresolved = resolved(
        "package P {\n\
         \tattribute def Number;\n\
         \tpart def T;\n\
         \tpart a : T;\n\
         \tpart b : T;\n\
         \tconnection k connect a to b { attribute rate : Number; }\n\
         \tbinding v bind a = b;\n\
         \tallocation w allocate a to b;\n\
         \tattribute reached : Number = k.rate;\n\
         \tattribute alsoReached = v;\n\
         \tattribute andW = w;\n\
         }\n",
    );
    assert!(unresolved.is_empty(), "{unresolved:?}");
}

#[test]
fn kerml_writes_new_too() {
    // `new A(x)` in KerML lexed `new` as a name, which left the corpus
    // with a reference to something that does not exist.
    let mut ws = Workspace::new();
    let file = ws.add_file(
        "e.kerml",
        "package P {\n\
         \tclass A { feature x; }\n\
         \tbehavior B { in feature x; out feature : A = new A(x); }\n\
         }\n",
    );
    ws.resolve_files(&[file]);
    let unresolved: Vec<&str> = ws.unresolved().iter().map(|u| u.name.as_str()).collect();
    assert!(unresolved.is_empty(), "{unresolved:?}");
}

#[test]
fn a_value_that_is_only_a_name_says_what_it_names() {
    // `= ledPinNumber` is a reference to a feature, not a computation.
    // The model reifies it as the standard's FeatureReferenceExpression
    // so that a reader can follow the name without resolving it again.
    use sysml_model::{ElementKind, Value};

    let mut ws = Workspace::new();
    let file = ws.add_file(
        "e.sysml",
        "package P {\n\
         \tattribute def Number;\n\
         \tattribute pinNumber : Number = 13;\n\
         \tattribute pin : Number = pinNumber;\n\
         \tattribute computed : Number = pinNumber + 1;\n\
         }\n",
    );
    ws.resolve_files(&[file]);
    assert!(ws.unresolved().is_empty(), "{:?}", ws.unresolved());

    let model = ws.model();
    let named = |want: &str| {
        model
            .ids()
            .find(|&id| model.name(id) == Some(want))
            .expect("declared")
    };
    let value_of = |usage| {
        model
            .owned(usage)
            .iter()
            .copied()
            .find(|&child| model.kind(child) == ElementKind::FeatureValue)
            .and_then(|membership| model.get(membership, "value")?.as_id())
            .expect("a declared value")
    };

    let reference = value_of(named("pin"));
    assert_eq!(
        model.kind(reference),
        ElementKind::FeatureReferenceExpression
    );
    assert_eq!(
        model.get(reference, "referent"),
        Some(&Value::Ref(named("pinNumber")))
    );

    // anything more than a name is an expression, and names nothing
    let computed = value_of(named("computed"));
    assert_eq!(model.kind(computed), ElementKind::Expression);
    assert_eq!(model.get(computed, "referent"), None);
}

#[test]
fn a_verification_case_says_what_it_verifies() {
    use sysml_model::{ElementKind, Value};

    let mut ws = Workspace::new();
    let file = ws.add_file(
        "v.sysml",
        "package P {\n\
         \tpart def Thing;\n\
         \trequirement def R { subject t : Thing; }\n\
         \trequirement def R2 { subject t : Thing; }\n\
         \tverification def V {\n\
         \t\tsubject t : Thing;\n\
         \t\tobjective {\n\
         \t\t\tverify R;\n\
         \t\t\tverify R2;\n\
         \t\t}\n\
         \t}\n\
         }\n",
    );
    ws.resolve_files(&[file]);
    assert!(ws.unresolved().is_empty(), "{:?}", ws.unresolved());

    let model = ws.model();
    let named = |want: &str| {
        model
            .ids()
            .find(|&id| model.name(id) == Some(want))
            .expect("declared")
    };
    let case = named("V");
    assert_eq!(model.kind(case), ElementKind::VerificationCaseDefinition);
    assert_eq!(
        model.get(case, "verifiedRequirement"),
        Some(&Value::RefList(vec![named("R"), named("R2")]))
    );
}

#[test]
fn a_verification_of_nothing_is_reported() {
    let unresolved = resolved(
        "package P {\n\
         \tpart def Thing;\n\
         \tverification def V {\n\
         \t\tsubject t : Thing;\n\
         \t\tobjective { verify NoSuchRequirement; }\n\
         \t}\n\
         }\n",
    );
    assert_eq!(unresolved, ["NoSuchRequirement"]);
}

/// `$` is the global namespace, and a name that starts there means the
/// same thing wherever it is written -- in an expression as much as in
/// a typing. Without it the name below reads as the `P` next door.
#[test]
fn a_name_written_from_the_root_starts_at_the_root() {
    let text = "package P {\n\
                \tattribute x;\n\
                }\n\
                package Q {\n\
                \tpackage P { attribute x; }\n\
                \tattribute a = $::P::x;\n\
                }\n";
    let mut ws = Workspace::new();
    let file = ws.add_file("e.sysml", text);
    ws.resolve_files(&[file]);
    assert_eq!(ws.unresolved().len(), 0, "{:?}", ws.unresolved());
    let at = ws
        .reference_at(
            0,
            sysml_syntax::TextSize::from(text.find("::x").unwrap() as u32 + 2),
        )
        .expect("the name in the value is a reference");
    assert_eq!(ws.qualified_name_of(at.target), "P::x");
}

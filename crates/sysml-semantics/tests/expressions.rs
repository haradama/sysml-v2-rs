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

    // anything more than a name is the invocation it is written as,
    // and names nothing itself
    let computed = value_of(named("computed"));
    assert_eq!(model.kind(computed), ElementKind::OperatorExpression);
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

/// "OperatorExpressions provide a shorthand notation for
/// InvocationExpressions that invoke a Function from the Kernel Function
/// Library": `a + b` invokes `DataFunctions::'+'`, hands it two
/// arguments through parameters that redefine the ones it takes, and
/// comes to what that function hands back.
#[test]
fn an_operator_invokes_the_function_the_specification_names_for_it() {
    use sysml_model::{ElementKind, Value};

    let mut ws = Workspace::new();
    ws.add_file(
        "functions.kerml",
        "standard library package Vals {\n\tdatatype Num;\n}\n\
         standard library package DataFunctions {\n\
         \tprivate import Vals::Num;\n\
         \tfunction '+' { in x : Num; in y : Num; return : Num; }\n}\n",
    );
    ws.add_file(
        "m.sysml",
        "package P {\n\
         \tattribute a : Vals::Num;\n\
         \tattribute b : Vals::Num;\n\
         \tattribute c = a + b;\n}\n",
    );
    ws.resolve_all();
    let named = |ws: &Workspace, want: &str| {
        ws.model()
            .ids()
            .find(|&id| ws.qualified_name_of(id) == want)
            .unwrap_or_else(|| panic!("`{want}` is declared"))
    };
    let value_of = |ws: &Workspace, of: &str| {
        let usage = named(ws, of);
        ws.model()
            .owned(usage)
            .iter()
            .copied()
            .find(|&child| ws.model().kind(child) == ElementKind::FeatureValue)
            .and_then(|membership| ws.model().get(membership, "value")?.as_id())
            .expect("a declared value")
    };
    let plus = value_of(&ws, "P::c");
    assert_eq!(ws.model().kind(plus), ElementKind::OperatorExpression);

    // an operator names no membership for what it invokes:
    // `OperatorExpression::instantiatedType()` resolves the symbol
    // against the function library instead
    assert!(
        !ws.model()
            .owned(plus)
            .iter()
            .any(|&it| ws.model().kind(it) == ElementKind::Membership),
        "the function is named by the operator, not by a membership"
    );
    assert_eq!(
        ws.qualified_name_of(named(&ws, "DataFunctions::+")),
        "DataFunctions::+"
    );

    // each argument redefines the parameter it is handed to, in order
    let redefines = |ws: &Workspace, feature: sysml_model::ElementId| {
        ws.model()
            .owned(feature)
            .iter()
            .copied()
            .find(|&it| ws.model().kind(it) == ElementKind::Redefinition)
            .and_then(|it| ws.model().get(it, "redefinedFeature")?.as_id())
            .map(|to| ws.qualified_name_of(to))
    };
    let arguments: Vec<sysml_model::ElementId> = ws
        .model()
        .owned(plus)
        .iter()
        .copied()
        .filter(|&it| ws.model().get(it, "direction") == Some(&Value::EnumLit("in")))
        .collect();
    assert_eq!(
        arguments
            .iter()
            .map(|&it| redefines(&ws, it))
            .collect::<Vec<_>>(),
        vec![
            Some("DataFunctions::+::x".to_string()),
            Some("DataFunctions::+::y".to_string())
        ]
    );

    // and what the expression comes to is what the function hands back
    let result = ws
        .model()
        .owned(plus)
        .iter()
        .copied()
        .find(|&it| ws.model().get(it, "direction") == Some(&Value::EnumLit("out")))
        .expect("the expression hands its value back");
    assert!(
        redefines(&ws, result).is_some(),
        "the result redefines the function's own"
    );
}

/// `F(q = 1, p = a)` says which parameter each argument is for, so the
/// order says nothing; `new A(...)` hands its arguments to the thing it
/// constructs rather than to itself.
#[test]
fn an_argument_that_names_its_parameter_is_read_by_the_name() {
    use sysml_model::{ElementKind, Value};

    let mut ws = Workspace::new();
    ws.add_file(
        "m.sysml",
        "package P {\n\
         \tattribute def A { attribute x; attribute y; }\n\
         \tattribute a : A;\n\
         \tcalc def F { in p : A; in q : A; return : A; }\n\
         \tattribute g = F(q = a, p = a);\n\
         \tattribute h = new A(y = a, x = a);\n}\n",
    );
    ws.resolve_all();
    let named = |ws: &Workspace, want: &str| {
        ws.model()
            .ids()
            .find(|&id| ws.qualified_name_of(id) == want)
            .unwrap_or_else(|| panic!("`{want}` is declared"))
    };
    let value_of = |ws: &Workspace, of: &str| {
        ws.model()
            .owned(named(ws, of))
            .iter()
            .copied()
            .find(|&child| ws.model().kind(child) == ElementKind::FeatureValue)
            .and_then(|membership| ws.model().get(membership, "value")?.as_id())
            .expect("a declared value")
    };
    let redefines = |ws: &Workspace, feature: sysml_model::ElementId| {
        ws.model()
            .owned(feature)
            .iter()
            .copied()
            .find(|&it| ws.model().kind(it) == ElementKind::Redefinition)
            .and_then(|it| ws.model().get(it, "redefinedFeature")?.as_id())
            .map(|to| ws.qualified_name_of(to))
    };

    // written `q` first, and `q` is what it redefines
    let call = value_of(&ws, "P::g");
    assert_eq!(ws.model().kind(call), ElementKind::InvocationExpression);
    let arguments: Vec<sysml_model::ElementId> = ws
        .model()
        .owned(call)
        .iter()
        .copied()
        .filter(|&it| ws.model().get(it, "direction") == Some(&Value::EnumLit("in")))
        .collect();
    assert_eq!(
        arguments
            .iter()
            .map(|&it| redefines(&ws, it))
            .collect::<Vec<_>>(),
        vec![Some("P::F::q".to_string()), Some("P::F::p".to_string())]
    );

    // `ConstructorResult : Feature = ArgumentList` -- the arguments are
    // the result's, and the constructor owns nothing else
    let construction = value_of(&ws, "P::h");
    assert_eq!(
        ws.model().kind(construction),
        ElementKind::ConstructorExpression
    );
    let result = ws
        .model()
        .owned(construction)
        .iter()
        .copied()
        .find(|&it| ws.model().get(it, "direction") == Some(&Value::EnumLit("out")))
        .expect("a constructor hands back what it constructs");
    assert_eq!(
        ws.model()
            .owned(result)
            .iter()
            .filter(|&&it| ws.model().kind(it) == ElementKind::Feature)
            .count(),
        2,
        "both arguments are the result's"
    );
}

/// `accept ... when c` waits for what a trigger function returns, and
/// which of the three it invokes is what the keyword says: "Return one
/// of the Functions TriggerWhen, TriggerAt or TriggerAfter ... depending
/// on whether the kind ... is when, at or after, respectively."
#[test]
fn an_accept_that_waits_invokes_the_trigger_its_keyword_names() {
    use sysml_model::{ElementKind, Value};

    let mut ws = Workspace::new();
    ws.add_file(
        "triggers.kerml",
        "standard library package Triggers {\n\
         \tfunction TriggerWhen { in changeExpression; return : Triggers; }\n\
         \tfunction TriggerAt { in timeInstant; return : Triggers; }\n\
         \tfunction TriggerAfter { in duration; return : Triggers; }\n}\n",
    );
    ws.add_file(
        "m.sysml",
        "action def A {\n\
         \tattribute ready;\n\
         \taction x;\n\
         \tthen accept when ready;\n}\n",
    );
    ws.resolve_all();
    let model = ws.model();
    let trigger = model
        .ids()
        .find(|&id| model.kind(id) == ElementKind::TriggerInvocationExpression)
        .expect("the accept waits for a trigger");
    assert_eq!(
        model.get(trigger, "kind"),
        Some(&Value::EnumLit("when")),
        "the keyword says which"
    );
    let names_it = model.owned(trigger)[0];
    assert_eq!(model.kind(names_it), ElementKind::Membership);
    let invoked = model
        .get(names_it, "memberElement")
        .and_then(Value::as_id)
        .expect("the trigger names the function it invokes");
    assert_eq!(ws.qualified_name_of(invoked), "Triggers::TriggerWhen");
    // and it hands the change expression over as its argument
    assert_eq!(
        ws.model()
            .owned(trigger)
            .iter()
            .filter(|&&it| ws.model().get(it, "direction") == Some(&Value::EnumLit("in")))
            .count(),
        1
    );
}

/// What an expression comes to, where the notation says it without
/// returning it: `x as T` selects the instances of `T`, `new A(...)`
/// constructs an `A`, and `e.f` chains to `f`.
#[test]
fn an_expression_comes_to_what_it_names() {
    use sysml_model::{ElementKind, Value};

    let mut ws = Workspace::new();
    ws.add_file(
        "functions.kerml",
        "standard library package Base {\n\tclassifier Anything;\n}\n\
         standard library package BaseFunctions {\n\
         \tprivate import Base::Anything;\n\
         \tfunction 'as' { in seq : Anything; return : Anything; }\n}\n",
    );
    ws.add_file(
        "m.sysml",
        "package P {\n\
         \tpart def A { part f; }\n\
         \tpart a;\n\
         \tattribute cast = a as A;\n\
         \tattribute made = new A();\n}\n",
    );
    ws.resolve_all();
    let named = |ws: &Workspace, want: &str| {
        ws.model()
            .ids()
            .find(|&id| ws.qualified_name_of(id) == want)
            .unwrap_or_else(|| panic!("`{want}` is declared"))
    };
    let comes_to = |ws: &mut Workspace, of: &str| {
        let usage = named(ws, of);
        let expression = ws
            .model()
            .owned(usage)
            .iter()
            .copied()
            .find(|&it| ws.model().kind(it) == ElementKind::FeatureValue)
            .and_then(|it| ws.model().get(it, "value")?.as_id())
            .expect("a declared value");
        let result = ws
            .model()
            .owned(expression)
            .iter()
            .copied()
            .find(|&it| ws.model().get(it, "direction") == Some(&Value::EnumLit("out")))
            .expect("the expression hands its value back");
        ws.supertypes(result)
            .into_iter()
            .map(|up| ws.qualified_name_of(up))
            .collect::<Vec<_>>()
    };
    assert!(
        comes_to(&mut ws, "P::cast").contains(&"P::A".to_string()),
        "a cast comes to the type it casts to"
    );
    assert!(
        comes_to(&mut ws, "P::made").contains(&"P::A".to_string()),
        "a construction comes to what it constructs"
    );
}

/// `MetadataAccessExpression = ElementReferenceMember '.' 'metadata'`
/// reads the metadata of what it names. `metadata` is a keyword rather
/// than a name, so `Foo.metadata` was a parse error and the construct
/// had nothing standing for it.
#[test]
fn a_metadata_access_names_the_element_it_reads() {
    use sysml_model::{ElementKind, Value};

    let mut ws = Workspace::new();
    let source = "package P {\n\tmetadata def Foo;\n\tattribute m = Foo.metadata;\n}\n";
    assert!(
        sysml_syntax::parse(source).ok(),
        "`Foo.metadata` is SysML the parser reads"
    );
    let file = ws.add_file("m.sysml", source);
    ws.resolve_all();
    let model = ws.model();
    let access = model
        .ids()
        .find(|&id| model.kind(id) == ElementKind::MetadataAccessExpression)
        .expect("the access is built");
    // the one membership it owns names what it reads, which is what
    // `validateMetadataAccessExpressionReferencedElement` asks for
    let names_it = model.owned(access)[0];
    assert_eq!(model.kind(names_it), ElementKind::Membership);
    let read = model
        .get(names_it, "memberElement")
        .and_then(Value::as_id)
        .expect("what it reads is named");
    assert_eq!(ws.qualified_name_of(read), "P::Foo");

    // The official corpus writes no metadata access at all, so the
    // constraint about one is asked of nothing there and holding it is
    // this test's job: it wants a membership that is not a feature
    // membership, which is the one naming what is read.
    let checked = ws.check_rules(&[file]);
    assert!(
        checked
            .held
            .contains(&"validateMetadataAccessExpressionReferencedElement"),
        "{checked:?}"
    );
    assert!(
        checked.violations.is_empty(),
        "a metadata access reading a metadata definition is sound: {:?}",
        checked.violations
    );
}

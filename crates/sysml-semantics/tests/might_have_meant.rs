//! What a name that resolved to nothing might have meant.
//!
//! Resolution reports one thing about a name it could not find: that it
//! could not find it. Two very different mistakes arrive looking
//! identical --
//!
//! ```text
//! attribute capacity : VolumeValue;   // ISQ::VolumeValue, un-imported
//! attribute temp : Temperature;       // TemperatureValue, misremembered
//! ```
//!
//! -- and a reader told only that both missed has to work out which is
//! which one name at a time. The workspace already knows: it can see
//! everything declared in it.

use sysml_semantics::Workspace;

fn resolved(text: &str) -> Workspace {
    let mut ws = Workspace::default();
    ws.add_file("m.sysml", text);
    ws.resolve_all();
    ws
}

/// A name the workspace declares somewhere is a right name in the wrong
/// scope, and that is what `elsewhere` says.
#[test]
fn a_name_that_is_declared_but_out_of_scope_is_said_to_be_declared() {
    let ws = resolved(
        "package Q { part def Wheel; }\n\
         package P { part def Car { part w : Wheel; } }\n",
    );
    let missed: Vec<String> = ws.unresolved().iter().map(|u| u.name.clone()).collect();
    assert_eq!(missed, ["Wheel"]);

    let [about] = &ws.suggestions(&missed)[..] else {
        panic!("one name, one answer");
    };
    assert_eq!(about.elsewhere, ["Q::Wheel"], "it is declared, in `Q`");
}

/// A name nothing declares is a wrong name, and what it wants is the
/// nearest thing that is declared.
#[test]
fn a_name_nothing_declares_is_offered_what_is_near_it() {
    let ws = resolved(
        "package P {\n\
         \tpart def WheelAssembly;\n\
         \tpart def Car { part w : Wheel; }\n\
         }\n",
    );
    let missed: Vec<String> = ws.unresolved().iter().map(|u| u.name.clone()).collect();
    let [about] = &ws.suggestions(&missed)[..] else {
        panic!("one name, one answer");
    };
    assert!(about.elsewhere.is_empty(), "nothing answers to `Wheel`");
    assert_eq!(about.near, ["P::WheelAssembly"]);
}

/// What begins with the name comes before what merely holds it, because
/// a name that begins with what was asked for is the one that was
/// probably meant.
#[test]
fn what_begins_with_the_name_is_offered_first() {
    let ws = resolved(
        "package P {\n\
         \tpart def LateMass;\n\
         \tpart def MassValue;\n\
         \tpart def Car { attribute m : Mass; }\n\
         }\n",
    );
    let missed: Vec<String> = ws.unresolved().iter().map(|u| u.name.clone()).collect();
    let [about] = &ws.suggestions(&missed)[..] else {
        panic!("one name, one answer");
    };
    assert_eq!(about.near, ["P::MassValue", "P::LateMass"]);
}

/// A one-letter name must not be offered for every query that happens
/// to contain its letter.
///
/// The library is full of them -- the parameters of its functions, the
/// symbols of its quantities -- and matching a declared name inside the
/// query answered `VolumeValue` with `L`, `M`, `o`, `u` and `v`.
#[test]
fn a_single_letter_is_not_near_everything() {
    let ws = resolved(
        "package P {\n\
         \tcalc def f { in v; }\n\
         \tpart def Car { attribute x : VolumeValue; }\n\
         }\n",
    );
    let missed: Vec<String> = ws.unresolved().iter().map(|u| u.name.clone()).collect();
    let [about] = &ws.suggestions(&missed)[..] else {
        panic!("one name, one answer");
    };
    assert!(
        !about.near.iter().any(|it| it.ends_with("::v")),
        "{:?}",
        about.near
    );
}

/// A name asked for with more on it than the library gives it is worth
/// answering the other way about -- but only where what is declared is
/// most of what was asked.
#[test]
fn a_declared_name_that_is_most_of_the_query_is_offered() {
    let ws = resolved(
        "package P {\n\
         \tpart def Wheel;\n\
         \tpart def Car { part w : WheelPart; }\n\
         }\n",
    );
    let missed: Vec<String> = ws.unresolved().iter().map(|u| u.name.clone()).collect();
    let [about] = &ws.suggestions(&missed)[..] else {
        panic!("one name, one answer");
    };
    assert_eq!(about.near, ["P::Wheel"]);
}

/// A qualified name misses because of its last segment, so that is what
/// is looked for: `Q::Wheel` failed over `Wheel`, not over `Q`.
#[test]
fn a_qualified_name_is_answered_by_its_last_segment() {
    let ws = resolved(
        "package Q { part def Wheel; }\n\
         package P { part def Car { part w : R::Wheel; } }\n",
    );
    let missed: Vec<String> = ws.unresolved().iter().map(|u| u.name.clone()).collect();
    let [about] = &ws.suggestions(&missed)[..] else {
        panic!("one name, one answer");
    };
    assert_eq!(about.elsewhere, ["Q::Wheel"]);
}

/// Several names at once, because the walk over every declared name is
/// what costs -- sixty thousand of them with the standard library
/// loaded, once rather than once per name.
#[test]
fn several_names_are_answered_in_one_walk() {
    let ws = resolved(
        "package Q { part def Wheel; part def Engine; }\n\
         package P { part def Car { part w : Wheel; part e : Engine; } }\n",
    );
    let missed: Vec<String> = ws.unresolved().iter().map(|u| u.name.clone()).collect();
    let answers = ws.suggestions(&missed);
    assert_eq!(answers.len(), missed.len());
    let told: Vec<&str> = answers
        .iter()
        .flat_map(|it| it.elsewhere.iter().map(String::as_str))
        .collect();
    assert!(told.contains(&"Q::Wheel"), "{told:?}");
    assert!(told.contains(&"Q::Engine"), "{told:?}");
}

/// Asked about nothing, it answers nothing -- and does not walk.
#[test]
fn nothing_asked_is_nothing_answered() {
    let ws = resolved("package P { part def Car; }\n");
    assert!(ws.suggestions(&[]).is_empty());
}

/// A name that is neither held by nor holds any declared name, because
/// the mistake was a letter rather than a word.
///
/// `Wheeel` is not a substring of `Wheel` and `Wheel` is not a substring
/// of `Wheeel`, so every rule above this one passes it over -- and it is
/// the commonest kind of wrong name there is.
#[test]
fn a_name_a_letter_wrong_is_offered_what_it_nearly_was() {
    let ws = resolved(
        "package P {\n\
         \tpart def Wheel;\n\
         \tpart def Car { part w : Wheeel; }\n\
         }\n",
    );
    let missed: Vec<String> = ws.unresolved().iter().map(|u| u.name.clone()).collect();
    let [about] = &ws.suggestions(&missed)[..] else {
        panic!("one name, one answer");
    };
    assert_eq!(about.near, ["P::Wheel"]);
}

/// Two letters the other way round is one mistake and not two, which is
/// what a pair of fingers does and what nothing above this rule catches.
#[test]
fn two_letters_swapped_is_one_name_apart() {
    let ws = resolved(
        "package P {\n\
         \tpart def Wheel;\n\
         \tpart def Car { part w : Whele; }\n\
         }\n",
    );
    let missed: Vec<String> = ws.unresolved().iter().map(|u| u.name.clone()).collect();
    let [about] = &ws.suggestions(&missed)[..] else {
        panic!("one name, one answer");
    };
    assert_eq!(about.near, ["P::Wheel"]);
}

/// And what is nearest is offered first: one letter wrong before two.
#[test]
fn what_is_nearest_is_offered_before_what_is_further() {
    let ws = resolved(
        "package P {\n\
         \tattribute def TemperatureValue;\n\
         \tattribute def TemperatureValues;\n\
         \tpart def Oven { attribute t : TemperatureVale; }\n\
         }\n",
    );
    let missed: Vec<String> = ws.unresolved().iter().map(|u| u.name.clone()).collect();
    let [about] = &ws.suggestions(&missed)[..] else {
        panic!("one name, one answer");
    };
    assert_eq!(about.near, ["P::TemperatureValue", "P::TemperatureValues"]);
}

/// And a name that is merely short is not near everything short: the
/// allowance is a third of what was asked for, never none of it.
#[test]
fn a_short_name_is_not_near_every_other_short_name() {
    let ws = resolved(
        "package P {\n\
         \tpart def Materials;\n\
         \tpart def Car { attribute m : Mas; }\n\
         }\n",
    );
    let missed: Vec<String> = ws.unresolved().iter().map(|u| u.name.clone()).collect();
    let [about] = &ws.suggestions(&missed)[..] else {
        panic!("one name, one answer");
    };
    assert!(about.near.is_empty(), "{:?}", about.near);
}

/// Names are case-sensitive, so "declared as" -- the very name, wanting
/// only an import -- is the exact spelling alone. `B` used to be "declared
/// as" the `b` being declared on the same line, and the quick fix wrote
/// `part b : b;`. Another case is a near miss, and the nearest.
#[test]
fn another_case_is_a_near_miss_not_the_name() {
    let ws = resolved("part def A;\npart a : A;\npart b : B;\n");
    let [about] = &ws.suggestions(&["B".to_string()])[..] else {
        panic!("one name, one answer");
    };
    assert!(about.elsewhere.is_empty(), "{about:?}");
    assert_eq!(about.near.first().map(String::as_str), Some("b"), "{about:?}");
}

/// What no qualified name can reach is no answer: a member of something
/// that has no name, or a private one, which no import brings in.
#[test]
fn what_cannot_be_named_from_outside_is_not_offered() {
    let ws = resolved(
        "package P {\n\
         \tprivate part def Hidden;\n\
         \tpart { part def Inside; }\n\
         }\n\
         part h : Hidden;\n\
         part i : Inside;\n",
    );
    for (name, about) in ["Hidden", "Inside"]
        .iter()
        .zip(ws.suggestions(&["Hidden".to_string(), "Inside".to_string()]))
    {
        assert!(about.elsewhere.is_empty(), "{name}: {about:?}");
        assert!(about.near.is_empty(), "{name}: {about:?}");
    }
}

/// A name that reaches the wrong kind of thing is spelt right and in
/// scope, so neither "declared as ... an import would" nor "did you mean"
/// is the answer; what it names is. A definition specializes a classifier
/// (`Subclassification::superclassifier : Classifier`), so `part def B :>
/// a;` naming a part usage names nothing -- it used to resolve -- and a
/// usage subsets a feature.
#[test]
fn a_name_of_the_wrong_kind_of_thing_says_what_it_names() {
    let ws = resolved(
        "package P {\n\
         \tpart a;\n\
         \tpart def A;\n\
         \tpart def B :> a;\n\
         \tpart b :> A;\n\
         \tpart def C :> A;\n\
         }\n",
    );
    let missed: Vec<(&str, Option<sysml_model::ElementKind>)> = ws
        .unresolved()
        .iter()
        .map(|it| (it.name.as_str(), it.wrong_kind.map(|(_, kind)| kind)))
        .collect();
    assert_eq!(
        missed,
        [
            ("a", Some(sysml_model::ElementKind::Classifier)),
            ("A", Some(sysml_model::ElementKind::Feature)),
        ]
    );
    let said = ws
        .wrong_kind_at(ws.unresolved()[0].file, ws.unresolved()[0].range)
        .unwrap();
    assert_eq!(said, "it names `P::a`, a `PartUsage`, where only a `Classifier` can stand");
    // and a name that is merely missing has nothing of the kind to say
    let ws = resolved("part x : Nope;\n");
    let nope = &ws.unresolved()[0];
    assert!(nope.wrong_kind.is_none());
    assert_eq!(ws.wrong_kind_at(nope.file, nope.range), None);
}

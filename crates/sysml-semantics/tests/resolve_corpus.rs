//! Resolution-rate regression test against the official corpus.
//! Skipped when the submodule is not checked out.

use sysml_corpus::vendor;
use sysml_semantics::Workspace;

#[test]
fn standard_library_resolves_completely() {
    let Some(root) = vendor() else { return };
    let mut ws = Workspace::new();
    ws.load_dir(&root.join("sysml.library")).unwrap();
    let stats = ws.resolve_all();
    assert_eq!(
        stats.unresolved,
        0,
        "library resolution regressed ({} resolved): {:?}",
        stats.resolved,
        &ws.unresolved()[..stats.unresolved.min(10)]
    );
}

/// Connector and transition ends are resolved too, so this covers the
/// operands of `connect`/`bind`/`allocate` and of `first ... then ...`
/// alongside every typing and specialization.
#[test]
fn examples_resolve_completely_against_the_library() {
    let Some(root) = vendor() else { return };
    let mut ws = Workspace::new();
    ws.load_dir(&root.join("sysml.library")).unwrap();
    ws.load_dir(&root.join("sysml/src")).unwrap();
    let stats = ws.resolve_all();
    assert_eq!(
        stats.unresolved,
        0,
        "combined resolution regressed ({} resolved): {:?}",
        stats.resolved,
        &ws.unresolved()[..stats.unresolved.min(10)]
    );
}

/// The KerML examples resolve too, which completes the corpus: all 403
/// files parse and every reference in any of them resolves -- and now
/// resolves to something other than itself.
#[test]
fn kerml_examples_resolve_completely_against_the_library() {
    let Some(root) = vendor() else { return };
    let mut ws = Workspace::new();
    ws.load_dir(&root.join("sysml.library")).unwrap();
    ws.load_dir(&root.join("kerml/src")).unwrap();
    let stats = ws.resolve_all();
    assert_eq!(
        stats.unresolved,
        0,
        "KerML example resolution regressed ({} resolved): {:?}",
        stats.resolved,
        &ws.unresolved()[..stats.unresolved.min(10)]
    );
}

/// A literal specializes the evaluation the library states for its kind,
/// and that evaluation is what declares the literal's result: `abstract
/// function LiteralIntegerEvaluation specializes LiteralEvaluation {
/// return : Integer[1]; }`.
///
/// Each kind of literal is a metaclass of its own, and none of them was
/// given the implicit specialization the library states, so a literal
/// specialized nothing and had no type, no result and no members at
/// all. The library states no evaluation for an infinite literal, so
/// that one is a literal evaluation and nothing narrower.
#[test]
fn a_literal_specializes_the_evaluation_the_library_states_for_it() {
    let Some(root) = vendor() else { return };
    let mut ws = Workspace::new();
    ws.load_dir(&root.join("sysml.library")).unwrap();
    let file = ws.add_file(
        "literals.sysml",
        "package L {\n\
         \tattribute i = 1;\n\
         \tattribute r = 1.5;\n\
         \tattribute s = \"a\";\n\
         \tattribute b = true;\n\
         \tattribute n [0..*];\n\
         }\n",
    );
    ws.resolve_all();
    let root = ws.file_roots(file)[0];
    let literals: Vec<sysml_model::ElementId> = ws
        .model()
        .descendants(root)
        .into_iter()
        .filter(|&id| {
            ws.model()
                .kind(id)
                .is_a(sysml_model::ElementKind::LiteralExpression)
        })
        .collect();
    let mut named: Vec<(String, String)> = Vec::new();
    for id in literals {
        let kind = ws.model().kind(id).name().to_string();
        let up = ws
            .supertypes(id)
            .first()
            .map(|&up| ws.qualified_name_of(up))
            .unwrap_or_default();
        named.push((kind, up));
    }
    named.sort();
    assert_eq!(
        named,
        [
            (
                "LiteralBoolean".to_string(),
                "Performances::literalBooleanEvaluations".to_string()
            ),
            // the library names no evaluation after an infinite
            // literal, and `checkLiteralInfinitySpecialization` says
            // which one it is all the same
            (
                "LiteralInfinity".to_string(),
                "Performances::literalIntegerEvaluations".to_string()
            ),
            (
                "LiteralInteger".to_string(),
                "Performances::literalIntegerEvaluations".to_string()
            ),
            (
                "LiteralInteger".to_string(),
                "Performances::literalIntegerEvaluations".to_string()
            ),
            (
                "LiteralRational".to_string(),
                "Performances::literalRationalEvaluations".to_string()
            ),
            (
                "LiteralString".to_string(),
                "Performances::literalStringEvaluations".to_string()
            ),
        ]
    );
}

/// The implicit specializations a usage gets are the standard library's
/// own names, and a model may declare a package of the same name: the
/// corpus has a `SimpleVehicleModel::...::Requirements` beside the
/// library's. Resolved through the local scope, a requirement there
/// specialized nothing at all.
///
/// And an enumeration value is a variant of its enumeration, but it
/// declares the value rather than naming one written elsewhere: `enum
/// def E1 { a; }` has no `a` anywhere else to bring along, and looking
/// for one reaches past the enumeration to whatever else is called `a`.
#[test]
fn a_flow_specializes_the_messages_the_library_states_for_it() {
    let Some(root) = vendor() else { return };
    let mut ws = Workspace::new();
    ws.load_dir(&root.join("sysml.library")).unwrap();
    let file = ws.add_file(
        "m.sysml",
        "package M {\n\
         \tattribute def A;\n\
         \tpart def P { out o : A; in i : A; }\n\
         \tflow def F;\n\
         \tpart p1 : P;\n\
         \tpart p2 : P;\n\
         \tflow f from p1.o to p2.i;\n\
         \tmessage m;\n\
         \tsuccession flow sf from p1.o to p2.i;\n\
         }\n",
    );
    ws.resolve_all();
    let ups = |ws: &mut Workspace, want: &str| -> Vec<String> {
        let elem = ws
            .model()
            .descendants(ws.file_roots(file)[0])
            .into_iter()
            .find(|&id| ws.model().name(id) == Some(want))
            .unwrap_or_else(|| panic!("`{want}` is declared"));
        ws.supertypes(elem)
            .iter()
            .map(|&up| ws.qualified_name_of(up))
            .collect()
    };
    // every flow usage subsets `messages`; only one with ends of its own
    // subsets `flows`, which is where the ends come from
    let f = ups(&mut ws, "f");
    assert!(
        f.contains(&"Flows::flows".to_string()) && f.contains(&"Flows::messages".to_string()),
        "a flow with ends subsets both: {f:?}"
    );
    let m = ups(&mut ws, "m");
    assert!(
        !m.contains(&"Flows::flows".to_string()) && m.contains(&"Flows::messages".to_string()),
        "a message subsets `messages` alone: {m:?}"
    );
    let sf = ups(&mut ws, "sf");
    assert!(
        sf.contains(&"Flows::successionFlows".to_string()),
        "a succession flow subsets the succession flows: {sf:?}"
    );
    // a definition specializes definitions: the general one, since it
    // declares no ends of its own
    let d = ups(&mut ws, "F");
    assert!(
        d.contains(&"Flows::MessageAction".to_string())
            && !d.contains(&"Flows::Message".to_string()),
        "a flow definition with no ends specializes `MessageAction`: {d:?}"
    );

    // `Flows::Flow` is a *sub*class of `Flows::Message`, so implying it
    // of every flow made the library's own `messages` inherit the ends
    // of the type that specializes it
    let messages = ws
        .model()
        .ids()
        .find(|&id| ws.qualified_name_of(id) == "Flows::messages")
        .expect("the library declares `messages`");
    let ends: Vec<String> = ws
        .supertypes(messages)
        .iter()
        .map(|&up| ws.qualified_name_of(up))
        .collect();
    assert!(
        !ends.iter().any(|it| it == "Flows::Flow"),
        "`messages` does not specialize what specializes it: {ends:?}"
    );
}

#[test]
fn what_the_standard_implies_is_named_from_the_root() {
    let Some(root) = vendor() else { return };
    let mut ws = Workspace::new();
    ws.load_dir(&root.join("sysml.library")).unwrap();
    let file = ws.add_file(
        "m.sysml",
        "package M {\n\
         \tpackage Requirements {\n\
         \t\trequirement r;\n\
         \t}\n\
         \tenum def E { a; }\n\
         \tpackage Other { attribute a; }\n\
         }\n",
    );
    ws.resolve_all();
    let named = |ws: &Workspace, want: &str| {
        ws.model()
            .descendants(ws.file_roots(file)[0])
            .into_iter()
            .find(|&id| ws.model().name(id) == Some(want))
            .unwrap_or_else(|| panic!("`{want}` is declared"))
    };
    // the library's `Requirements`, not the model's
    let (r, value) = (named(&ws, "r"), named(&ws, "a"));
    let ups: Vec<String> = ws
        .supertypes(r)
        .iter()
        .map(|&up| ws.qualified_name_of(up))
        .collect();
    assert!(
        ups.contains(&"Requirements::RequirementCheck".to_string()),
        "a requirement specializes the library's check: {ups:?}"
    );
    // and the enumeration value brings nothing along
    let ups: Vec<String> = ws
        .supertypes(value)
        .iter()
        .map(|&up| ws.qualified_name_of(up))
        .collect();
    assert!(
        !ups.iter().any(|it| it.ends_with("Other::a")),
        "an enumeration value declares itself: {ups:?}"
    );
}

/// Every end of a connection participates in the link it makes.
///
/// "If a Feature has isEnd = true and an owningType that is an
/// Association or a Connector, then it must directly or indirectly
/// specialize `Links::Link::participant`", and the semantics section
/// writes an N-ary association out "with implied relationships
/// included" as one `end feature eN[1..1] subsets
/// Links::Link::participant;` per end.
///
/// The first two reach it through the `source` and `target` they are
/// made to redefine, which the library writes as `subsets participant`.
/// A third redefines nothing, because nothing above it has a third, and
/// without this it has no supertype at all and so no type -- which is
/// what `validateAssociationEndTypes` asks for, and what it reported of
/// `ConnectionTest.sysml`, a model of the corpus and therefore sound.
#[test]
fn every_end_of_a_connection_participates_in_the_link_it_makes() {
    let Some(root) = vendor() else { return };
    let mut ws = Workspace::new();
    ws.load_dir(&root.join("sysml.library")).unwrap();
    let file = ws.add_file(
        "n-ary.sysml",
        "package P {\n\tabstract connection def C {\n\
         \t\tpart p;\n\t\tend end1;\n\t\tend end2;\n\t\tend end3;\n\t}\n}\n",
    );
    ws.resolve_all();

    let named = |ws: &Workspace, want: &str| {
        ws.file_elements(file)
            .iter()
            .copied()
            .find(|&id| ws.model().name(id) == Some(want))
            .unwrap_or_else(|| panic!("`{want}` is declared"))
    };
    for end in ["end1", "end2", "end3"] {
        let id = named(&ws, end);
        let mut reached = vec![id];
        let mut at = 0;
        while at < reached.len() {
            let up = reached[at];
            at += 1;
            for over in ws.supertypes(up) {
                if !reached.contains(&over) {
                    reached.push(over);
                }
            }
        }
        let names: Vec<String> = reached.iter().map(|&it| ws.qualified_name_of(it)).collect();
        assert!(
            names.contains(&"Links::Link::participant".to_string()),
            "{end} participates: {names:?}"
        );
    }
}

/// A metadata annotation is a feature in KerML and a usage in SysML,
/// and each takes the base its own language names.
///
/// The library says which: "MetadataItem is the base type of all
/// MetadataDefinitions", and "metadataItems is the base feature of all
/// MetadataUsages". Given the type where the feature belongs, a usage
/// carried a second metaclass among its types beside the one it names
/// -- a `MetadataDefinition` is a `Metaclass` -- and given the SysML
/// metaclass in a KerML file, `#atom classifier MyBike;` took the SysML
/// base as well.
///
/// Neither shows while the implied relationships are only computed.
/// Written down, as `export` writes them, `validateMetadataFeature
/// Metaclass` reported seventy-two models of the corpus, which are
/// sound.
#[test]
fn a_metadata_annotation_takes_the_base_its_own_language_names() {
    let Some(root) = vendor() else { return };
    let mut ws = Workspace::new();
    ws.load_dir(&root.join("sysml.library")).unwrap();
    let sysml = ws.add_file(
        "m.sysml",
        "package P {\n\tmetadata def C;\n\t#C part def Q;\n}\n",
    );
    let kerml = ws.add_file(
        "m.kerml",
        "package K {\n\tmetaclass A;\n\t#A classifier B;\n\tmetadata m : A;\n}\n",
    );
    ws.resolve_all();

    let one = |ws: &Workspace, file: usize, want: sysml_model::ElementKind| {
        ws.model()
            .ids()
            .filter(|&id| ws.element_file(id) == Some(file))
            .find(|&id| ws.model().kind(id) == want)
            .unwrap_or_else(|| panic!("{want:?} is built"))
    };
    // the SysML one is a usage, and subsets the base feature
    let usage = one(&ws, sysml, sysml_model::ElementKind::MetadataUsage);
    let ups: Vec<String> = ws
        .supertypes(usage)
        .iter()
        .map(|&up| ws.qualified_name_of(up))
        .collect();
    assert!(
        ups.contains(&"Metadata::metadataItems".to_string()),
        "a usage subsets the base feature: {ups:?}"
    );
    // and the KerML one is a feature, which takes no SysML base at all --
    // `metadata C;` declares one the same way, so it is a feature too
    let feature = one(&ws, kerml, sysml_model::ElementKind::MetadataFeature);
    let ups: Vec<String> = ws
        .supertypes(feature)
        .iter()
        .map(|&up| ws.qualified_name_of(up))
        .collect();
    assert!(
        !ups.iter().any(|it| it.starts_with("Metadata::")),
        "a KerML metadata feature is not a SysML usage: {ups:?}"
    );
}

/// What `export` writes down is a model the standard accepts.
///
/// Resolution works out the implied relationships without materializing
/// them, so a defect in what they would be shows nowhere -- until
/// `materialize_implied` writes them and something reads them back.
/// Written down, the corpus drew seventy-three violations: a metadata
/// usage given the base type where the base feature belongs, a KerML
/// metadata annotation and declaration read as the SysML usage, and a
/// conjugated type given a supertype the constraint about conjugation
/// forbids it.
///
/// The corpus is sound, so this is the checker being wrong about what
/// it wrote, and the number that says so is zero.
#[test]
fn the_implied_relationships_written_down_are_a_model_the_standard_accepts() {
    let Some(root) = vendor() else { return };
    for examples in ["sysml/src", "kerml/src"] {
        let mut ws = Workspace::new();
        let mut files = ws.load_dir(&root.join("sysml.library")).unwrap();
        files += ws.load_dir(&root.join(examples)).unwrap();
        ws.resolve_all();
        assert!(
            ws.materialize_implied() > 0,
            "{examples}: something is written"
        );

        let checked = ws.check_rules(&(0..files).collect::<Vec<_>>());
        let said: Vec<String> = checked
            .violations
            .iter()
            .map(|violation| {
                format!(
                    "{} of `{}`",
                    violation.rule,
                    ws.qualified_name_of(violation.element)
                )
            })
            .collect();
        assert!(said.is_empty(), "{examples}:\n{}", said.join("\n"));
    }
}

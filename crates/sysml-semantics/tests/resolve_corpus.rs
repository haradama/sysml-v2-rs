//! Resolution-rate regression test against the official corpus.
//! Skipped when the submodule is not checked out.

use std::path::Path;

use sysml_semantics::Workspace;

fn vendor() -> Option<std::path::PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/sysml-v2-release");
    if root.exists() {
        Some(root)
    } else {
        eprintln!("skipping: {} not checked out", root.display());
        None
    }
}

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
            (
                "LiteralInfinity".to_string(),
                "Performances::literalEvaluations".to_string()
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

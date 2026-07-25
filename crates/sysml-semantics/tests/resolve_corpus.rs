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

/// Connector and transition ends are resolved too, and 28 of them do not
/// yet land, all in example files and all about messages and events:
///
/// - `bind` operands walking four levels into a behaviour to reach a
///   payload parameter the standard library contributes
///   (`...::publish::sentMessage`, `...::accepter::acceptedMessage`)
/// - `sourceEvent`/`targetEvent` and the message usages carrying them
///
/// The number is asserted exactly: it must fall as the resolver improves,
/// and any new gap fails the test.
const KNOWN_UNRESOLVED_ENDS: usize = 28;

#[test]
fn examples_resolve_against_the_library() {
    let Some(root) = vendor() else { return };
    let mut ws = Workspace::new();
    ws.load_dir(&root.join("sysml.library")).unwrap();
    ws.load_dir(&root.join("sysml/src")).unwrap();
    let stats = ws.resolve_all();
    assert_eq!(
        stats.unresolved,
        KNOWN_UNRESOLVED_ENDS,
        "combined resolution moved ({} resolved): {:?}",
        stats.resolved,
        &ws.unresolved()[..stats.unresolved.min(10)]
    );
}

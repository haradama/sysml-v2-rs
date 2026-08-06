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

/// The KerML examples are not there yet: two references need semantics
/// this resolver does not have -- a feature reachable through the type it
/// is `featured by`, and an import whose path starts at a name another
/// import brought in. The count is pinned so that the ten which used to
/// fail alongside them cannot come back, and so that these two stay
/// visible rather than being quietly tolerated.
#[test]
fn kerml_examples_resolve_but_for_two_known_references() {
    let Some(root) = vendor() else { return };
    let mut ws = Workspace::new();
    ws.load_dir(&root.join("sysml.library")).unwrap();
    ws.load_dir(&root.join("kerml/src")).unwrap();
    let stats = ws.resolve_all();
    assert!(
        stats.unresolved <= 2,
        "KerML example resolution regressed ({} unresolved of {}): {:?}",
        stats.unresolved,
        stats.resolved + stats.unresolved,
        &ws.unresolved()[..stats.unresolved.min(10)]
    );
}

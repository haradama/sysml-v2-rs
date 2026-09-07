//! The specification's constraints, run over the specification's own
//! models.
//!
//! Every file the OMG publishes is well-formed SysML by construction, so
//! a constraint that fires on one of them is this checker being wrong
//! and not the corpus. That is the whole test: there are no negative
//! examples to check against, and this is the one ground truth there is.
//!
//! Skipped when the submodule is not checked out.

use std::path::{Path, PathBuf};

use sysml_semantics::Workspace;

fn corpus() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/sysml-v2-release");
    if root.join("sysml.library").is_dir() {
        return Some(root);
    }
    eprintln!("skipping: {} not checked out", root.display());
    None
}

#[test]
fn no_constraint_fires_on_the_models_the_specification_publishes() {
    let Some(root) = corpus() else { return };
    let mut ws = Workspace::new();
    let mut files = ws
        .load_dir(&root.join("sysml.library"))
        .expect("the library loads");
    files += ws
        .load_dir(&root.join("sysml/src"))
        .expect("the SysML examples load");
    files += ws
        .load_dir(&root.join("kerml/src"))
        .expect("the KerML examples load");
    assert!(files > 400, "the corpus is there: {files} files");
    ws.resolve_all();

    let checked = ws.check_rules(&(0..files).collect::<Vec<_>>());
    let said: Vec<String> = checked
        .violations
        .iter()
        .map(|violation| {
            format!(
                "{} of `{}`: {}",
                violation.rule,
                ws.qualified_name_of(violation.element),
                violation.says
            )
        })
        .collect();
    assert!(
        said.is_empty(),
        "{} constraint(s) fire on models that are well-formed by \
         construction, so the fault is here:\n{}",
        said.len(),
        said.join("\n")
    );

    // and something was actually asked -- a checker that evaluates
    // nothing also reports nothing
    assert!(!checked.held.is_empty(), "{checked:?}");
    // What could not be answered is named, with the property of the
    // abstract syntax this model does not build. That list is the work
    // it would take to run the rest of them.
    assert!(
        checked.unevaluated.iter().all(|(_, why)| !why.is_empty()),
        "{checked:?}"
    );
}

//! The copy is the release, byte for byte.
//!
//! `library/` is a copy of `vendor/sysml-v2-release/sysml.library`, and a
//! copy is a thing that drifts. Nothing else here would notice: a stale
//! library resolves every name in the corpus just as well as a current
//! one, because the corpus is stale with it. What would notice is a user
//! writing against the release this claims to be and finding a name that
//! is not there.
//!
//! So the two are compared whenever the submodule is checked out. Where
//! it is not, this skips -- the copy is what ships, and the submodule is
//! only how it is checked.

use std::path::{Path, PathBuf};

/// The library inside the vendored release, when it is checked out.
fn vendored() -> Option<PathBuf> {
    let at =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/sysml-v2-release/sysml.library");
    if at.is_dir() {
        return Some(at);
    }
    eprintln!("skipping: {} is not checked out", at.display());
    None
}

fn model_files(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            model_files(&path, root, out);
        } else if matches!(
            path.extension().and_then(|it| it.to_str()),
            Some("sysml" | "kerml")
        ) {
            let name = path
                .strip_prefix(root)
                .expect("walked from the root")
                .to_string_lossy()
                .replace('\\', "/");
            out.push((
                name,
                std::fs::read_to_string(&path).expect("a readable file"),
            ));
        }
    }
}

#[test]
fn what_is_bundled_is_what_the_release_holds() {
    let Some(root) = vendored() else { return };
    let mut theirs = Vec::new();
    model_files(&root, &root, &mut theirs);
    theirs.sort();

    let ours: Vec<(String, String)> = sysml_stdlib::FILES
        .iter()
        .map(|(name, text)| ((*name).to_string(), (*text).to_string()))
        .collect();

    let named = |them: &[(String, String)]| -> Vec<String> {
        them.iter().map(|(name, _)| name.clone()).collect()
    };
    assert_eq!(
        named(&ours),
        named(&theirs),
        "the bundled library holds different files from the release"
    );
    for ((name, ours), (_, theirs)) in ours.iter().zip(&theirs) {
        assert_eq!(
            ours, theirs,
            "`{name}` differs from the release it was copied from"
        );
    }
}

/// And it is the release this crate says it is.
///
/// A copy that is current but described as something else is the same
/// problem one step along: a reader who reproduces an answer needs the
/// name of what produced it.
#[test]
fn the_release_it_names_is_the_one_checked_out() {
    let Some(root) = vendored() else { return };
    let asked = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root.parent().expect("the release above its library"))
        .output();
    let Ok(out) = asked else {
        eprintln!("skipping: no git to ask");
        return;
    };
    if !out.status.success() {
        eprintln!("skipping: the release is not a git checkout here");
        return;
    }
    let commit = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(
        commit,
        sysml_stdlib::RELEASE_COMMIT,
        "the submodule is at `{commit}`, which is not the {} release",
        sysml_stdlib::RELEASE
    );
}

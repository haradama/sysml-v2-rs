//! Where the vendored corpus is, for the tests that read it.
//!
//! Every crate here tests against the official SysML-v2-Release, which
//! arrives as a git submodule and may not be checked out. Asking whether
//! it is there is one question, and it was being answered five times by
//! five copies that had drifted apart: some canonicalised the path and
//! some did not; some probed the release directory and some the library
//! inside it, so a submodule checked out but empty was a skip for four and
//! a failure for the fifth; some said out loud that they were skipping and
//! some went quiet. One walked the tree with `Path::is_dir`, which follows
//! a link and so goes round in circles.
//!
//! So it is answered here, once. This crate is `publish = false` and only
//! ever a dev-dependency, and a path dev-dependency carries no version, so
//! `cargo publish` drops it.

// Nothing here needs `unsafe`, and saying so is what keeps it that way.
#![forbid(unsafe_code)]
// Every public item carries a line saying what it is for. The two
// crates that do not turn this on are `sysml-syntax`, whose public
// surface is two hundred and seventy-nine syntax kinds whose names are
// the documentation, and `sysml-model`, whose is generated from the
// metamodel and would want the generator to write it.
#![warn(missing_docs)]
use std::path::{Path, PathBuf};

/// The vendored release, when the submodule is checked out.
///
/// Corpus tests skip rather than fail without it, and they all skip for
/// the same reason and say so in the same words.
pub fn vendor() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/sysml-v2-release");
    announced(&root)
}

/// [`checked_out`], saying out loud when the answer is no.
///
/// A test that returns early without a word is indistinguishable from a
/// test that passed, and the reader of a CI log is the one who has to
/// tell them apart.
pub fn announced(root: &Path) -> Option<PathBuf> {
    let found = checked_out(root);
    if found.is_none() {
        eprintln!("skipping: {} is not checked out", root.display());
    }
    found
}

/// The release at `root`, if what is there is a release at all.
///
/// The standard library is what is probed, rather than the directory
/// that holds it: an uninitialised submodule leaves the directory
/// behind and empties it, so its being there says nothing. The release
/// is wanted for its *examples* now -- the library is `sysml-stdlib`'s
/// -- but it is still the surest sign that the submodule came down.
///
/// The path comes back canonical. A test that hands it to a subprocess
/// and then compares the paths in the answer has to be given the
/// spelling the subprocess will use, and a `..` in the middle is not it.
pub fn checked_out(root: &Path) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    root.join("sysml.library").is_dir().then_some(root)
}

/// The standard library.
///
/// Not the release's copy, but `sysml-stdlib`'s: that is the one that
/// ships, the one a user has, and the one the tool falls back on. A test
/// resolving against a different copy would be a test about something
/// nobody runs.
///
/// It is checked in, so it is always there -- which is why this is not an
/// `Option`, and why a test needing only the library no longer skips in a
/// checkout with no submodule. Canonical, for the same reason [`vendor`]
/// is: a test that hands the path to a subprocess has to be given the
/// spelling the subprocess will use.
pub fn library() -> PathBuf {
    let at = Path::new(env!("CARGO_MANIFEST_DIR")).join("../sysml-stdlib/library");
    at.canonicalize()
        .unwrap_or_else(|why| panic!("{} is part of this workspace: {why}", at.display()))
}

/// The official SysML example, training and validation models.
pub fn sysml_examples() -> Option<PathBuf> {
    Some(vendor()?.join("sysml/src"))
}

/// The KerML ones beside them.
pub fn kerml_examples() -> Option<PathBuf> {
    Some(vendor()?.join("kerml/src"))
}

/// Every official example model under `root`, in a stable order.
pub fn models(root: &Path) -> Vec<PathBuf> {
    let mut found = model_files(&root.join("sysml/src"));
    found.extend(model_files(&root.join("kerml/src")));
    found.sort();
    found
}

/// Those and the standard library together: all 403 files.
///
/// The examples come from the release and the library from
/// `sysml-stdlib`, which is where each of them lives.
pub fn everything(root: &Path) -> Vec<PathBuf> {
    let mut found = models(root);
    found.extend(model_files(&library()));
    found.sort();
    found
}

/// Every `.sysml`/`.kerml` file under `dir`, in a stable order.
///
/// A directory that is not there is no files rather than an error: a
/// caller asking for the KerML half of a release that has none is
/// asking a fair question.
pub fn model_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(dir, &mut found);
    found.sort();
    found
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Only a directory that is one is entered. `is_dir` follows a
        // link, and a walk that follows one goes round in circles --
        // loading every file once per level the kernel allows, or never
        // coming back at all.
        let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
        if is_dir {
            walk(&path, out);
        } else if matches!(
            path.extension().and_then(|it| it.to_str()),
            Some("sysml" | "kerml")
        ) {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The answer for a checkout that has no submodule, which is the one
    /// the real `vendor` cannot give while the submodule is there.
    #[test]
    fn a_release_that_is_not_there_is_not_a_release() {
        let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-checkout");
        assert_eq!(announced(&missing), None);
    }

    /// And neither is a directory that exists but holds no library --
    /// which is what an uninitialised submodule leaves behind.
    #[test]
    fn a_directory_without_the_library_is_not_a_release() {
        assert_eq!(checked_out(Path::new(env!("CARGO_MANIFEST_DIR"))), None);
    }

    #[test]
    fn the_walk_takes_the_model_files_and_sorts_them() {
        let dir = std::env::temp_dir().join("sysml-corpus-walk-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("below")).unwrap();
        std::fs::write(dir.join("b.sysml"), "").unwrap();
        std::fs::write(dir.join("a.kerml"), "").unwrap();
        std::fs::write(dir.join("notes.txt"), "").unwrap();
        std::fs::write(dir.join("below/c.sysml"), "").unwrap();

        let found = model_files(&dir);
        let names: Vec<&str> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, ["a.kerml", "b.sysml", "c.sysml"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A link to a directory is not followed, so a tree that points at
    /// itself is walked once rather than for ever.
    #[cfg(unix)]
    #[test]
    fn a_link_to_a_directory_is_not_walked_into() {
        let dir = std::env::temp_dir().join("sysml-corpus-link-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.sysml"), "").unwrap();
        std::os::unix::fs::symlink(&dir, dir.join("round")).unwrap();

        assert_eq!(model_files(&dir).len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Where there is nothing to read there are no files, and no panic.
    #[test]
    fn a_directory_that_is_not_there_holds_no_models() {
        assert!(model_files(Path::new("/no/such/directory")).is_empty());
    }

    /// The release itself, when it is there: the counts the README
    /// states, which is what tells a walk that quietly stopped early
    /// from one that read the whole tree.
    #[test]
    fn the_release_holds_the_files_the_readme_counts() {
        let Some(root) = vendor() else { return };
        assert_eq!(everything(&root).len(), 403);
        assert!(!models(&root).is_empty());
        assert!(sysml_examples().is_some());
        assert!(kerml_examples().is_some());
    }

    /// The library is not the release's, so it is there whether the
    /// submodule is or not -- which is why the tests that need only the
    /// library no longer skip.
    #[test]
    fn the_library_is_there_without_a_submodule() {
        assert_eq!(model_files(&library()).len(), 94);
    }
}

//! Smoke test: drive the `sysml` binary over the vendored SysML-v2-Release
//! corpus.
//!
//! The per-crate corpus regressions (`sysml-syntax`, `sysml-semantics`, ...)
//! call the parser directly. This one goes through the shipped binary, so file
//! discovery, dialect selection from the extension and the reported statistics
//! are exercised the way a user hits them.
//!
//! Requires the submodule:
//!
//! ```sh
//! git submodule update --init --depth 1
//! ```
//!
//! The test skips when `vendor/` is absent — packaged crates do not ship it —
//! but a checkout that is present and truncated fails instead of passing
//! silently.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A corpus smaller than this means a broken checkout, not a shrunken release.
const MIN_FILES: usize = 100;

fn sysml(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sysml"))
        .args(args)
        .output()
        .unwrap()
}

fn vendor_root() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/sysml-v2-release");
    if root.is_dir() {
        Some(root)
    } else {
        eprintln!(
            "skipping vendor corpus smoke test: {} not checked out",
            root.display()
        );
        None
    }
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("sysml" | "kerml")
        ) {
            out.push(path);
        }
    }
}

/// Every `.sysml`/`.kerml` file the vendored release ships, whatever the
/// release's directory layout happens to be.
fn corpus_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_files(root, &mut files);
    files.sort();
    assert!(
        files.len() >= MIN_FILES,
        "only {} corpus file(s) under {}: the submodule looks truncated \
         (re-run `git submodule update --init --depth 1`)",
        files.len(),
        root.display()
    );
    files
}

fn count_by_extension(files: &[PathBuf]) -> BTreeMap<String, usize> {
    let mut by_ext = BTreeMap::new();
    for path in files {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap();
        *by_ext.entry(ext.to_string()).or_insert(0) += 1;
    }
    by_ext
}

#[test]
fn corpus_subcommand_reports_a_clean_parse_rate() {
    let Some(root) = vendor_root() else { return };
    let by_ext = count_by_extension(&corpus_files(&root));
    // the release carries both notations; only one means a partial checkout
    assert!(
        by_ext.contains_key("sysml") && by_ext.contains_key("kerml"),
        "expected .sysml and .kerml files under {}, found {:?}",
        root.display(),
        by_ext.keys().collect::<Vec<_>>()
    );

    let out = sysml(&["corpus", root.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "`sysml corpus` exited with {}:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    // `corpus` exits 0 even when files fail to parse, so the printed rate --
    // not the exit code -- is what this asserts on.
    let stdout = String::from_utf8_lossy(&out.stdout);
    for (ext, count) in &by_ext {
        let expected = format!(".{ext}: {count}/{count} files ok (100.0%), 0 total error(s)");
        assert!(
            stdout.contains(&expected),
            "expected `{expected}`, got:\n{stdout}"
        );
    }
}

#[test]
fn parse_subcommand_succeeds_on_every_corpus_file() {
    let Some(root) = vendor_root() else { return };
    let files = corpus_files(&root);

    let mut args = vec!["parse"];
    args.extend(files.iter().map(|path| path.to_str().unwrap()));
    let out = sysml(&args);

    let stderr = String::from_utf8_lossy(&out.stderr);
    let problems: Vec<&str> = stderr
        .lines()
        .filter(|line| line.contains("error:"))
        .take(20)
        .collect();
    assert!(
        out.status.success(),
        "`sysml parse` reported errors over {} corpus file(s):\n{}",
        files.len(),
        problems.join("\n")
    );
}

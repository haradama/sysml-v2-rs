//! Regression test against the official SysML-v2-Release corpus.
//!
//! Requires the `vendor/sysml-v2-release` git submodule; the test is skipped
//! (with a note) when the submodule is not checked out.

use std::path::Path;

use sysml_corpus::model_files;
use sysml_syntax::{parse_dialect, Dialect};

fn check_corpus(subdir: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/sysml-v2-release")
        .join(subdir);
    if !root.exists() {
        eprintln!("skipping corpus test: {} not checked out", root.display());
        return;
    }
    let files = model_files(&root);
    assert!(
        !files.is_empty(),
        "no corpus files under {}",
        root.display()
    );

    let mut failures = Vec::new();
    for path in &files {
        let text = std::fs::read_to_string(path).unwrap();
        let dialect = Dialect::from_path(path);
        let parse = parse_dialect(&text, dialect);
        assert_eq!(
            parse.syntax().text().to_string(),
            text,
            "lossless round-trip violated for {}",
            path.display()
        );
        if !parse.ok() {
            failures.push(format!(
                "{}: {} error(s), first: {}",
                path.display(),
                parse.errors().len(),
                parse.errors()[0].message
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} corpus files failed to parse:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}

#[test]
fn standard_libraries_parse_cleanly() {
    check_corpus("sysml.library");
}

#[test]
fn official_sysml_examples_parse_cleanly() {
    check_corpus("sysml/src");
}

#[test]
fn official_kerml_examples_parse_cleanly() {
    check_corpus("kerml/src");
}

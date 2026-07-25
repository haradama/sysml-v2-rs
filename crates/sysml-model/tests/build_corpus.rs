//! The AST→model builder must handle every file of the official corpus
//! without panicking. Skipped when the submodule is not checked out.

use std::path::{Path, PathBuf};

use sysml_model::build_model;
use sysml_syntax::{parse_dialect, Dialect};

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

#[test]
fn builds_models_for_the_whole_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/sysml-v2-release");
    if !root.exists() {
        eprintln!("skipping: {} not checked out", root.display());
        return;
    }
    let mut files = Vec::new();
    collect_files(&root, &mut files);
    assert!(!files.is_empty());

    let mut elements = 0usize;
    for path in &files {
        let text = std::fs::read_to_string(path).unwrap();
        let dialect = match path.extension().and_then(|e| e.to_str()) {
            Some("kerml") => Dialect::KerML,
            _ => Dialect::SysML,
        };
        let parse = parse_dialect(&text, dialect);
        let (model, roots) = build_model(&parse);
        assert!(!roots.is_empty(), "no root elements for {}", path.display());
        elements += model.len();
    }
    // the corpus is large; make sure we actually built something substantial
    assert!(elements > 10_000, "only {elements} elements built");
}

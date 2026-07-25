//! Formatter guarantees over the whole official corpus: reparse
//! equivalence (identical non-trivia token streams, no new errors) and
//! idempotency. Skipped when the submodule is not checked out.

use std::path::{Path, PathBuf};

use sysml_syntax::{fmt::format, parse_dialect, Dialect, SyntaxKind};

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

fn tokens(parse: &sysml_syntax::Parse) -> Vec<(SyntaxKind, String)> {
    parse
        .syntax()
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia())
        .map(|t| (t.kind(), t.text().to_string()))
        .collect()
}

#[test]
fn corpus_formats_safely_and_idempotently() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/sysml-v2-release");
    if !root.exists() {
        eprintln!("skipping: {} not checked out", root.display());
        return;
    }
    let mut files = Vec::new();
    collect_files(&root, &mut files);
    files.sort();
    assert!(!files.is_empty());

    for path in &files {
        let text = std::fs::read_to_string(path).unwrap();
        let dialect = match path.extension().and_then(|e| e.to_str()) {
            Some("kerml") => Dialect::KerML,
            _ => Dialect::SysML,
        };
        let original = parse_dialect(&text, dialect);
        let formatted = format(&text, dialect);
        let reparsed = parse_dialect(&formatted, dialect);

        assert_eq!(
            reparsed.errors().len(),
            original.errors().len(),
            "formatting introduced parse errors in {}",
            path.display()
        );
        assert_eq!(
            tokens(&original),
            tokens(&reparsed),
            "formatting changed the token stream of {}",
            path.display()
        );
        let twice = format(&formatted, dialect);
        assert_eq!(
            twice,
            formatted,
            "formatting is not idempotent for {}",
            path.display()
        );
    }
}

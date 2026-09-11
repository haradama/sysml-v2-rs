//! Formatter guarantees over the whole official corpus: reparse
//! equivalence (identical non-trivia token streams, no new errors) and
//! idempotency. Skipped when the submodule is not checked out.
//!
//! A comment is compared by what it says rather than by how it was
//! drawn: the formatter brings its interior under the column its `/*`
//! ends up in, and the margin it redraws is not part of the text -- the
//! model reads a body with that margin taken off.

use sysml_corpus::{model_files, vendor};
use sysml_syntax::{fmt::format, parse_dialect, Dialect, SyntaxKind};

fn tokens(parse: &sysml_syntax::Parse) -> Vec<(SyntaxKind, String)> {
    parse
        .syntax()
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia())
        .map(|t| {
            let text = match t.kind() {
                SyntaxKind::COMMENT_BODY => sysml_syntax::comment_text(t.text()),
                _ => t.text().to_string(),
            };
            (t.kind(), text)
        })
        .collect()
}

#[test]
fn corpus_formats_safely_and_idempotently() {
    let Some(root) = vendor() else { return };
    let files = model_files(&root);
    assert!(!files.is_empty());

    for path in &files {
        let text = std::fs::read_to_string(path).unwrap();
        let dialect = Dialect::from_path(path);
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
        // Every break the formatter writes is `\n`. A `\r` survives only
        // inside a comment body or a block note, whose interior is one
        // token's text and is emitted as the author wrote it -- a `//`
        // note used to carry the `\r` of a CRLF line ending too, which
        // left twenty of these files formatted with both endings mixed.
        let stray: Vec<String> = sysml_syntax::lex_dialect(&formatted, dialect)
            .0
            .into_iter()
            .filter(|t| !matches!(t.kind, SyntaxKind::COMMENT_BODY | SyntaxKind::BLOCK_NOTE))
            .filter(|t| formatted[t.range.clone()].contains('\r'))
            .map(|t| format!("{:?} at {:?}", t.kind, t.range))
            .collect();
        assert!(
            stray.is_empty(),
            "formatting left a carriage return outside a comment in {}: {stray:?}",
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

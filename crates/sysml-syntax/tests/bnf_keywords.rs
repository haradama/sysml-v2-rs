//! The lexer's keyword table against the specification's own BNF.
//!
//! The SysML-v2-Release ships the reserved-keyword lists of both textual
//! notations (`bnf/*.kebnf`, extracted from the KerML and SysML
//! specifications). Every word they reserve must be a keyword here, and
//! every keyword here beyond their union must be one of the contextual
//! words the grammar spells inline (`{ kind = 'guard' }`) -- which the
//! parser must keep usable as a plain name.
//!
//! Skipped when the corpus submodule is not checked out.

use std::collections::BTreeSet;
use std::path::Path;

/// The words a `RESERVED_KEYWORD =` production reserves.
fn reserved(path: &Path) -> BTreeSet<String> {
    let text = std::fs::read_to_string(path).unwrap();
    let block = text
        .split("RESERVED_KEYWORD =")
        .nth(1)
        .expect("the BNF lists its reserved keywords")
        .split("\n\n")
        .next()
        .unwrap();
    let mut out = BTreeSet::new();
    let mut rest = block;
    while let Some(open) = rest.find('\'') {
        let after = &rest[open + 1..];
        let close = after.find('\'').expect("quotes are balanced");
        out.insert(after[..close].to_string());
        rest = &after[close + 1..];
    }
    out
}

/// Keywords the lexer knows beyond the reserved lists: the specification
/// spells them inline as transition-feature and constraint kinds, without
/// reserving them.
const CONTEXTUAL: [&str; 4] = ["assumption", "effect", "guard", "trigger"];

#[test]
fn the_lexer_reserves_exactly_what_the_specification_does() {
    let bnf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/sysml-v2-release/bnf");
    if !bnf.is_dir() {
        eprintln!("skipping: {} not checked out", bnf.display());
        return;
    }
    let sysml = reserved(&bnf.join("SysML-textual-bnf.kebnf"));
    let kerml = reserved(&bnf.join("KerML-textual-bnf.kebnf"));
    assert!(sysml.len() > 100, "the SysML list looks truncated");
    assert!(kerml.len() > 80, "the KerML list looks truncated");
    let union: BTreeSet<&str> = sysml.union(&kerml).map(String::as_str).collect();

    // every reserved word is a keyword of the lexer
    let missing: Vec<&&str> = union
        .iter()
        .filter(|word| sysml_syntax::SyntaxKind::from_keyword(word).is_none())
        .collect();
    assert!(missing.is_empty(), "not keywords here: {missing:?}");

    // and the lexer reserves nothing else, the contextual words aside
    let extra: Vec<&str> = sysml_syntax::KEYWORDS
        .iter()
        .map(|(word, _)| *word)
        .filter(|word| !union.contains(word) && !CONTEXTUAL.contains(word))
        .collect();
    assert!(
        extra.is_empty(),
        "keywords the BNF does not reserve: {extra:?}"
    );

    // the contextual list stays honest: not reserved, but real keywords
    for word in CONTEXTUAL {
        assert!(!union.contains(word), "`{word}` became reserved; drop it");
        assert!(
            sysml_syntax::SyntaxKind::from_keyword(word).is_some(),
            "`{word}` is no longer a keyword; drop it"
        );
    }
}

#[test]
fn contextual_keywords_still_work_as_names() {
    // reserving them would reject models the specification allows
    for word in CONTEXTUAL {
        let source = format!("part def {word};\npart x : {word};\n");
        let parse = sysml_syntax::parse(&source);
        assert!(parse.ok(), "`{word}` cannot be used as a name: {source}");
    }
}

//! The parser against input nobody would write on purpose.
//!
//! Recovery is the parser's job in an editor: every keystroke leaves the
//! file broken, and the tree still has to cover the text so that
//! highlighting, folding and the next edit's offsets stay put. The
//! corpus only ever presents finished files, so this presents each of
//! them cut short at thirty-two points, with a character taken out or
//! doubled at sixteen more, plus the shapes a fuzzer reaches first --
//! five hundred open braces, an unterminated note, a lone byte order
//! mark. About twenty thousand inputs in all. None may panic, and the
//! tree must still spell the input back exactly.

use std::panic::{catch_unwind, AssertUnwindSafe};

use sysml_corpus::{model_files, vendor};

/// Everything but the spacing: what the formatter may move around but
/// may not add to or take away from.
fn letters(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn why(said: Box<dyn std::any::Any + Send>) -> String {
    said.downcast_ref::<String>()
        .cloned()
        .or_else(|| said.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}

/// Parse `text`; the tree must cover it exactly, whatever the errors.
fn survives(
    what: &str,
    at: &str,
    text: &str,
    dialect: sysml_syntax::Dialect,
    bag: &mut Vec<String>,
) {
    match catch_unwind(AssertUnwindSafe(|| {
        let parse = sysml_syntax::parse_dialect(text, dialect);
        let back = parse.syntax().text().to_string();
        let formatted = sysml_syntax::fmt::format(text, dialect);
        (back, formatted)
    })) {
        Err(said) => bag.push(format!("PANIC-{what}\t{at}\t{}", why(said))),
        Ok((back, formatted)) => {
            if back != text {
                bag.push(format!("lossy-{what}\t{at}"));
            }
            // The formatter must not drop text it could not understand.
            // What it may do is re-space it, and where a quote or a
            // comment is left open that moves the boundary of the token
            // holding the rest of the file -- which is why `sysml fmt
            // --write` refuses a file that does not parse. Printing it
            // is still safe, and this is what safe means: every
            // character the modeller wrote is still in the output.
            if letters(text) != letters(&formatted) {
                bag.push(format!("fmt-lost-{what}\t{at}"));
            }
        }
    }
}

#[test]
fn broken_input_is_still_parsed() {
    let Some(root) = vendor() else { return };
    let mut found: Vec<String> = Vec::new();
    let mut all = model_files(&root.join("sysml/src"));
    all.extend(model_files(&root.join("kerml/src")));
    all.sort();
    eprintln!("{} files", all.len());

    for (n, path) in all.iter().enumerate() {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let dialect = sysml_syntax::Dialect::from_extension(
            path.extension().and_then(|e| e.to_str()).unwrap_or("sysml"),
        );
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        // Every prefix of the file, at 32 evenly spaced cuts: what a
        // language server sees while someone is still typing.
        for step in 0..32 {
            let want = text.len() * step / 32;
            let cut = (want..=text.len())
                .find(|&i| text.is_char_boundary(i))
                .unwrap_or(text.len());
            survives(
                "cut",
                &format!("{name}@{cut}"),
                &text[..cut],
                dialect,
                &mut found,
            );
        }

        // Deletions and doublings, spread over the file.
        let mut seed = (n as u64)
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        for _ in 0..16 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            if text.is_empty() {
                break;
            }
            let want = (seed >> 33) as usize % text.len();
            let from = (0..=want)
                .rev()
                .find(|&i| text.is_char_boundary(i))
                .unwrap();
            let to = (from + 1..=text.len())
                .find(|&i| text.is_char_boundary(i))
                .unwrap_or(text.len());
            let mut cut = text.to_string();
            let gone = cut.drain(from..to).collect::<String>();
            survives("bite", &format!("{name}-{from}"), &cut, dialect, &mut found);
            let mut twice = text.to_string();
            twice.insert_str(from, &gone);
            survives(
                "twice",
                &format!("{name}+{from}"),
                &twice,
                dialect,
                &mut found,
            );
        }
    }

    // Shapes a fuzzer finds before a corpus does.
    for (what, text) in [
        ("empty", ""),
        ("nul", "\0"),
        ("brace", "{"),
        ("braces", &"{".repeat(500)),
        ("parens", &"(".repeat(500)),
        ("brackets", &"[".repeat(500)),
        (
            "deep",
            &format!("part def A {{{}}}", "part b {".repeat(200)),
        ),
        ("expr", &format!("attribute x = {};", "(".repeat(300))),
        ("dots", &".".repeat(500)),
        ("quote", "part def '"),
        ("note", "//* unterminated"),
        ("bom", "\u{feff}part def A;"),
        ("cr", "part def A;\r\npart def B;\r"),
        ("wide", &format!("part def {};", "あ".repeat(100))),
        ("colons", &":".repeat(500)),
        ("arrows", &"->".repeat(500)),
    ] {
        for dialect in [sysml_syntax::Dialect::SysML, sysml_syntax::Dialect::KerML] {
            survives(
                "shape",
                &format!("{what}/{dialect:?}"),
                text,
                dialect,
                &mut found,
            );
        }
    }

    let mut kinds: std::collections::BTreeMap<&str, usize> = Default::default();
    for line in &found {
        *kinds.entry(line.split('\t').next().unwrap()).or_default() += 1;
    }
    eprintln!("--- {} findings", found.len());
    for (kind, n) in &kinds {
        eprintln!("{kind}: {n}");
    }
    for line in found.iter().take(30) {
        eprintln!("{line}");
    }
    assert!(found.is_empty());
}

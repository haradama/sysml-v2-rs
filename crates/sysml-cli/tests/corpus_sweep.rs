//! Everything the toolchain does, to every file in the corpus.
//!
//! Each crate has its own corpus test, and each asks about its own
//! concern. This asks the question none of them do -- does any of it
//! fall over -- by running the whole set over all 403 files: resolve,
//! export to JSON and read it back, draw the definition and browser
//! views and every definition's internals, and format. Nothing may
//! panic; the JSON must survive a round trip unchanged; the SVG must be
//! balanced; and formatting must be idempotent, must keep every token,
//! and must leave the library resolving to exactly what it did before.
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

fn vendor() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/sysml-v2-release")
        .canonicalize()
        .ok()?;
    root.join("sysml.library").is_dir().then_some(root)
}

fn files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![
        root.join("sysml/src"),
        root.join("kerml/src"),
        root.join("sysml.library"),
    ];
    while let Some(at) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&at) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("sysml" | "kerml")
            ) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

fn why(said: Box<dyn std::any::Any + Send>) -> String {
    said.downcast_ref::<String>()
        .cloned()
        .or_else(|| said.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}

#[test]
fn sweep() {
    let Some(root) = vendor() else { return };
    let mut found: Vec<String> = Vec::new();
    let all = files(&root);
    eprintln!("{} files", all.len());

    for path in &all {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let name = path.to_string_lossy().to_string();
        let at = || path.display().to_string();

        let built = catch_unwind(AssertUnwindSafe(|| {
            let mut ws = sysml_semantics::Workspace::new();
            let file = ws.add_file(name.clone(), &text);
            ws.resolve_files(&[file]);
            (ws, file)
        }));
        let Ok((ws, file)) = built else {
            found.push(format!(
                "PANIC-resolve\t{}\t{}",
                at(),
                why(built.err().unwrap())
            ));
            continue;
        };
        if !ws.file_parse(file).ok() {
            continue;
        }
        let roots = ws.file_roots(file).to_vec();

        match catch_unwind(AssertUnwindSafe(|| {
            let once = sysml_interchange::to_json(ws.model());
            let back = sysml_interchange::from_json(&once);
            (once, back)
        })) {
            Err(said) => found.push(format!("PANIC-interchange\t{}\t{}", at(), why(said))),
            Ok((once, Ok((model, _)))) => {
                if once != sysml_interchange::to_json(&model) {
                    found.push(format!("interchange-drift\t{}", at()));
                }
            }
            Ok((_, Err(e))) => found.push(format!("interchange-refused\t{}\t{e:?}", at())),
        }

        let style = sysml_diagram::Style::default();
        for (what, made) in [
            (
                "diagram",
                catch_unwind(AssertUnwindSafe(|| {
                    sysml_diagram::render(
                        &sysml_diagram::definition_diagram(ws.model(), &roots),
                        &style,
                    )
                })),
            ),
            (
                "browser",
                catch_unwind(AssertUnwindSafe(|| {
                    sysml_diagram::render_browser(
                        &sysml_diagram::browser_view(ws.model(), &roots),
                        &style,
                    )
                })),
            ),
        ] {
            match made {
                Err(said) => found.push(format!("PANIC-{what}\t{}\t{}", at(), why(said))),
                Ok(svg) => {
                    if let Err(e) = well_formed(&svg) {
                        found.push(format!("{what}-svg\t{}\t{e}", at()));
                    }
                }
            }
        }

        let inner = catch_unwind(AssertUnwindSafe(|| {
            let mut svgs = Vec::new();
            for elem in ws.model().descendants(ws.root()) {
                let d = sysml_diagram::interconnection_diagram(ws.model(), elem);
                if !d.nodes.is_empty() {
                    svgs.push(sysml_diagram::render(&d, &style));
                }
            }
            svgs
        }));
        match inner {
            Err(said) => found.push(format!("PANIC-internal\t{}\t{}", at(), why(said))),
            Ok(svgs) => {
                for svg in svgs {
                    if let Err(e) = well_formed(&svg) {
                        found.push(format!("internal-svg\t{}\t{e}", at()));
                        break;
                    }
                }
            }
        }

        let dialect = sysml_syntax::Dialect::from_extension(
            path.extension().and_then(|e| e.to_str()).unwrap_or("sysml"),
        );
        match catch_unwind(AssertUnwindSafe(|| {
            sysml_syntax::fmt::format(&text, dialect)
        })) {
            Err(said) => found.push(format!("PANIC-fmt\t{}\t{}", at(), why(said))),
            Ok(once) => {
                if once != sysml_syntax::fmt::format(&once, dialect) {
                    found.push(format!("fmt-unstable\t{}", at()));
                }
                let reparsed = sysml_syntax::parse_dialect(&once, dialect);
                if !reparsed.errors().is_empty() {
                    found.push(format!(
                        "fmt-broke-parse\t{}\t{:?}",
                        at(),
                        reparsed.errors().first()
                    ));
                } else if words(&text) != words(&once) {
                    found.push(format!("fmt-lost-tokens\t{}", at()));
                }
            }
        }
    }

    let whole = catch_unwind(AssertUnwindSafe(|| {
        let mut ws = sysml_semantics::Workspace::new();
        ws.load_dir(&root.join("sysml.library")).unwrap();
        let stats = ws.resolve_all();
        (ws, stats)
    }));
    match whole {
        Err(said) => found.push(format!("PANIC-library\t{}", why(said))),
        Ok((ws, stats)) => {
            eprintln!(
                "library: {} resolved, {} not",
                stats.resolved, stats.unresolved
            );
            let mut formatted = sysml_semantics::Workspace::new();
            for file in 0..ws.file_count() {
                let name = ws.file_name(file).to_string();
                let text = std::fs::read_to_string(&name).unwrap_or_default();
                let dialect = sysml_syntax::Dialect::from_extension(
                    Path::new(&name)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("sysml"),
                );
                formatted.add_file(name, &sysml_syntax::fmt::format(&text, dialect));
            }
            let after = formatted.resolve_all();
            if after.resolved != stats.resolved || after.unresolved != stats.unresolved {
                found.push(format!(
                    "fmt-changed-meaning\tlibrary\t{} -> {} resolved, {} -> {} not",
                    stats.resolved, after.resolved, stats.unresolved, after.unresolved
                ));
            }
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

/// The non-trivia tokens, as text. Formatting may move them; it may not
/// add, drop or change one.
fn words(text: &str) -> Vec<String> {
    use sysml_syntax::SyntaxKind;
    sysml_syntax::lex(text)
        .0
        .into_iter()
        .filter(|t| {
            !matches!(
                t.kind,
                SyntaxKind::WHITESPACE
                    | SyntaxKind::LINE_NOTE
                    | SyntaxKind::BLOCK_NOTE
                    | SyntaxKind::ERROR
            )
        })
        .map(|t| text[t.range.clone()].to_string())
        .collect()
}

/// Tags balance and nothing is left open.
fn well_formed(svg: &str) -> Result<(), String> {
    let mut stack: Vec<String> = Vec::new();
    let mut at = 0;
    while let Some(open) = svg[at..].find('<') {
        let start = at + open;
        let Some(close) = svg[start..].find('>') else {
            return Err("unclosed tag".into());
        };
        let end = start + close;
        let inner = &svg[start + 1..end];
        at = end + 1;
        if inner.starts_with('?') || inner.starts_with('!') {
            continue;
        }
        if let Some(rest) = inner.strip_prefix('/') {
            let tag = rest.trim();
            match stack.pop() {
                Some(open) if open == tag => {}
                other => return Err(format!("</{tag}> closes {other:?}")),
            }
        } else if !inner.ends_with('/') {
            stack.push(
                inner
                    .split([' ', '\n', '\t'])
                    .next()
                    .unwrap_or_default()
                    .to_string(),
            );
        }
    }
    if stack.is_empty() {
        Ok(())
    } else {
        Err(format!("left open: {stack:?}"))
    }
}

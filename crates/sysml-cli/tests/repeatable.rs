//! The same question, asked twice, must get the same answer.
//!
//! Two ways it might not. A generator that walks a hash map writes its
//! definitions in whatever order that map happened to have, which is a
//! different order in every process -- and a checked-in generated file
//! would then differ from the one the last run wrote. And a resolver
//! that remembers what it has worked out can remember something it
//! should not, so what resolves comes to depend on which file was asked
//! about first. Neither shows up in a test that runs once.

use std::path::{Path, PathBuf};

fn vendor() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/sysml-v2-release")
        .canonicalize()
        .ok()?;
    root.join("sysml.library").is_dir().then_some(root)
}

fn models(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.join("sysml/src"), root.join("kerml/src")];
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

fn workspace(name: &str, text: &str) -> (sysml_semantics::Workspace, usize) {
    let mut ws = sysml_semantics::Workspace::new();
    let file = ws.add_file(name, text);
    ws.resolve_files(&[file]);
    (ws, file)
}

/// Everything a run writes, from one workspace.
fn written(ws: &sysml_semantics::Workspace, file: usize) -> (String, String, String, String) {
    let roots = ws.file_roots(file).to_vec();
    let style = sysml_diagram::Style::default();
    let rust: String = sysml_rust::generate(ws.model(), &roots)
        .unwrap_or_default()
        .lines()
        .map(|line| match line.split_once("todo!(\"{}\", ") {
            Some((before, _)) => format!("{before}todo!(<as written>)\n"),
            None => format!("{line}\n"),
        })
        .collect();
    let svg = sysml_diagram::render(
        &sysml_diagram::definition_diagram(ws.model(), &roots),
        &style,
    );
    let browser =
        sysml_diagram::render_browser(&sysml_diagram::browser_view(ws.model(), &roots), &style);
    let json = sysml_interchange::to_json(ws.model()).to_string();
    (rust, svg, browser, json)
}

#[test]
fn nothing_written_depends_on_the_run() {
    let Some(root) = vendor() else { return };
    let mut found = Vec::new();
    for path in models(&root) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let name = path.to_string_lossy().to_string();
        let (a, file_a) = workspace(&name, &text);
        if !a.file_parse(file_a).ok() {
            continue;
        }
        let (b, file_b) = workspace(&name, &text);
        let first = written(&a, file_a);
        let second = written(&b, file_b);
        for (what, x, y) in [
            ("rust", &first.0, &second.0),
            ("svg", &first.1, &second.1),
            ("browser", &first.2, &second.2),
            ("json", &first.3, &second.3),
        ] {
            if x != y {
                found.push(format!("{what}\t{}", path.display()));
            }
        }
    }
    eprintln!("--- {} findings", found.len());
    for line in found.iter().take(20) {
        eprintln!("{line}");
    }
    assert!(found.is_empty());
}

#[test]
fn what_resolves_does_not_depend_on_the_order_it_was_asked_in() {
    let Some(root) = vendor() else { return };
    let all = models(&root);
    let mut found = Vec::new();
    for pair in all.chunks(2) {
        let [one, two] = pair else { continue };
        let (Ok(first), Ok(second)) = (std::fs::read_to_string(one), std::fs::read_to_string(two))
        else {
            continue;
        };

        // both files at once, in each order
        let mut forward = sysml_semantics::Workspace::new();
        let a = forward.add_file(one.to_string_lossy(), &first);
        let b = forward.add_file(two.to_string_lossy(), &second);
        let ahead = forward.resolve_files(&[a, b]);

        let mut backward = sysml_semantics::Workspace::new();
        let b2 = backward.add_file(two.to_string_lossy(), &second);
        let a2 = backward.add_file(one.to_string_lossy(), &first);
        let behind = backward.resolve_files(&[b2, a2]);

        if (ahead.resolved, ahead.unresolved) != (behind.resolved, behind.unresolved) {
            found.push(format!(
                "order\t{}+{}\t{}/{} vs {}/{}",
                one.display(),
                two.display(),
                ahead.resolved,
                ahead.unresolved,
                behind.resolved,
                behind.unresolved
            ));
        }

        // and one file at a time must agree with both at once
        let mut apart = sysml_semantics::Workspace::new();
        let c = apart.add_file(one.to_string_lossy(), &first);
        let d = apart.add_file(two.to_string_lossy(), &second);
        let left = apart.resolve_files(&[c]);
        let right = apart.resolve_files(&[d]);
        if (
            left.resolved + right.resolved,
            left.unresolved + right.unresolved,
        ) != (ahead.resolved, ahead.unresolved)
        {
            found.push(format!(
                "piecemeal\t{}+{}\t{}/{} vs {}/{}",
                one.display(),
                two.display(),
                left.resolved + right.resolved,
                left.unresolved + right.unresolved,
                ahead.resolved,
                ahead.unresolved
            ));
        }
    }
    eprintln!("--- {} findings", found.len());
    for line in found.iter().take(20) {
        eprintln!("{line}");
    }
    assert!(found.is_empty());
}

/// Formatting moves whitespace about, and nothing generated may move
/// with it -- except in one place, which both the model and the
/// generated code have on purpose: the wording of an expression neither
/// of them could translate, kept as the author wrote it for whoever
/// reads it next. That is the text the formatter just re-spaced, so it
/// is taken out of both sides before they are compared.
#[test]
fn formatting_a_model_does_not_change_what_is_generated_from_it() {
    let Some(root) = vendor() else { return };
    let mut found = Vec::new();
    for path in models(&root) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let dialect = sysml_syntax::Dialect::from_extension(
            path.extension().and_then(|e| e.to_str()).unwrap_or("sysml"),
        );
        let formatted = sysml_syntax::fmt::format(&text, dialect);
        let name = path.to_string_lossy().to_string();
        let (Some(before), Some(after)) = (generated(&name, &text), generated(&name, &formatted))
        else {
            continue;
        };
        if before.0 != after.0 {
            found.push(format!("rust\t{}", path.display()));
        }
        if before.1 != after.1 {
            found.push(format!("model\t{}", path.display()));
        }
    }
    assert!(
        found.is_empty(),
        "formatting showed through:\n{}",
        found.join("\n")
    );
}

/// The generated Rust, and the exported model with the author's own
/// wording of each expression taken out of it.
fn generated(name: &str, text: &str) -> Option<(String, String)> {
    let mut ws = sysml_semantics::Workspace::new();
    let file = ws.add_file(name, text);
    if !ws.file_parse(file).ok() {
        return None;
    }
    ws.resolve_files(&[file]);
    let roots = ws.file_roots(file).to_vec();
    let rust: String = sysml_rust::generate(ws.model(), &roots)
        .unwrap_or_default()
        .lines()
        .map(|line| match line.split_once("todo!(\"{}\", ") {
            Some((before, _)) => format!("{before}todo!(<as written>)\n"),
            None => format!("{line}\n"),
        })
        .collect();
    let mut json = sysml_interchange::to_json(ws.model());
    for element in json.as_array_mut().into_iter().flatten() {
        if element.get("@type").and_then(|t| t.as_str()) == Some("TextualRepresentation") {
            element["body"] = serde_json::Value::Null;
        }
    }
    Some((rust, json.to_string()))
}

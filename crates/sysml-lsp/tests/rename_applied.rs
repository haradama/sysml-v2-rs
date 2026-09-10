//! Renaming, applied.
//!
//! Rename is the one request that rewrites the modeller's file, and the
//! only way to know it is right is to apply what it offers and look at
//! what the model says afterwards. It must say the same thing: the same
//! references resolving to the same number of things. Every name the
//! first few symbols of each of the 309 example models declare is
//! renamed here, about twelve hundred in all -- which is how it came out
//! that a name mentioned in the middle of a qualified one, in an import,
//! or through a redefinition that borrowed it was left behind.

mod common;

use common::serving;
use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use serde_json::{json, Value};
use sysml_corpus::{models, vendor};

/// Apply LSP text edits to `text`, last first so the offsets hold.
fn apply(text: &str, edits: &[Value]) -> Option<String> {
    let index: Vec<usize> = std::iter::once(0)
        .chain(
            text.char_indices()
                .filter_map(|(at, ch)| (ch == '\n').then_some(at + 1)),
        )
        .collect();
    let offset = |position: &Value| -> Option<usize> {
        let line = position["line"].as_u64()? as usize;
        let character = position["character"].as_u64()? as usize;
        let start = *index.get(line)?;
        let mut units = 0usize;
        for (at, ch) in text[start..].char_indices() {
            if units == character {
                return Some(start + at);
            }
            units += ch.len_utf16();
        }
        (units == character).then_some(text.len())
    };
    let mut spans: Vec<(usize, usize, String)> = Vec::new();
    for edit in edits {
        let from = offset(&edit["range"]["start"])?;
        let to = offset(&edit["range"]["end"])?;
        spans.push((from, to, edit["newText"].as_str()?.to_string()));
    }
    spans.sort_by_key(|(from, ..)| *from);
    if spans.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return None; // overlapping edits are a finding of their own
    }
    let mut out = text.to_string();
    for (from, to, new) in spans.into_iter().rev() {
        out.replace_range(from..to, &new);
    }
    Some(out)
}

/// What the file resolves to on its own.
fn resolution(name: &str, text: &str) -> (usize, usize, bool) {
    let mut ws = sysml_semantics::Workspace::new();
    let file = ws.add_file(name, text);
    let parses = ws.file_parse(file).ok();
    let stats = ws.resolve_files(&[file]);
    (stats.resolved, stats.unresolved, parses)
}

#[test]
fn a_rename_leaves_the_model_saying_the_same_thing() {
    let Some(root) = vendor() else { return };
    let (mut client, _handle) = serving();
    client
        .ask(
            lsp_types::request::Initialize::METHOD,
            json!({ "capabilities": {}, "initializationOptions": { "noLibrary": true } }),
        )
        .unwrap();
    client.notify(lsp_types::notification::Initialized::METHOD, json!({}));

    let mut found: Vec<String> = Vec::new();
    let mut renames = 0usize;
    for path in models(&root) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let uri = lsp_types::Url::from_file_path(&path).unwrap().to_string();
        client.notify(
            lsp_types::notification::DidOpenTextDocument::METHOD,
            json!({"textDocument":{"uri":&uri,"languageId":"sysml","version":1,"text":text}}),
        );
        client.diagnostics_for(&uri);
        let before = resolution(&path.to_string_lossy(), &text);

        let symbols = client
            .ask(
                lsp_types::request::DocumentSymbolRequest::METHOD,
                json!({"textDocument":{"uri":&uri}}),
            )
            .unwrap_or(Value::Null);

        let mut places = Vec::new();
        collect(&symbols, &mut places);
        for place in places.iter().take(4) {
            let answer = client.ask(
                lsp_types::request::Rename::METHOD,
                json!({"textDocument":{"uri":&uri},"position":place,"newName":"Renamed_x"}),
            );
            let Ok(workspace) = answer else { continue };
            let Some(edits) = workspace["changes"][&uri].as_array() else {
                continue;
            };
            if edits.is_empty() {
                continue;
            }
            renames += 1;
            let Some(after_text) = apply(&text, edits) else {
                found.push(format!("overlapping\t{}\t{place}", path.display()));
                continue;
            };
            let after = resolution(&path.to_string_lossy(), &after_text);
            if before.2 && !after.2 {
                found.push(format!("broke-parse\t{}\t{place}", path.display()));
            } else if (before.0, before.1) != (after.0, after.1) {
                found.push(format!(
                    "changed-meaning\t{}\t{place}\t{}/{} -> {}/{}",
                    path.display(),
                    before.0,
                    before.1,
                    after.0,
                    after.1
                ));
            }
        }
        client.notify(
            lsp_types::notification::DidCloseTextDocument::METHOD,
            json!({"textDocument":{"uri":&uri}}),
        );
    }

    let mut kinds: std::collections::BTreeMap<&str, usize> = Default::default();
    for line in &found {
        *kinds.entry(line.split('\t').next().unwrap()).or_default() += 1;
    }
    eprintln!("--- {renames} renames, {} findings {kinds:?}", found.len());
    for line in found.iter().take(20) {
        eprintln!("{line}");
    }
    assert!(found.is_empty());
}

/// The start position of every symbol the document declares.
fn collect(symbols: &Value, out: &mut Vec<Value>) {
    for symbol in symbols.as_array().into_iter().flatten() {
        if let Some(range) = symbol.get("selectionRange") {
            out.push(range["start"].clone());
        }
        if let Some(children) = symbol.get("children") {
            collect(children, out);
        }
    }
}

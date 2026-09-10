//! The language server asked about every kind of place in the corpus.
//!
//! An editor sends requests wherever the cursor happens to be: inside a
//! name, on a brace, past the end of a line, past the end of the file.
//! The end-to-end tests cover a handful of positions in a model written
//! for them; this covers roughly a quarter of a million requests across
//! all 309 example models, at columns chosen to land off the end as
//! often as on a name. It also applies a run of incremental edits and
//! then sends the same text whole, so that the two must agree -- an
//! off-by-one in UTF-16 offsets shows up as nothing else.

mod common;

use common::serving;
use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use serde_json::{json, Value};
use sysml_corpus::{models, vendor};

#[test]
fn every_position_in_the_corpus() {
    let Some(root) = vendor() else { return };
    let (mut client, handle) = serving();
    client
        .ask(
            lsp_types::request::Initialize::METHOD,
            json!({ "capabilities": {}, "initializationOptions": { "noLibrary": true } }),
        )
        .unwrap();
    client.notify(lsp_types::notification::Initialized::METHOD, json!({}));

    let mut found: Vec<String> = Vec::new();
    let all = models(&root);
    eprintln!("{} files", all.len());

    for (n, path) in all.iter().enumerate() {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let uri = lsp_types::Url::from_file_path(path).unwrap().to_string();
        client.notify(
            lsp_types::notification::DidOpenTextDocument::METHOD,
            json!({"textDocument":{"uri":&uri,"languageId":"sysml","version":1,"text":text}}),
        );
        client.diagnostics_for(&uri);

        // whole-document requests
        for (method, params) in [
            (
                lsp_types::request::DocumentSymbolRequest::METHOD,
                json!({"textDocument":{"uri":&uri}}),
            ),
            (
                lsp_types::request::Formatting::METHOD,
                json!({"textDocument":{"uri":&uri},"options":{"tabSize":4,"insertSpaces":true}}),
            ),
        ] {
            if let Err(e) = client.ask(method, params) {
                found.push(format!("request\t{}\t{e}", path.display()));
            }
        }

        // every position on a spread of lines
        let lines: Vec<&str> = text.lines().collect();
        let step = (lines.len() / 24).max(1);
        for (row, line) in lines.iter().enumerate().step_by(step) {
            // columns: start, a few inside, one past the end, and far past
            let wide = line.chars().count();
            let cols: Vec<usize> = [0, wide / 3, wide / 2, wide, wide + 1, wide + 40]
                .into_iter()
                .collect();
            for col in cols {
                let at =
                    json!({"textDocument":{"uri":&uri},"position":{"line":row,"character":col}});
                for method in [
                    lsp_types::request::HoverRequest::METHOD,
                    lsp_types::request::GotoDefinition::METHOD,
                    lsp_types::request::SignatureHelpRequest::METHOD,
                ] {
                    if let Err(e) = client.ask(method, at.clone()) {
                        found.push(format!("{method}\t{}:{row}:{col}\t{e}", path.display()));
                    }
                }
                if let Err(e) = client.ask(
                    lsp_types::request::Completion::METHOD,
                    json!({"textDocument":{"uri":&uri},"position":{"line":row,"character":col},
                           "context":{"triggerKind":1}}),
                ) {
                    found.push(format!("completion\t{}:{row}:{col}\t{e}", path.display()));
                }
                if let Err(e) = client.ask(
                    lsp_types::request::References::METHOD,
                    json!({"textDocument":{"uri":&uri},"position":{"line":row,"character":col},
                           "context":{"includeDeclaration":true}}),
                ) {
                    found.push(format!("references\t{}:{row}:{col}\t{e}", path.display()));
                }
                if let Err(e) = client.ask(
                    lsp_types::request::Rename::METHOD,
                    json!({"textDocument":{"uri":&uri},"position":{"line":row,"character":col},
                           "newName":"Renamed"}),
                ) {
                    // A rename it declines -- off a name, of an alias
                    // whose uses it could not follow, of a feature that
                    // declares no name and borrows none either, or of
                    // something the standard library declares -- is the
                    // answer, not a fault.
                    //
                    // That last one only began to happen when the server
                    // gained a copy of the library to resolve against:
                    // before, a cursor on `Anything` landed on nothing,
                    // and the refusal that says an editor must not write
                    // into the library was never reached.
                    if !e.contains("nothing to rename here")
                        && !e.contains("alias")
                        && !e.contains("declares no name")
                        && !e.contains("declared outside the project")
                    {
                        found.push(format!("rename\t{}:{row}:{col}\t{e}", path.display()));
                    }
                }
            }
        }

        // positions beyond the end of the document
        for (row, col) in [(lines.len(), 0), (lines.len() + 100, 5), (99_999, 99_999)] {
            let at = json!({"textDocument":{"uri":&uri},"position":{"line":row,"character":col}});
            if let Err(e) = client.ask(lsp_types::request::HoverRequest::METHOD, at) {
                found.push(format!("past-end\t{}\t{e}", path.display()));
            }
        }

        // incremental edits must land where a full replace would
        if n % 8 == 0 && !text.is_empty() {
            let mut mine = text.clone();
            let mut seed = (n as u64).wrapping_mul(6364136223846793005).wrapping_add(1);
            for round in 0..8 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let want = (seed >> 33) as usize % mine.len();
                let from = (0..=want)
                    .rev()
                    .find(|&i| mine.is_char_boundary(i))
                    .unwrap();
                let insert = ["x", "\n", "とりで", "}}", "// ノート\n"][round % 5];
                let before = utf16_position(&mine, from);
                mine.insert_str(from, insert);
                client.notify(
                    lsp_types::notification::DidChangeTextDocument::METHOD,
                    json!({"textDocument":{"uri":&uri,"version":2+round},
                           "contentChanges":[{"range":{"start":before,"end":before},"text":insert}]}),
                );
                client.diagnostics_for(&uri);
            }
            let incremental = client
                .ask(
                    lsp_types::request::DocumentSymbolRequest::METHOD,
                    json!({"textDocument":{"uri":&uri}}),
                )
                .unwrap_or(Value::Null);
            // the same text, sent whole
            client.notify(
                lsp_types::notification::DidChangeTextDocument::METHOD,
                json!({"textDocument":{"uri":&uri,"version":99},
                       "contentChanges":[{"text":mine}]}),
            );
            client.diagnostics_for(&uri);
            let whole = client
                .ask(
                    lsp_types::request::DocumentSymbolRequest::METHOD,
                    json!({"textDocument":{"uri":&uri}}),
                )
                .unwrap_or(Value::Null);
            if incremental != whole {
                found.push(format!("incremental-drift\t{}", path.display()));
            }
        }

        client.notify(
            lsp_types::notification::DidCloseTextDocument::METHOD,
            json!({"textDocument":{"uri":&uri}}),
        );
    }

    for query in ["", "Part", "zzzz", "'", "::"] {
        if let Err(e) = client.ask(
            lsp_types::request::WorkspaceSymbolRequest::METHOD,
            json!({ "query": query }),
        ) {
            found.push(format!("workspace-symbol\t{query}\t{e}"));
        }
    }

    client
        .ask(lsp_types::request::Shutdown::METHOD, json!(null))
        .ok();
    client.notify(lsp_types::notification::Exit::METHOD, json!(null));
    let _ = handle.join();

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

/// The LSP position of a byte offset, counting UTF-16 code units.
fn utf16_position(text: &str, offset: usize) -> Value {
    let before = &text[..offset];
    let line = before.matches('\n').count();
    let start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let character: usize = text[start..offset].chars().map(char::len_utf16).sum();
    json!({ "line": line, "character": character })
}

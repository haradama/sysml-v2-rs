//! What the editor is not showing: a project file that changed on disk
//! since the last analysis, and a declaration that is not where the name
//! was clicked.
//!
//! Both are places where the server answered about one text while
//! measuring against another.

mod common;

use common::{serving, Client};
use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use serde_json::{json, Value};

/// A server serving `root`, with `open` opened as `sysml`.
fn over(
    root: &std::path::Path,
    open: &std::path::Path,
) -> (Client, std::thread::JoinHandle<()>, lsp_types::Url) {
    let (mut client, handle) = serving();
    client.request(
        lsp_types::request::Initialize::METHOD,
        json!({
            "capabilities": {},
            "initializationOptions": { "noLibrary": true },
            "workspaceFolders": [
                { "uri": lsp_types::Url::from_file_path(root).unwrap(), "name": "project" }
            ],
        }),
    );
    client.notify(lsp_types::notification::Initialized::METHOD, json!({}));
    let uri = lsp_types::Url::from_file_path(open).unwrap();
    let text = std::fs::read_to_string(open).unwrap();
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": {
            "uri": uri, "languageId": "sysml", "version": 1, "text": text
        }}),
    );
    (client, handle, uri)
}

#[test]
fn a_project_file_rewritten_on_disk_does_not_take_the_server_down() {
    let root = std::env::temp_dir().join("sysml-lsp-stale-test");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let root = std::fs::canonicalize(&root).unwrap();
    let declaring = root.join("a.sysml");
    let using = root.join("b.sysml");
    std::fs::write(&declaring, "package A {\n    part def Thing;\n}\n").unwrap();
    std::fs::write(&using, "package B {\n    part t : A::Thing;\n}\n").unwrap();

    let (mut client, handle, uri) = over(&root, &using);
    let where_ = |answer: &Value| {
        (answer["range"]["start"]["line"].as_u64().unwrap(), {
            answer["range"]["start"]["character"].as_u64().unwrap()
        })
    };
    let asked = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 17 },
    });
    let first = client.request(lsp_types::request::GotoDefinition::METHOD, asked.clone());
    assert_eq!(where_(&first), (1, 13));

    // The file the answer points into is rewritten behind the editor's
    // back, and every byte after the first line moves. Read afresh, the
    // offset the analysis holds lands inside a character of the new
    // text -- which is where slicing it killed the server.
    std::fs::write(
        &declaring,
        "package A { /* \u{3042}\u{3042}\u{3042}\u{3042}\u{3042} */ part def Thing; }\n",
    )
    .unwrap();
    let again = client.request(lsp_types::request::GotoDefinition::METHOD, asked);
    // and it still answers about the text it analysed, not the new one
    assert_eq!(where_(&again), (1, 13));
    client.stop(handle);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn renaming_a_borrowed_name_rewrites_the_declaration_it_came_from() {
    let root = std::env::temp_dir().join("sysml-lsp-borrowed-name-test");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let root = std::fs::canonicalize(&root).unwrap();
    let path = root.join("m.sysml");
    let source = "package P {\n\
                  \x20   part def Comp;\n\
                  \x20   part def Logical { part component : Comp; }\n\
                  \x20   part l : Logical { part :>> component; }\n\
                  }\n";
    std::fs::write(&path, source).unwrap();

    let (mut client, handle, uri) = over(&root, &path);
    // asked of the `component` in `:>> component`, which declares no
    // name of its own: there is nothing there to rewrite
    let edit = client.request(
        lsp_types::request::Rename::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 3, "character": 32 },
            "newName": "unit",
        }),
    );
    let edits = edit["changes"][uri.as_str()].as_array().unwrap();
    let mut spans: Vec<(u64, u64, u64)> = edits
        .iter()
        .map(|edit| {
            (
                edit["range"]["start"]["line"].as_u64().unwrap(),
                edit["range"]["start"]["character"].as_u64().unwrap(),
                edit["range"]["end"]["character"].as_u64().unwrap(),
            )
        })
        .collect();
    spans.sort_unstable();
    // the declaration on line 2 and the mention on line 3, each the
    // width of the name and nothing more -- the whole `part :>>
    // component;` used to be replaced by `unit`
    assert_eq!(spans, [(2, 28, 37), (3, 32, 41)]);
    assert!(
        edits.iter().all(|edit| edit["newText"] == "unit"),
        "{edits:?}"
    );
    client.stop(handle);
    let _ = std::fs::remove_dir_all(&root);
}

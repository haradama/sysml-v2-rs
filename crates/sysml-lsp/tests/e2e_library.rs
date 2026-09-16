//! Second end-to-end pass: a preloaded library, misses and error paths.

mod common;

use common::{serving, Client};
use lsp_server::{Connection, Message, RequestId, Response};
use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use serde_json::{json, Value};

#[test]
fn library_navigation_and_error_paths() {
    // a tiny standard library on disk
    let lib_dir = std::env::temp_dir().join("sysml-lsp-lib-test");
    let _ = std::fs::remove_dir_all(&lib_dir);
    std::fs::create_dir_all(&lib_dir).unwrap();
    std::fs::write(
        lib_dir.join("Base.sysml"),
        "package Base {\n    part def Anything { doc /* the base of everything */ }\n}\n",
    )
    .unwrap();

    let (mut client, handle) = serving();

    client.request(
        lsp_types::request::Initialize::METHOD,
        json!({
            "capabilities": {},
            "initializationOptions": { "libraryPath": lib_dir.to_str().unwrap() }
        }),
    );
    client.notify(lsp_types::notification::Initialized::METHOD, json!({}));

    let uri = "file:///app.sysml";
    let text = "package App {\n    import Base::*;\n    part thing : Anything;\n}\n";
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": { "uri": uri, "languageId": "sysml", "version": 1, "text": text } }),
    );
    let diags = client.wait_diagnostics();
    assert_eq!(diags["diagnostics"].as_array().unwrap().len(), 0);

    // definition of `Anything` jumps INTO the library file on disk
    let definition = client.request(
        lsp_types::request::GotoDefinition::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 2, "character": 20 }
        }),
    );
    let target = definition["uri"].as_str().unwrap();
    assert!(target.starts_with("file://"), "{target}");
    assert!(target.contains("Base.sysml"), "{target}");

    // hover shows the library documentation
    let hover = client.request(
        lsp_types::request::HoverRequest::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 2, "character": 20 }
        }),
    );
    let contents = hover["contents"]["value"].as_str().unwrap();
    assert!(contents.contains("Base::Anything"), "{contents}");

    // A library file opened for reading is the library's copy, not a
    // second one: the files beside it are never read into the project,
    // which holds for what the library already declares.
    let base = lsp_types::Url::from_file_path(lib_dir.join("Base.sysml")).unwrap();
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": { "uri": base, "languageId": "sysml", "version": 1,
                "text": "package Base {\n    part def Anything { doc /* the base of everything */ }\n}\n" } }),
    );
    client.wait_diagnostics();
    // a document from outside the project has the files beside it read
    // into it, which is what builds that layer again -- with the library
    // file above open, and standing where no directory does
    let ghost = "file:///sysml-lsp-no-such-directory/ghost.sysml";
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": { "uri": ghost, "languageId": "sysml", "version": 1,
                                  "text": "package Ghost {\n    part def Absent;\n}\n" } }),
    );
    client.wait_diagnostics();
    let definition = client.request(
        lsp_types::request::GotoDefinition::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 2, "character": 20 }
        }),
    );
    assert!(
        definition["uri"].as_str().unwrap().contains("Base.sysml"),
        "{definition}"
    );
    client.notify(
        lsp_types::notification::DidCloseTextDocument::METHOD,
        json!({ "textDocument": { "uri": ghost } }),
    );
    client.notify(
        lsp_types::notification::DidCloseTextDocument::METHOD,
        json!({ "textDocument": { "uri": base } }),
    );

    // find-references without the declaration
    let refs = client.request(
        lsp_types::request::References::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 2, "character": 20 },
            "context": { "includeDeclaration": false }
        }),
    );
    assert_eq!(refs.as_array().unwrap().len(), 1);

    // renaming a library element is refused, and says why
    let rename = client.send_request(
        lsp_types::request::Rename::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 2, "character": 20 },
            "newName": "Something"
        }),
    );
    assert!(rename
        .error
        .is_some_and(|e| e.message.contains("outside the project")));

    // requests that miss return null
    for method in [
        lsp_types::request::GotoDefinition::METHOD,
        lsp_types::request::HoverRequest::METHOD,
        lsp_types::request::SignatureHelpRequest::METHOD,
    ] {
        let miss = client.request(
            method,
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 0, "character": 0 }
            }),
        );
        assert!(miss.is_null(), "{method} should miss");
    }
    // rename with no target, and rename to something that is no name
    let miss = client.send_request(
        lsp_types::request::Rename::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": 0 },
            "newName": "x"
        }),
    );
    assert!(miss
        .error
        .is_some_and(|e| e.message.contains("nothing to rename")));
    for (spelling, reason) in [("part", "is a keyword"), ("2nd", "is not a name")] {
        let refused = client.send_request(
            lsp_types::request::Rename::METHOD,
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 2, "character": 20 },
                "newName": spelling
            }),
        );
        assert!(refused.error.is_some_and(|e| e.message.contains(reason)));
    }
    // workspace symbols: library hit and a miss
    let symbols = client.request(
        lsp_types::request::WorkspaceSymbolRequest::METHOD,
        json!({ "query": "anyth" }),
    );
    assert!(!symbols.as_array().unwrap().is_empty());
    let none = client.request(
        lsp_types::request::WorkspaceSymbolRequest::METHOD,
        json!({ "query": "zzzznothing" }),
    );
    assert!(none.as_array().unwrap().is_empty());

    // unknown method -> MethodNotFound; invalid params -> InvalidParams
    let resp = client.send_request("textDocument/unknownFeature", json!({}));
    assert!(resp.error.is_some());
    let resp = client.send_request(lsp_types::request::HoverRequest::METHOD, json!("garbage"));
    assert!(resp.error.is_some());

    // a stray response message is ignored by the server
    client.send_raw(Message::Response(Response::new_ok(
        RequestId::from(999),
        Value::Null,
    )));

    // formatting a messy document yields one full-document edit
    client.notify(
        lsp_types::notification::DidChangeTextDocument::METHOD,
        json!({
            "textDocument": { "uri": uri, "version": 2 },
            "contentChanges": [{ "text": "package   App {  }" }]
        }),
    );
    client.wait_diagnostics();
    let edits = client.request(
        lsp_types::request::Formatting::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }),
    );
    assert_eq!(edits.as_array().unwrap().len(), 1);

    // an out-of-bounds ranged change falls back to full replacement
    client.notify(
        lsp_types::notification::DidChangeTextDocument::METHOD,
        json!({
            "textDocument": { "uri": uri, "version": 3 },
            "contentChanges": [{
                "range": {
                    "start": { "line": 99, "character": 0 },
                    "end": { "line": 99, "character": 1 }
                },
                "text": "package Clean { }"
            }]
        }),
    );
    client.wait_diagnostics();
    let symbols = client.request(
        lsp_types::request::DocumentSymbolRequest::METHOD,
        json!({ "textDocument": { "uri": uri } }),
    );
    assert_eq!(symbols[0]["name"], "Clean");

    // closing the document stops diagnostics for it
    client.notify(
        lsp_types::notification::DidCloseTextDocument::METHOD,
        json!({ "textDocument": { "uri": uri } }),
    );
    // requests for a closed document miss gracefully
    let miss = client.request(
        lsp_types::request::DocumentSymbolRequest::METHOD,
        json!({ "textDocument": { "uri": uri } }),
    );
    assert!(miss.is_null());

    // a second document with parse errors publishes error diagnostics
    let bad_uri = "file:///broken.sysml";
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": { "uri": bad_uri, "languageId": "sysml", "version": 1,
                 "text": "part def {{{" } }),
    );
    let diags = client.wait_diagnostics();
    assert_eq!(diags["uri"], bad_uri);
    assert!(!diags["diagnostics"].as_array().unwrap().is_empty());

    // an inverted range falls back to replacing the whole document
    client.notify(
        lsp_types::notification::DidChangeTextDocument::METHOD,
        json!({
            "textDocument": { "uri": bad_uri, "version": 2 },
            "contentChanges": [{
                "range": {
                    "start": { "line": 0, "character": 5 },
                    "end": { "line": 0, "character": 1 }
                },
                "text": "package Fixed { }"
            }]
        }),
    );
    let diags = client.wait_diagnostics();
    assert_eq!(diags["uri"], bad_uri);
    assert_eq!(diags["diagnostics"].as_array().unwrap().len(), 0);

    // a change for a document that was never opened is ignored
    client.notify(
        lsp_types::notification::DidChangeTextDocument::METHOD,
        json!({
            "textDocument": { "uri": "file:///ghost.sysml", "version": 1 },
            "contentChanges": [{ "text": "package G { }" }]
        }),
    );
    // an unrelated notification is ignored
    client.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": {} }),
    );

    // invalid params produce error responses for every request type
    for method in [
        lsp_types::request::GotoDefinition::METHOD,
        lsp_types::request::DocumentSymbolRequest::METHOD,
        lsp_types::request::References::METHOD,
        lsp_types::request::Rename::METHOD,
        lsp_types::request::Completion::METHOD,
        lsp_types::request::WorkspaceSymbolRequest::METHOD,
        lsp_types::request::SignatureHelpRequest::METHOD,
        lsp_types::request::Formatting::METHOD,
    ] {
        let resp = client.send_request(method, json!("garbage"));
        assert!(resp.error.is_some(), "{method} accepted garbage");
    }

    client.request(lsp_types::request::Shutdown::METHOD, Value::Null);
    client.notify(lsp_types::notification::Exit::METHOD, Value::Null);
    handle.join().unwrap();
}

#[test]
fn client_disconnect_without_shutdown_terminates_the_server() {
    let (server_side, client_side) = Connection::memory();
    let handle = std::thread::spawn(move || sysml_lsp::run(&server_side));
    let mut client = Client::new(client_side);
    client.request(
        lsp_types::request::Initialize::METHOD,
        json!({ "capabilities": {}, "initializationOptions": { "noLibrary": true } }),
    );
    client.notify(lsp_types::notification::Initialized::METHOD, json!({}));
    drop(client);
    // the receive loop ends when the channel closes
    handle.join().unwrap().unwrap();
}

#[test]
fn exit_before_shutdown_leaves_with_a_failure() {
    // the spec asks for it, and a client reads the code to tell a stop
    // it asked for from a server that fell over
    let (server_side, client_side) = Connection::memory();
    let handle = std::thread::spawn(move || sysml_lsp::run(&server_side));
    let mut client = Client::new(client_side);
    client.request(
        lsp_types::request::Initialize::METHOD,
        json!({ "capabilities": {}, "initializationOptions": { "noLibrary": true } }),
    );
    client.notify(lsp_types::notification::Initialized::METHOD, json!({}));
    client.notify(lsp_types::notification::Exit::METHOD, Value::Null);
    assert!(handle.join().unwrap().is_err());
}

/// The protocol's own traffic may arrive at any point, including in the
/// one gap where the specification names the message that comes next.
#[test]
fn set_trace_before_initialized_does_not_stop_the_server() {
    let (server_side, client_side) = Connection::memory();
    let handle = std::thread::spawn(move || sysml_lsp::run(&server_side));
    let mut client = Client::new(client_side);
    client.request(
        lsp_types::request::Initialize::METHOD,
        json!({ "capabilities": {}, "initializationOptions": { "noLibrary": true } }),
    );
    // between the handshake and `initialized`, which is where a server
    // that took the specification's sequence too literally fell over
    client.notify("$/setTrace", json!({ "value": "verbose" }));
    client.notify(lsp_types::notification::Initialized::METHOD, json!({}));
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": {
            "uri": "file:///traced.sysml",
            "languageId": "sysml",
            "version": 1,
            "text": "part def Wheel;\n",
        }}),
    );
    // it is still serving, which is the whole of what this asks
    let diagnostics = client.wait_diagnostics();
    assert_eq!(diagnostics["uri"], "file:///traced.sysml");
    client.stop(std::thread::spawn(move || {
        handle.join().unwrap().unwrap();
    }));
}

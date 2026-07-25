//! Second end-to-end pass: a preloaded library, misses and error paths.

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use serde_json::{json, Value};

struct Client {
    connection: Connection,
    next_id: i32,
}

impl Client {
    fn send_request(&mut self, method: &str, params: Value) -> Response {
        let id = RequestId::from(self.next_id);
        self.next_id += 1;
        self.connection
            .sender
            .send(Message::Request(Request {
                id: id.clone(),
                method: method.into(),
                params,
            }))
            .unwrap();
        loop {
            match self.recv() {
                Message::Response(resp) if resp.id == id => return resp,
                _ => continue,
            }
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let resp = self.send_request(method, params);
        assert!(resp.error.is_none(), "error response: {:?}", resp.error);
        resp.result.unwrap_or(Value::Null)
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.connection
            .sender
            .send(Message::Notification(Notification {
                method: method.into(),
                params,
            }))
            .unwrap();
    }

    fn recv(&mut self) -> Message {
        self.connection
            .receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("server did not answer")
    }

    fn wait_diagnostics(&mut self) -> Value {
        loop {
            match self.recv() {
                Message::Notification(n)
                    if n.method == lsp_types::notification::PublishDiagnostics::METHOD =>
                {
                    return n.params;
                }
                _ => continue,
            }
        }
    }
}

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

    let (server_side, client_side) = Connection::memory();
    let handle = std::thread::spawn(move || sysml_lsp::run(&server_side).unwrap());
    let mut client = Client {
        connection: client_side,
        next_id: 1,
    };

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

    // renaming a library element is refused
    let rename = client.request(
        lsp_types::request::Rename::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 2, "character": 20 },
            "newName": "Something"
        }),
    );
    assert!(rename.is_null());

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
    // rename with no target
    let miss = client.request(
        lsp_types::request::Rename::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": 0 },
            "newName": "x"
        }),
    );
    assert!(miss.is_null());
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
    client
        .connection
        .sender
        .send(Message::Response(Response::new_ok(
            RequestId::from(999),
            Value::Null,
        )))
        .unwrap();

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
    let mut client = Client {
        connection: client_side,
        next_id: 1,
    };
    client.request(
        lsp_types::request::Initialize::METHOD,
        json!({ "capabilities": {} }),
    );
    client.notify(lsp_types::notification::Initialized::METHOD, json!({}));
    drop(client);
    // the receive loop ends when the channel closes
    handle.join().unwrap().unwrap();
}

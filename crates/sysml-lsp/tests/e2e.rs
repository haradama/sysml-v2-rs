//! End-to-end test driving the language server over an in-memory
//! connection: initialize, open a document, receive diagnostics, jump to a
//! definition, hover, and format.

use lsp_server::{Connection, Message, Notification, Request, RequestId};
use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use serde_json::{json, Value};

struct Client {
    connection: Connection,
    next_id: i32,
}

impl Client {
    fn request(&mut self, method: &str, params: Value) -> Value {
        let resp = self.send_request(method, params);
        assert!(resp.error.is_none(), "error response: {:?}", resp.error);
        resp.result.unwrap_or(Value::Null)
    }

    fn send_request(&mut self, method: &str, params: Value) -> lsp_server::Response {
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
fn serves_diagnostics_definition_hover_and_formatting() {
    let (server_side, client_side) = Connection::memory();
    let handle = std::thread::spawn(move || sysml_lsp::run(&server_side).unwrap());
    let mut client = Client {
        connection: client_side,
        next_id: 1,
    };

    // handshake
    client.request(
        lsp_types::request::Initialize::METHOD,
        json!({ "capabilities": {} }),
    );
    client.notify(lsp_types::notification::Initialized::METHOD, json!({}));

    // open a document: one good reference, one unresolved, one parse error
    let uri = "file:///demo.sysml";
    let text = "package Demo {\n    doc D /* about demo */\n    part def Vehicle;\n    part car : Vehicle;\n    part bad : NoSuchThing;\n    calc def Sum {\n        in a : Real;\n        in b : Real;\n    }\n    attribute s = Sum(1, 2);\n}\n";
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": { "uri": uri, "languageId": "sysml", "version": 1, "text": text } }),
    );
    let diags = client.wait_diagnostics();
    let list = diags["diagnostics"].as_array().unwrap();
    // NoSuchThing and the (library-less) Real are unresolved
    assert_eq!(list.len(), 3, "{list:?}");
    assert!(list
        .iter()
        .any(|d| d["message"].as_str().unwrap().contains("NoSuchThing")));

    // go to definition of `Vehicle` in `part car : Vehicle;` (line 2)
    let definition = client.request(
        lsp_types::request::GotoDefinition::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 3, "character": 16 }
        }),
    );
    assert_eq!(definition["uri"], uri);
    assert_eq!(definition["range"]["start"]["line"], 2);

    // hover shows the metaclass and qualified name
    let hover = client.request(
        lsp_types::request::HoverRequest::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 3, "character": 16 }
        }),
    );
    let contents = hover["contents"]["value"].as_str().unwrap();
    assert!(contents.contains("PartDefinition"), "{contents}");
    assert!(contents.contains("Demo::Vehicle"), "{contents}");

    // document symbols expose the tree
    let symbols = client.request(
        lsp_types::request::DocumentSymbolRequest::METHOD,
        json!({ "textDocument": { "uri": uri } }),
    );
    assert_eq!(symbols[0]["name"], "Demo");
    assert_eq!(symbols[0]["children"].as_array().unwrap().len(), 6);

    // formatting of an already-formatted file is a no-op
    let edits = client.request(
        lsp_types::request::Formatting::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }),
    );
    assert_eq!(edits.as_array().map(Vec::len), Some(0));

    // find references to Vehicle (from its declaration, incl. declaration)
    let refs = client.request(
        lsp_types::request::References::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 2, "character": 14 },
            "context": { "includeDeclaration": true }
        }),
    );
    let refs = refs.as_array().unwrap();
    assert_eq!(refs.len(), 2, "{refs:?}"); // declaration + `car : Vehicle`

    // rename Vehicle -> Car (declaration + reference edited)
    let edit = client.request(
        lsp_types::request::Rename::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 3, "character": 16 },
            "newName": "Car"
        }),
    );
    let edits = edit["changes"][uri].as_array().unwrap();
    assert_eq!(edits.len(), 2, "{edits:?}");
    assert!(edits.iter().all(|e| e["newText"] == "Car"));

    // renaming onto a name already visible there would capture it
    let clash = client.send_request(
        lsp_types::request::Rename::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 3, "character": 16 },
            "newName": "Sum"
        }),
    );
    assert!(clash
        .error
        .is_some_and(|e| e.message.contains("already visible")));

    // completion inside the package body sees Vehicle and keywords
    let completions = client.request(
        lsp_types::request::Completion::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 4, "character": 15 }
        }),
    );
    let labels: Vec<&str> = completions
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["label"].as_str().unwrap())
        .collect();
    assert!(labels.contains(&"Vehicle"), "{labels:?}");
    assert!(labels.contains(&"car"), "{labels:?}");
    assert!(labels.contains(&"part"), "{labels:?}");
    assert!(labels.contains(&"D"), "{labels:?}");

    // signature help inside `Sum(1, 2)` — after the comma (line 5 col 22)
    let help = client.request(
        lsp_types::request::SignatureHelpRequest::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 9, "character": 24 }
        }),
    );
    assert_eq!(
        help["signatures"][0]["label"],
        "Sum(in a : Real, in b : Real)"
    );
    assert_eq!(help["activeParameter"], 1);

    // workspace symbol search finds Vehicle
    let symbols = client.request(
        lsp_types::request::WorkspaceSymbolRequest::METHOD,
        json!({ "query": "vehic" }),
    );
    let names: Vec<&str> = symbols
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"Vehicle"), "{names:?}");

    // incremental edit: fix `NoSuchThing` -> `Vehicle` via a ranged change
    client.notify(
        lsp_types::notification::DidChangeTextDocument::METHOD,
        json!({
            "textDocument": { "uri": uri, "version": 2 },
            "contentChanges": [{
                "range": {
                    "start": { "line": 4, "character": 15 },
                    "end": { "line": 4, "character": 26 }
                },
                "text": "Vehicle"
            }]
        }),
    );
    let diags = client.wait_diagnostics();
    let list = diags["diagnostics"].as_array().unwrap();
    assert_eq!(list.len(), 2, "{list:?}"); // only the two Real warnings remain

    // shutdown
    client.request(lsp_types::request::Shutdown::METHOD, Value::Null);
    client.notify(lsp_types::notification::Exit::METHOD, Value::Null);
    handle.join().unwrap();
}

#[test]
fn serves_diagrams_for_a_preview() {
    let (server_side, client_side) = Connection::memory();
    let handle = std::thread::spawn(move || sysml_lsp::run(&server_side).unwrap());
    let mut client = Client {
        connection: client_side,
        next_id: 1,
    };
    client.request(
        lsp_types::request::Initialize::METHOD,
        json!({ "capabilities": {} }),
    );
    client.notify(lsp_types::notification::Initialized::METHOD, json!({}));

    let uri = "file:///preview.sysml";
    let text = "part def PowerSource;\npart def Engine :> PowerSource {\n\tpart p : Piston;\n}\npart def Piston;\n";
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": { "uri": uri, "languageId": "sysml", "version": 1, "text": text } }),
    );
    client.wait_diagnostics();

    // the default view draws the definitions and their relationships
    let result = client.request("sysml/diagram", json!({ "uri": uri }));
    let svg = result["svg"].as_str().unwrap();
    assert!(svg.starts_with("<svg xmlns="));
    assert!(svg.contains("PowerSource"));
    assert!(svg.contains("marker-end=\"url(#specialization)\""));

    // the internal view draws one element's structure
    let result = client.request(
        "sysml/diagram",
        json!({ "uri": uri, "view": "internal", "element": "Engine" }),
    );
    assert!(result["svg"].as_str().unwrap().contains("p : Piston"));

    // the browser view draws the membership tree
    let result = client.request("sysml/diagram", json!({ "uri": uri, "view": "browser" }));
    assert!(result["svg"].as_str().unwrap().contains("Engine"));

    // a document with no definitions falls back to the tree
    let empty = "package P {\n\tpart car;\n}\n";
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": { "uri": "file:///plain.sysml", "languageId": "sysml", "version": 1, "text": empty } }),
    );
    client.wait_diagnostics();
    let result = client.request("sysml/diagram", json!({ "uri": "file:///plain.sysml" }));
    assert!(result["svg"].as_str().unwrap().contains("car"));

    // an unknown document is null, not an error
    let result = client.request("sysml/diagram", json!({ "uri": "file:///nowhere.sysml" }));
    assert!(result.is_null());

    // malformed parameters are a proper error response, not a crash
    let id = client.next_id;
    client.next_id += 1;
    client
        .connection
        .sender
        .send(Message::Request(lsp_server::Request {
            id: id.into(),
            method: "sysml/diagram".into(),
            params: json!({ "uri": 42 }),
        }))
        .unwrap();
    loop {
        if let Message::Response(response) = client.recv() {
            assert!(response.error.is_some());
            break;
        }
    }

    client.request(lsp_types::request::Shutdown::METHOD, Value::Null);
    client.notify(lsp_types::notification::Exit::METHOD, Value::Null);
    handle.join().unwrap();
}

/// `layout: "graphviz"` runs the configured dot command for positions;
/// a missing command falls back to the built-in layout instead of
/// leaving the preview empty.
#[test]
fn lays_diagrams_out_with_graphviz_when_asked() {
    use std::os::unix::fs::PermissionsExt;
    let fake = std::env::temp_dir().join(format!("sysml-e2e-fake-dot-{}", std::process::id()));
    std::fs::write(
        &fake,
        "#!/bin/sh\ncat >/dev/null\n\
         printf 'graph 1 6 2\\n'\n\
         printf 'node n0 1.5 1 1 1\\n'\n\
         printf 'node n1 4 1 1 1\\n'\n\
         printf 'stop\\n'\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

    for (command, expected_height) in [
        // the fake lays a 6in x 2in canvas: 2in * 72dpi + two 16px margins
        (fake.to_str().unwrap(), Some("height=\"176\"")),
        // no such command: the built-in layout draws instead
        ("/nonexistent/graphviz/dot", None),
    ] {
        let (server_side, client_side) = Connection::memory();
        let handle = std::thread::spawn(move || sysml_lsp::run(&server_side).unwrap());
        let mut client = Client {
            connection: client_side,
            next_id: 1,
        };
        client.request(
            lsp_types::request::Initialize::METHOD,
            json!({ "capabilities": {}, "initializationOptions": { "dotCommand": command } }),
        );
        client.notify(lsp_types::notification::Initialized::METHOD, json!({}));

        let uri = "file:///layout.sysml";
        client.notify(
            lsp_types::notification::DidOpenTextDocument::METHOD,
            json!({ "textDocument": { "uri": uri, "languageId": "sysml", "version": 1,
                     "text": "part def A;\npart def B :> A;\n" } }),
        );
        client.wait_diagnostics();

        let result = client.request("sysml/diagram", json!({ "uri": uri, "layout": "graphviz" }));
        let svg = result["svg"].as_str().unwrap();
        assert!(svg.starts_with("<svg xmlns="));
        assert!(svg.contains(">A<") && svg.contains(">B<"));
        match expected_height {
            Some(height) => assert!(svg.contains(height), "{svg:.240}"),
            None => assert!(svg.contains("marker-end=\"url(#specialization)\"")),
        }

        client.request(lsp_types::request::Shutdown::METHOD, Value::Null);
        client.notify(lsp_types::notification::Exit::METHOD, Value::Null);
        handle.join().unwrap();
    }
    std::fs::remove_file(&fake).ok();
}

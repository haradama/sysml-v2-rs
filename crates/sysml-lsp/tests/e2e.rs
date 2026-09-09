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

/// `layout: "elk"` runs the configured ELK command for positions; a
/// missing command falls back to the built-in layout instead of
/// leaving the preview empty.
#[test]
fn lays_diagrams_out_with_elk_when_asked() {
    use std::os::unix::fs::PermissionsExt;
    let fake = std::env::temp_dir().join(format!("sysml-e2e-fake-elk-{}", std::process::id()));
    std::fs::write(
        &fake,
        "#!/bin/sh\ncat >/dev/null\n\
         printf '{\"width\":400,\"height\":144,\"children\":\
[{\"id\":\"n0\",\"x\":0,\"y\":0},{\"id\":\"n1\",\"x\":0,\"y\":100}]}\\n'\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

    for (command, expected_height) in [
        // the fake's canvas plus two 16px margins
        (fake.to_str().unwrap(), Some("height=\"176\"")),
        // no such command: the built-in layout draws instead
        ("/nonexistent/elk/elkrs", None),
    ] {
        let (server_side, client_side) = Connection::memory();
        let handle = std::thread::spawn(move || sysml_lsp::run(&server_side).unwrap());
        let mut client = Client {
            connection: client_side,
            next_id: 1,
        };
        client.request(
            lsp_types::request::Initialize::METHOD,
            json!({ "capabilities": {}, "initializationOptions": { "elkCommand": command } }),
        );
        client.notify(lsp_types::notification::Initialized::METHOD, json!({}));

        let uri = "file:///layout.sysml";
        client.notify(
            lsp_types::notification::DidOpenTextDocument::METHOD,
            json!({ "textDocument": { "uri": uri, "languageId": "sysml", "version": 1,
                     "text": "part def A;\npart def B :> A;\n" } }),
        );
        client.wait_diagnostics();

        let result = client.request("sysml/diagram", json!({ "uri": uri, "layout": "elk" }));
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

/// Open one document on a fresh server and hand back the client.
fn opened(uri: &str, text: &str) -> (Client, std::thread::JoinHandle<()>) {
    let (client, handle, _) = opened_as(uri, "sysml", text);
    (client, handle)
}

/// Open one document of the language its client declares, and hand back
/// the client with what the server first said about the document.
fn opened_as(
    uri: &str,
    language: &str,
    text: &str,
) -> (Client, std::thread::JoinHandle<()>, Value) {
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
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": {
            "uri": uri, "languageId": language, "version": 1, "text": text
        }}),
    );
    let diagnostics = client.wait_diagnostics();
    (client, handle, diagnostics)
}

fn shut(mut client: Client, handle: std::thread::JoinHandle<()>) {
    client.request(lsp_types::request::Shutdown::METHOD, Value::Null);
    client.notify(lsp_types::notification::Exit::METHOD, Value::Null);
    handle.join().unwrap();
}

#[test]
fn a_new_name_is_whatever_this_file_reads_as_one_name() {
    let uri = "file:///names.sysml";
    let (mut client, handle) = opened(
        uri,
        "package Demo {\n    part def Vehicle;\n    part car : Vehicle;\n}\n",
    );
    let rename = |client: &mut Client, to: &str| {
        client.send_request(
            lsp_types::request::Rename::METHOD,
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 1, "character": 14 },
                "newName": to,
            }),
        )
    };

    // a quoted name is a name, and used to be refused
    let quoted = rename(&mut client, "'two words'");
    assert!(quoted.error.is_none(), "{:?}", quoted.error);
    let edits = quoted.result.unwrap()["changes"][uri]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(edits, 2);

    // this lexer's names are ASCII, so a rename to anything else would
    // be applied and leave a file that no longer reads
    let refused = rename(&mut client, "\u{540d}\u{524d}");
    assert!(
        refused
            .error
            .is_some_and(|err| err.message.contains("is not a name")),
        "a name outside the lexer's alphabet was accepted"
    );
    shut(client, handle);
}

#[test]
fn a_document_that_does_not_parse_is_left_alone() {
    // the `;` is missing, so the tree is a guess: formatting it would
    // set two declarations on one line as though that were meant
    let uri = "file:///broken.sysml";
    let (mut client, handle) = opened(uri, "package Demo {\n    part def A\n    part def B;\n}\n");
    let edits = client.request(
        lsp_types::request::Formatting::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }),
    );
    assert!(edits.is_null(), "{edits}");
    shut(client, handle);
}

#[test]
fn a_notification_this_server_cannot_read_is_dropped_rather_than_fatal() {
    // a client is not supposed to send these; one that does used to end
    // the session, leaving the file it was editing with no diagnostics,
    // no completion and no navigation until the window was reloaded
    let uri = "file:///nonsense.sysml";
    let (mut client, handle) = opened(uri, "package Demo {\n    part def Vehicle;\n}\n");
    for method in [
        lsp_types::notification::DidOpenTextDocument::METHOD,
        lsp_types::notification::DidChangeTextDocument::METHOD,
        lsp_types::notification::DidCloseTextDocument::METHOD,
        lsp_types::notification::DidChangeWatchedFiles::METHOD,
    ] {
        client.notify(method, json!({ "textDocument": { "uri": 42 } }));
    }
    // still answering, and still holding the document it was given
    let symbols = client.request(
        lsp_types::request::DocumentSymbolRequest::METHOD,
        json!({ "textDocument": { "uri": uri } }),
    );
    assert_eq!(symbols[0]["name"], "Demo");
    shut(client, handle);
}

#[test]
fn a_rename_that_would_capture_a_name_where_it_is_used_is_refused() {
    // Nothing named `B` is visible where `A` is declared, so the
    // declaration's own scope says the rename is safe. It is not: `R`
    // imports both packages, and `part a : A` there binds to `Q::B` the
    // moment it is spelled `B` -- a model that still parses, still
    // resolves, and means something else.
    let uri = "file:///capture.sysml";
    let (mut client, handle) = opened(
        uri,
        "package Top {\n    package Q {\n        part def B;\n    }\n    package P {\n        part def A;\n    }\n    package R {\n        import P::*;\n        import Q::*;\n        part a : A;\n        part b : P::A;\n    }\n}\n",
    );
    let refused = client.send_request(
        lsp_types::request::Rename::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 5, "character": 17 },
            "newName": "B",
        }),
    );
    assert!(
        refused
            .error
            .is_some_and(|err| err.message.contains("visible where this one is used")),
        "a rename that rebinds a reference to another element was offered"
    );

    // a name nothing else answers to anywhere is still offered
    let allowed = client.send_request(
        lsp_types::request::Rename::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 5, "character": 17 },
            "newName": "C",
        }),
    );
    assert!(allowed.error.is_none(), "{:?}", allowed.error);
    let edits = allowed.result.unwrap()["changes"][uri]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(edits, 3); // the declaration and both mentions
    shut(client, handle);
}

#[test]
fn a_mention_spelled_some_other_way_is_not_rewritten_as_this_name() {
    // `x : e` names the same definition by its short name, which
    // renaming the long one leaves standing. Rewriting it would put
    // `Motor` where the model still means `e`.
    let uri = "file:///short.sysml";
    let (mut client, handle) = opened(
        uri,
        "package P {\n    part def <e> Engine;\n    part x : e;\n}\n",
    );
    let edit = client.request(
        lsp_types::request::Rename::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 17 },
            "newName": "Motor",
        }),
    );
    let edits = edit["changes"][uri].as_array().unwrap();
    assert_eq!(edits.len(), 1, "{edits:?}"); // the declaration, alone
    shut(client, handle);
}

#[test]
fn a_rename_no_alias_can_follow_is_refused_rather_than_half_done() {
    // `y : Motor` reaches `Engine` through the alias, and nothing
    // records where the alias's own `for Engine` was written. Renaming
    // the declaration leaves that pointing at a name that no longer
    // exists, and rewriting the mention says `Turbine` where the model
    // still means `Motor`.
    let uri = "file:///alias.sysml";
    let (mut client, handle) = opened(
        uri,
        "package P {\n    part def Engine;\n    alias Motor for Engine;\n    part y : Motor;\n}\n",
    );
    let refused = client.send_request(
        lsp_types::request::Rename::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 13 },
            "newName": "Turbine",
        }),
    );
    assert!(
        refused
            .error
            .is_some_and(|err| err.message.contains("no rename can follow")),
        "a rename that leaves an alias pointing at nothing was offered"
    );
    shut(client, handle);
}

#[test]
fn a_buffer_with_no_file_behind_it_is_read_as_the_language_its_client_declared() {
    // Nothing in `untitled:Untitled-1` says which of the two languages
    // it holds, and read as SysML this KerML does not parse: `frame` is
    // a keyword there and an ordinary name here.
    let uri = "untitled:Untitled-1";
    let (mut client, handle, diagnostics) = opened_as(
        uri,
        "kerml",
        "package P {\n    class frame;\n    feature f : frame;\n}\n",
    );
    assert_eq!(
        diagnostics["diagnostics"].as_array().map(Vec::len),
        Some(0),
        "{diagnostics}"
    );

    // and what may be renamed to is what this language reads as a name:
    // `part` is a keyword in SysML and nothing of the kind in KerML
    let renamed = client.send_request(
        lsp_types::request::Rename::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 2, "character": 12 },
            "newName": "part",
        }),
    );
    assert!(renamed.error.is_none(), "{:?}", renamed.error);
    shut(client, handle);
}

#[test]
fn a_layout_command_that_never_answers_does_not_take_the_server_with_it() {
    // ELK is a child process, and this server has one thread. Waiting on
    // it left an editor with no diagnostics, no completion and no
    // navigation in any file until the window was reloaded.
    use std::os::unix::fs::PermissionsExt;
    let fake = std::env::temp_dir().join(format!("sysml-e2e-stuck-elk-{}", std::process::id()));
    std::fs::write(&fake, "#!/bin/sh\nsleep 5\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

    let (server_side, client_side) = Connection::memory();
    let handle = std::thread::spawn(move || sysml_lsp::run(&server_side).unwrap());
    let mut client = Client {
        connection: client_side,
        next_id: 1,
    };
    client.request(
        lsp_types::request::Initialize::METHOD,
        json!({ "capabilities": {}, "initializationOptions": {
            "elkCommand": fake.to_str().unwrap(), "elkTimeoutMs": 200
        }}),
    );
    client.notify(lsp_types::notification::Initialized::METHOD, json!({}));
    let uri = "file:///stuck.sysml";
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": { "uri": uri, "languageId": "sysml", "version": 1,
                 "text": "part def A;\npart def B :> A;\n" } }),
    );
    client.wait_diagnostics();

    let result = client.request("sysml/diagram", json!({ "uri": uri, "layout": "elk" }));
    let svg = result["svg"].as_str().unwrap();
    // the built-in layout drew it, and the server is still answering
    assert!(
        svg.contains("marker-end=\"url(#specialization)\""),
        "{svg:.240}"
    );
    let symbols = client.request(
        lsp_types::request::DocumentSymbolRequest::METHOD,
        json!({ "textDocument": { "uri": uri } }),
    );
    assert_eq!(symbols[0]["name"], "A");
    shut(client, handle);
    std::fs::remove_file(&fake).ok();
}

/// An editor is told what the specification requires, and not only what
/// resolves.
///
/// A model whose every name resolves can still be one the standard
/// rejects. Asked of one open document the constraints cost about a
/// millisecond, so they can be answered while it is typed -- but only
/// once it parses and resolves, and only with the library that they are
/// written against.
#[test]
fn diagnostics_say_what_the_specification_requires() {
    let library = std::path::Path::new("../../vendor/sysml-v2-release/sysml.library");
    if !library.is_dir() {
        return; // the corpus submodule is not checked out
    }
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
            "initializationOptions": { "libraryPath": library.to_str().unwrap() },
        }),
    );
    client.notify(lsp_types::notification::Initialized::METHOD, json!({}));

    // an objective belongs to a case, and this is a part: every name
    // here resolves, and the model is still one the standard rejects
    let uri = "file:///rules.sysml";
    let text = "package P {\n    part def Engine {\n        objective misplaced;\n    }\n}\n";
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": { "uri": uri, "languageId": "sysml", "version": 1, "text": text } }),
    );
    let diags = client.wait_diagnostics();
    let list = diags["diagnostics"].as_array().unwrap();
    let broken: Vec<&Value> = list
        .iter()
        .filter(|d| d["code"] == "validateObjectiveMembershipOwningType")
        .collect();
    assert_eq!(broken.len(), 1, "{list:?}");
    assert_eq!(broken[0]["range"]["start"]["line"], 2, "{broken:?}");
    assert!(
        broken[0]["message"]
            .as_str()
            .unwrap()
            .contains("CaseDefinition"),
        "{broken:?}"
    );

    // put where it belongs, nothing is said about it
    let right = "package P {\n    case def Trip {\n        objective placed;\n    }\n}\n";
    client.notify(
        lsp_types::notification::DidChangeTextDocument::METHOD,
        json!({
            "textDocument": { "uri": uri, "version": 2 },
            "contentChanges": [{ "text": right }],
        }),
    );
    let diags = client.wait_diagnostics();
    assert_eq!(diags["diagnostics"].as_array().unwrap().len(), 0, "{diags}");

    client.request(lsp_types::request::Shutdown::METHOD, json!(null));
    client.notify(lsp_types::notification::Exit::METHOD, json!(null));
    handle.join().unwrap();
}

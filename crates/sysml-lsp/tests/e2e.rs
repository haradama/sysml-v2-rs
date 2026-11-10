//! End-to-end test driving the language server over an in-memory
//! connection: initialize, open a document, receive diagnostics, jump to a
//! definition, hover, and format.

mod common;

use common::{serving, Client};
use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use serde_json::{json, Value};

#[test]
fn serves_diagnostics_definition_hover_and_formatting() {
    let (mut client, handle) = serving();

    // handshake
    client.initialize();

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
    let (mut client, handle) = serving();
    client.initialize();

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

    // A file of usages declares no definition of its own: read alone it
    // has nothing to draw, and read with the folder it was written in it
    // has the definitions that folder declares.
    let usages = "file:///usages.sysml";
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": { "uri": usages, "languageId": "sysml", "version": 1,
                                  "text": "package U {\n\tpart e : Engine;\n}\n" } }),
    );
    client.wait_diagnostics();
    let alone = client.request("sysml/diagram", json!({ "uri": usages }));
    let svg = alone["svg"].as_str().unwrap();
    assert!(!svg.contains("<rect class=\"box\""), "{svg}");
    let folder = client.request(
        "sysml/diagram",
        json!({ "uri": usages, "scope": "directory" }),
    );
    let svg = folder["svg"].as_str().unwrap();
    assert!(svg.contains("<rect class=\"box\""), "{svg}");
    assert!(svg.contains("Engine"), "{svg}");

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
    let response = client.send_request("sysml/diagram", json!({ "uri": 42 }));
    assert!(response.error.is_some());

    client.request(lsp_types::request::Shutdown::METHOD, Value::Null);
    client.notify(lsp_types::notification::Exit::METHOD, Value::Null);
    handle.join().unwrap();
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
    let (mut client, handle) = serving();
    client.initialize();
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": {
            "uri": uri, "languageId": language, "version": 1, "text": text
        }}),
    );
    let diagnostics = client.wait_diagnostics();
    (client, handle, diagnostics)
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

    // `''` reads as a name and names nothing, which is what it is
    // refused for -- not for being a name already taken
    let empty = rename(&mut client, "''");
    assert!(
        empty
            .error
            .as_ref()
            .is_some_and(|err| err.message.contains("is an empty name")),
        "{:?}",
        empty.error
    );
    client.stop(handle);
}

/// A feature that redefines two things answers to the name of the first
/// of them alone, so renaming the second leaves every mention of it
/// standing -- those are spelled with the first name and follow it.
#[test]
fn renaming_the_second_name_a_redefinition_borrows_leaves_the_first_alone() {
    let uri = "file:///two.sysml";
    let (mut client, handle) = opened(
        uri,
        "package Demo {\n\
         \x20   part driver;\n\
         \x20   part driver_b;\n\
         \x20   part outer {\n\
         \x20       part :>> driver :>> driver_b;\n\
         \x20       part also subsets driver;\n\
         \x20   }\n\
         }\n",
    );
    let renamed = client.send_request(
        lsp_types::request::Rename::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 2, "character": 11 },
            "newName": "operator",
        }),
    );
    assert!(renamed.error.is_none(), "{:?}", renamed.error);

    // the declaration and the `:>> driver_b` that names it, and nothing
    // on the line that says `subsets driver`
    let edits = renamed.result.unwrap()["changes"][uri]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(edits.len(), 2, "{edits:?}");
    assert!(
        edits.iter().all(|edit| edit["range"]["start"]["line"] != 5),
        "{edits:?}"
    );
    client.stop(handle);
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
    client.stop(handle);
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
    client.stop(handle);
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
    client.stop(handle);
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
    client.stop(handle);
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
    client.stop(handle);
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
    client.stop(handle);
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
    let library = std::path::Path::new("../sysml-stdlib/library");
    if !library.is_dir() {
        return; // the corpus submodule is not checked out
    }
    let (mut client, handle) = serving();
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

/// `formatWidth` is what the client says about where a line gives way,
/// and the server formats to it.
///
/// The setting is read at initialize, as the library path is: an editor
/// that changes it restarts the server.
#[test]
fn the_client_says_how_wide_a_line_may_be() {
    let uri = "file:///wide.sysml";
    let text = "package P {\n\tpart def A {\n\t\tattribute a = b and c and d and e;\n\t}\n}\n";
    let formatted = |width: Value| {
        let (mut client, handle) = serving();
        let mut options = json!({ "noLibrary": true });
        if !width.is_null() {
            options["formatWidth"] = width;
        }
        client.initialize_with(options);
        client.notify(
            lsp_types::notification::DidOpenTextDocument::METHOD,
            json!({ "textDocument": {
                "uri": uri, "languageId": "sysml", "version": 1, "text": text,
            }}),
        );
        client.wait_diagnostics();
        let edits = client.request(
            lsp_types::request::Formatting::METHOD,
            json!({
                "textDocument": { "uri": uri },
                "options": { "tabSize": 4, "insertSpaces": true }
            }),
        );
        let said = edits[0]["newText"].as_str().unwrap_or(text).to_string();
        client.stop(handle);
        said
    };

    // told nothing, the line holds; told thirty columns, it gives way
    let wide = formatted(Value::Null);
    assert_eq!(
        wide.lines().filter(|line| line.contains("and")).count(),
        1,
        "{wide}"
    );
    let narrow = formatted(json!(30));
    assert!(
        narrow.lines().filter(|line| line.contains("and")).count() > 1,
        "{narrow}"
    );
    assert_eq!(narrow.matches("and").count(), 3, "{narrow}");
    // a settings file that holds the number as a string means the same
    assert_eq!(formatted(json!("30")), narrow);
    // and one that holds no number at all leaves the width as it was
    assert_eq!(formatted(json!("narrow")), wide);
}

/// A width that is no number is said to be, where the client collects
/// this server's log, rather than dropped without a word.
#[test]
fn a_width_that_is_no_number_is_said_to_be() {
    let (mut client, handle) = serving();
    client.initialize_with(json!({ "noLibrary": true, "formatWidth": "narrow" }));
    let said = loop {
        if let lsp_server::Message::Notification(note) = client.recv() {
            if note.method == "window/logMessage" {
                break note.params["message"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
            }
        }
    };
    assert!(said.contains("format width"), "{said}");
    client.stop(handle);
}

/// The client says how the preview is painted, by name or in full, and
/// a skin that will not read leaves the drawing as it was.
#[test]
fn the_client_says_how_the_preview_is_painted() {
    let uri = "file:///painted.sysml";
    let text = "package P {\n\tpart def A;\n}\n";
    let drawn = |skin: Value| {
        let (mut client, handle) = serving();
        let mut options = json!({ "noLibrary": true });
        if !skin.is_null() {
            options["skin"] = skin;
        }
        client.initialize_with(options);
        client.notify(
            lsp_types::notification::DidOpenTextDocument::METHOD,
            json!({ "textDocument": {
                "uri": uri, "languageId": "sysml", "version": 1, "text": text,
            }}),
        );
        client.wait_diagnostics();
        let result = client.request("sysml/diagram", json!({ "uri": uri }));
        let svg = result["svg"].as_str().unwrap().to_string();
        client.stop(handle);
        svg
    };

    // told nothing, the drawing is the one it always was
    let plain = drawn(Value::Null);
    assert!(plain.contains("prefers-color-scheme"), "{plain:.200}");
    assert!(!plain.contains("kind-"), "{plain:.200}");

    // told a name
    let mono = drawn(json!("mono"));
    assert!(!mono.contains("prefers-color-scheme"), "{mono:.200}");

    // told what to paint it in
    let painted = drawn(json!({ "light": { "kinds": { "part def": "#e8f0fe" } } }));
    assert!(
        painted.contains(".kind-part-def .box { fill: #e8f0fe; }"),
        "{painted:.400}"
    );

    // and told something that is not a skin, it draws what it drew
    assert_eq!(drawn(json!({ "light": { "paper": "#fff" } })), plain);
}

/// After `P::` the completion is what `P` holds, and no keyword: one was
/// never what could be written there, and the names around the cursor
/// were offered in place of `P`'s.
#[test]
fn completion_after_a_qualifier_offers_what_the_qualifier_holds() {
    let (mut client, handle) = serving();
    client.initialize();
    let uri = "file:///qualified.sysml";
    let text = "package P {\n    part def Wheel;\n}\npackage Q {\n    import P::\n}\n";
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": { "uri": uri, "languageId": "sysml", "version": 1, "text": text } }),
    );
    client.wait_diagnostics();
    let completions = client.request(
        lsp_types::request::Completion::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 4, "character": 14 }
        }),
    );
    let labels: Vec<&str> = completions
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["Wheel"]);

    client.request(lsp_types::request::Shutdown::METHOD, Value::Null);
    client.notify(lsp_types::notification::Exit::METHOD, Value::Null);
    handle.join().unwrap();
}

/// Open several documents on a fresh server, each in the language its
/// client declares, and wait until each has been heard about.
fn all_opened(docs: &[(&str, &str, &str)]) -> (Client, std::thread::JoinHandle<()>) {
    let (mut client, handle) = serving();
    client.initialize();
    for (uri, language, text) in docs {
        client.notify(
            lsp_types::notification::DidOpenTextDocument::METHOD,
            json!({ "textDocument": {
                "uri": uri, "languageId": language, "version": 1, "text": text
            }}),
        );
    }
    client.diagnostics(docs.len());
    (client, handle)
}

/// Where a list of ranges starts and ends, as `line:column-line:column`,
/// sorted, which is what two answers about the same places agree on.
fn spans<'a>(ranges: impl IntoIterator<Item = &'a Value>) -> Vec<String> {
    let at = |position: &Value| format!("{}:{}", position["line"], position["character"]);
    let mut spans: Vec<String> = ranges
        .into_iter()
        .map(|range| format!("{}-{}", at(&range["start"]), at(&range["end"])))
        .collect();
    spans.sort();
    spans
}

fn rename_at(client: &mut Client, uri: &str, line: u32, character: u32, to: &str) -> Value {
    client.request(
        lsp_types::request::Rename::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
            "newName": to,
        }),
    )
}

fn references_at(client: &mut Client, uri: &str, line: u32, character: u32) -> Vec<String> {
    let found = client.request(
        lsp_types::request::References::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
            "context": { "includeDeclaration": true },
        }),
    );
    spans(found.as_array().unwrap().iter().map(|it| &it["range"]))
}

/// `part` is a name in KerML and a keyword in SysML. A rename asked in
/// a KerML file read it as a name and wrote it bare into a SysML file
/// that names the same element, which then stopped parsing; each file
/// gets it the way it has to be written there.
#[test]
fn a_new_name_is_quoted_in_a_file_that_reads_it_as_a_keyword() {
    let kerml = "file:///p/k.kerml";
    let sysml = "file:///p/a.sysml";
    let (mut client, handle) = all_opened(&[
        (kerml, "kerml", "package K {\n    classifier Foo;\n}\n"),
        (sysml, "sysml", "part x : K::Foo;\n"),
    ]);
    let edit = rename_at(&mut client, kerml, 1, 16, "part");
    assert_eq!(edit["changes"][kerml][0]["newText"], "part", "{edit}");
    assert_eq!(edit["changes"][sysml][0]["newText"], "'part'", "{edit}");
    client.stop(handle);
}

/// `<W>` is a name of the element as much as `Wheel` is. Asked about at
/// the `W` that declares it, references and rename answered nothing; a
/// rename asked at a `W` that uses it rewrote `Wheel` and left the `W`.
#[test]
fn a_short_name_is_found_and_renamed_where_it_is_pointed_at() {
    let uri = "file:///short-names.sysml";
    let (mut client, handle) = opened(
        uri,
        "part def <W> Wheel;\npart w : W;\npart v : Wheel;\npart def <X>;\n",
    );

    // both spellings are the one element, wherever it is asked about
    let all = ["0:13-0:18", "1:9-1:10", "2:9-2:14"];
    assert_eq!(references_at(&mut client, uri, 0, 10), all);
    assert_eq!(references_at(&mut client, uri, 1, 9), all);
    // a declaration with only a short name is named by it, not by the
    // whole of itself
    assert_eq!(references_at(&mut client, uri, 3, 10), ["3:10-3:11"]);

    // renamed by the short name, at a use or at the declaration, the
    // short name is what changes
    for (line, character) in [(1, 9), (0, 10)] {
        let edit = rename_at(&mut client, uri, line, character, "T");
        let edits = edit["changes"][uri].as_array().unwrap();
        assert_eq!(
            spans(edits.iter().map(|it| &it["range"])),
            ["0:10-0:11", "1:9-1:10"]
        );
        assert!(edits.iter().all(|it| it["newText"] == "T"), "{edit}");
    }
    client.stop(handle);
}

/// Find-references is what a reader asks before a rename, to see what
/// it will touch. It answered with the whole of `a.c` or `P::Wheel`, and
/// left out what reaches the element through a feature that borrows its
/// name, which the rename rewrites.
#[test]
fn references_are_the_places_a_rename_rewrites() {
    let uri = "file:///agree.sysml";
    let (mut client, handle) = opened(
        uri,
        "part def A { part c; }\n\
         part a : A { part :>> c; }\n\
         part z :> a.c;\n\
         package P { part def Wheel; }\n\
         part y : P::Wheel;\n",
    );
    let found = references_at(&mut client, uri, 0, 18);
    let edit = rename_at(&mut client, uri, 0, 18, "d");
    let renamed = spans(
        edit["changes"][uri]
            .as_array()
            .unwrap()
            .iter()
            .map(|it| &it["range"]),
    );
    assert_eq!(found, renamed);
    assert!(found.contains(&"2:12-2:13".to_string()), "{found:?}");

    assert_eq!(
        references_at(&mut client, uri, 3, 21),
        ["3:21-3:26", "4:12-4:17"]
    );
    client.stop(handle);
}

/// Every name in a document symbol tree, depth first.
fn symbol_names(symbols: &Value, out: &mut Vec<String>) {
    for symbol in symbols.as_array().into_iter().flatten() {
        out.push(symbol["name"].as_str().unwrap().to_string());
        symbol_names(&symbol["children"], out);
    }
}

/// An element declared without a name was left out of the outline, and
/// everything named inside it with it.
#[test]
fn an_unnamed_element_stays_in_the_outline_with_what_it_holds() {
    let sysml = "file:///outline.sysml";
    let kerml = "file:///outline.kerml";
    let (mut client, handle) = all_opened(&[
        (
            sysml,
            "sysml",
            "package Q {\n\
             \x20   part def A { part x; doc /* about A */ }\n\
             \x20   part a : A { part :>> x { part inner; } attribute v = 1 + 2; }\n\
             \x20   part { part alsoInner; }\n\
             \x20   connection { end part e1; }\n\
             \x20   perform action { action step1; }\n\
             \x20   part def <X>;\n\
             }\n",
        ),
        // an expression is no declaration, but one that holds a
        // parameter is somewhere the parameter is
        (kerml, "kerml", "package K {\n    expr { in y; }\n}\n"),
    ]);
    let names = |client: &mut Client, uri: &str| {
        let symbols = client.request(
            lsp_types::request::DocumentSymbolRequest::METHOD,
            json!({ "textDocument": { "uri": uri } }),
        );
        let mut names = Vec::new();
        symbol_names(&symbols, &mut names);
        names
    };
    assert_eq!(
        names(&mut client, sysml),
        [
            "Q",
            "A",
            "x",
            "a",
            "x",
            "inner",
            "v",
            "(unnamed)",
            "alsoInner",
            "(unnamed)",
            "e1",
            "(unnamed)",
            "step1",
            "<X>"
        ]
    );
    assert_eq!(names(&mut client, kerml), ["K", "(unnamed)", "y"]);
    client.stop(handle);
}

/// Inside a comment, a string or a quoted name there is nothing to
/// complete, and a keyword of SysML is a name in KerML.
#[test]
fn completion_offers_nothing_inside_text_and_only_this_notation_s_keywords() {
    let sysml = "file:///completing.sysml";
    let kerml = "file:///completing.kerml";
    let (mut client, handle) = all_opened(&[
        (
            sysml,
            "sysml",
            "// a note\npackage P {\n    doc /* body */\n    attribute s = \"text\";\n    part 'quoted name';\n    \n}\n",
        ),
        (kerml, "kerml", "package K {\n    \n}\n"),
    ]);
    let labels = |client: &mut Client, uri: &str, line: u32, character: u32| -> Vec<String> {
        let items = client.request(
            lsp_types::request::Completion::METHOD,
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }),
        );
        items
            .as_array()
            .unwrap()
            .iter()
            .map(|it| it["label"].as_str().unwrap().to_string())
            .collect()
    };
    // in the note, at the end of its line, in a doc's body, in a string
    // and in a quoted name
    for (line, character) in [(0, 5), (0, 9), (2, 12), (3, 20), (4, 12)] {
        assert_eq!(
            labels(&mut client, sysml, line, character),
            [] as [String; 0],
            "{line}:{character}"
        );
    }
    let between = labels(&mut client, sysml, 5, 4);
    assert!(between.iter().any(|it| it == "part"), "{between:?}");

    let kerml_labels = labels(&mut client, kerml, 1, 4);
    assert!(kerml_labels.iter().any(|it| it == "classifier"));
    assert!(!kerml_labels.iter().any(|it| it == "part"));
    client.stop(handle);
}

/// The editor says how a level is indented, and formatting answers in it.
#[test]
fn formatting_indents_the_way_the_editor_asks() {
    let uri = "file:///indent.sysml";
    let text = "package P {\n    part def A {\n        part b;\n    }\n}\n";
    let (mut client, handle) = opened(uri, text);
    let mut formatted = |tab_size: u32, insert_spaces: bool| {
        let edits = client.request(
            lsp_types::request::Formatting::METHOD,
            json!({
                "textDocument": { "uri": uri },
                "options": { "tabSize": tab_size, "insertSpaces": insert_spaces }
            }),
        );
        edits[0]["newText"].as_str().unwrap_or(text).to_string()
    };
    assert_eq!(formatted(4, true), text);
    assert_eq!(
        formatted(2, true),
        "package P {\n  part def A {\n    part b;\n  }\n}\n"
    );
    assert_eq!(
        formatted(4, false),
        "package P {\n\tpart def A {\n\t\tpart b;\n\t}\n}\n"
    );
    client.stop(handle);
}

/// A name that resolves to nothing fails `sysml check`, so an editor is
/// told it is an error and not a warning.
#[test]
fn an_unresolved_name_is_an_error() {
    let (client, handle, said) = opened_as("file:///missing.sysml", "sysml", "part x : Missing;\n");
    let diagnostics = said["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "{said}");
    assert_eq!(diagnostics[0]["severity"], 1, "{said}");
    client.stop(handle);
}

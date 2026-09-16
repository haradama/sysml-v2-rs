//! What a name that resolved to nothing might have meant, asked for as
//! code actions.
//!
//! `Workspace::suggestions` tells two mistakes apart and the server has
//! to keep them apart: a name the workspace declares somewhere is a right
//! one that nothing brought into scope, and a name nothing declares is a
//! wrong one. Both come back as an edit, and the edit is what says which.

mod common;

use common::{serving, Client};
use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use serde_json::{json, Value};

/// Open `text` and hand back the diagnostics it produced.
fn opened(client: &mut Client, uri: &str, text: &str) -> Vec<Value> {
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": { "uri": uri, "languageId": "sysml", "version": 1, "text": text } }),
    );
    client.diagnostics_for(uri)["diagnostics"]
        .as_array()
        .expect("the diagnostics that were published")
        .clone()
}

/// What the server offers over the range of the diagnostic about `about`.
fn offered(client: &mut Client, uri: &str, found: &[Value], about: &str) -> Vec<(String, String)> {
    let diagnostic = found
        .iter()
        .find(|it| {
            it["message"]
                .as_str()
                .is_some_and(|said| said.contains(about))
        })
        .unwrap_or_else(|| panic!("nothing was said about `{about}` in {found:?}"));
    let actions = client.request(
        lsp_types::request::CodeActionRequest::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "range": diagnostic["range"],
            "context": { "diagnostics": [diagnostic] },
        }),
    );
    actions
        .as_array()
        .expect("an array of actions")
        .iter()
        .map(|action| {
            let edit = &action["edit"]["changes"][uri][0];
            (
                action["title"].as_str().unwrap().to_string(),
                edit["newText"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// A name nothing declares is a wrong one, and what it wants is the
/// nearest thing that is declared.
#[test]
fn a_misspelled_name_is_offered_the_one_it_is_near() {
    let (mut client, handle) = serving();
    client.initialize();

    let uri = "file:///typo.sysml";
    let found = opened(
        &mut client,
        uri,
        "package P {\n    part def Wheel;\n    part w : Wheeel;\n}\n",
    );
    let offers = offered(&mut client, uri, &found, "Wheeel");
    assert!(
        offers
            .iter()
            .any(|(title, text)| text == "P::Wheel" && title == "did you mean `P::Wheel`?"),
        "{offers:?}"
    );

    client.stop(handle);
}

/// A name the workspace declares somewhere is a right one that nothing
/// brought into scope. Spelled from the root it resolves where it stands,
/// which is an edit a client can apply without touching the imports.
#[test]
fn a_name_that_is_declared_elsewhere_is_offered_where_it_lives() {
    let (mut client, handle) = serving();
    client.initialize();

    let uri = "file:///unimported.sysml";
    let found = opened(
        &mut client,
        uri,
        "package Parts {\n    part def Wheel;\n}\npackage P {\n    part w : Wheel;\n}\n",
    );
    let offers = offered(&mut client, uri, &found, "Wheel");
    assert!(
        offers
            .iter()
            .any(|(title, text)| text == "Parts::Wheel" && title.contains("declared elsewhere")),
        "{offers:?}"
    );

    client.stop(handle);
}

/// A qualified name is wrong in its last segment and right in the rest,
/// so what is offered keeps the rest.
#[test]
fn a_qualified_name_keeps_everything_that_was_right() {
    let (mut client, handle) = serving();
    client.initialize();

    let uri = "file:///qualified.sysml";
    let found = opened(
        &mut client,
        uri,
        "package Parts {\n    part def Wheel;\n}\npackage P {\n    part w : Parts::Wheeel;\n}\n",
    );
    let offers = offered(&mut client, uri, &found, "Wheeel");
    assert!(
        offers.iter().any(|(_, text)| text == "Parts::Wheel"),
        "{offers:?}"
    );

    client.stop(handle);
}

/// Where nothing resolved to nothing, there is nothing to offer -- and
/// the request still answers, because a client asks over every range a
/// reader points at.
#[test]
fn a_model_with_nothing_wrong_is_offered_nothing() {
    let (mut client, handle) = serving();
    client.initialize();

    let uri = "file:///fine.sysml";
    opened(&mut client, uri, "package P {\n    part def Wheel;\n}\n");
    let actions = client.request(
        lsp_types::request::CodeActionRequest::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 20 } },
            "context": { "diagnostics": [] },
        }),
    );
    assert_eq!(actions.as_array().map(Vec::len), Some(0), "{actions:?}");

    client.stop(handle);
}

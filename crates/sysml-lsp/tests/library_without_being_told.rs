//! The standard library, without being handed one.
//!
//! Almost nothing in a SysML model resolves without it, so a server
//! started by an editor extension and told nothing else -- the common
//! case, and the one nobody configures -- would otherwise underline
//! every name in every file. The copy built into the binary is what it
//! answers against instead.

mod common;

use common::serving;
use lsp_types::notification::Notification as _;
use serde_json::json;

/// `ISQ::MassValue` is in the Quantities and Units domain library and is
/// reached through two levels of import, so a copy that is there but
/// truncated fails this as surely as no copy at all.
const REACHES: &str = "package P {\n\
                       \tprivate import ISQ::*;\n\
                       \tpart def Tank {\n\
                       \t\tattribute fuel : MassValue;\n\
                       \t}\n\
                       }\n";

fn diagnostics_for(options: serde_json::Value, text: &str) -> Vec<serde_json::Value> {
    let (mut client, handle) = serving();
    client.initialize_with(options);
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": {
            "uri": "file:///tank.sysml", "languageId": "sysml", "version": 1, "text": text,
        }}),
    );
    let published = client.wait_diagnostics();
    let found = published["diagnostics"].as_array().unwrap().clone();
    client.stop(handle);
    found
}

#[test]
fn a_model_resolves_against_the_copy_built_in() {
    let found = diagnostics_for(json!({}), REACHES);
    assert!(found.is_empty(), "{found:?}");
}

/// And `noLibrary` is what says otherwise, for a client that means to
/// ask about a model on its own terms.
#[test]
fn no_library_is_the_way_to_say_otherwise() {
    let found = diagnostics_for(json!({ "noLibrary": true }), REACHES);
    assert!(
        found.iter().any(|it| it["message"]
            .as_str()
            .is_some_and(|said| said.contains("MassValue"))),
        "{found:?}"
    );
}

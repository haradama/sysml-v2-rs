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

/// A library path that will not load falls back to the copy built in,
/// rather than to no library at all.
///
/// One mistyped `sysml.library.path` used to underline every name in
/// every file, silently: the server degraded to an empty library and
/// nothing anywhere said why. The command line and the MCP server had
/// both stopped doing that; this was the front end left.
#[test]
fn a_library_that_will_not_load_falls_back_to_the_one_built_in() {
    let found = diagnostics_for(json!({ "libraryPath": "/nowhere/at/all" }), REACHES);
    assert!(found.is_empty(), "{found:?}");
}

/// And so does one that is there and holds no model at all, which is
/// what a path pointed at the wrong directory looks like.
#[test]
fn a_library_directory_with_nothing_in_it_falls_back_too() {
    let empty = std::env::temp_dir().join("sysml-lsp-empty-library");
    std::fs::create_dir_all(&empty).unwrap();
    let found = diagnostics_for(json!({ "libraryPath": empty.to_str().unwrap() }), REACHES);
    assert!(found.is_empty(), "{found:?}");
}

/// A library that is there and cannot be read is the other way one
/// fails to load, and falls back the same way.
///
/// The walk over a directory swallows what it cannot enter and reaches
/// nothing; what errs is a file it found and cannot open. So the file
/// is there, and unreadable.
#[cfg(unix)]
#[test]
fn a_library_that_cannot_be_read_falls_back_too() {
    use std::os::unix::fs::PermissionsExt;
    let shut = std::env::temp_dir().join("sysml-lsp-library-shut");
    std::fs::create_dir_all(&shut).unwrap();
    let hidden = shut.join("hidden.sysml");
    std::fs::write(&hidden, "package Hidden;\n").unwrap();
    std::fs::set_permissions(&hidden, std::fs::Permissions::from_mode(0o000)).unwrap();
    let found = diagnostics_for(json!({ "libraryPath": shut.to_str().unwrap() }), REACHES);
    std::fs::set_permissions(&hidden, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(found.is_empty(), "{found:?}");
}

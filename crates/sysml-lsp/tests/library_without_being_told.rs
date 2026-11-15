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
use lsp_types::request::Request as _;
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

/// The copy built in was no file, so nothing in it had a place for a
/// workspace-symbol answer to go to -- and the search was cut to its limit
/// before those were dropped. (Written out to a cache now, it has places,
/// and the project's own are ranked before it for the same reason.) A query as short as `e` matches thousands of
/// the library's names, so the limit was spent on them and the answer held
/// one of this document's three. What has nowhere to go is passed over
/// before anything is counted.
#[test]
fn the_library_built_in_does_not_crowd_out_the_documents_own_symbols() {
    let (mut client, handle) = serving();
    client.initialize_with(json!({}));
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": {
            "uri": "file:///p/a.sysml", "languageId": "sysml", "version": 1,
            "text": "package Mine {\n  attribute def MyValueThing;\n  part def Engine;\n  part def Part;\n}\n",
        }}),
    );
    client.wait_diagnostics();
    let found = client.request(
        lsp_types::request::WorkspaceSymbolRequest::METHOD,
        json!({ "query": "e" }),
    );
    let all: Vec<&str> = found
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["name"].as_str().unwrap())
        .collect();
    // the library may follow, now that it has files to go to, but the
    // document's own come first, every one of them
    let mut names = all[..3.min(all.len())].to_vec();
    names.sort_unstable();
    assert_eq!(names, ["Engine", "Mine", "MyValueThing"], "{all:?}");
    client.stop(handle);
}

/// A name from the copy built in can be jumped to.
///
/// That copy is no file, so a definition inside it had no URL and
/// go-to-definition on `Part` answered null, while the crate said that
/// definitions inside the library can be jumped to. Where there is a
/// filesystem the copy is written out to a cache directory and answered
/// from there, as a library directory the client named would be.
#[cfg(not(target_family = "wasm"))]
#[test]
fn a_definition_in_the_copy_built_in_can_be_jumped_to() {
    let (mut client, handle) = serving();
    client.initialize_with(json!({}));
    let uri = "file:///p/car.sysml";
    let text = "part def Car :> Parts::Part;\n";
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": {
            "uri": uri, "languageId": "sysml", "version": 1, "text": text,
        }}),
    );
    client.wait_diagnostics();
    let definition = client.request(
        lsp_types::request::GotoDefinition::METHOD,
        json!({ "textDocument": { "uri": uri }, "position": { "line": 0, "character": 24 } }),
    );
    let target = definition["uri"]
        .as_str()
        .unwrap_or_else(|| panic!("{definition}"));
    assert!(target.starts_with("file://"), "{target}");
    assert!(target.ends_with("/Parts.sysml"), "{target}");
    // and the place is where the file written there says `Part` is
    let path = lsp_types::Url::parse(target)
        .unwrap()
        .to_file_path()
        .unwrap();
    let written = std::fs::read_to_string(path).unwrap();
    let line = definition["range"]["start"]["line"].as_u64().unwrap() as usize;
    let column = definition["range"]["start"]["character"].as_u64().unwrap() as usize;
    let at = written.lines().nth(line).unwrap();
    assert!(at[column..].starts_with("Part"), "{at}");

    // opened for reading, the jump still lands in the library's copy
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": {
            "uri": target, "languageId": "sysml", "version": 1, "text": written,
        }}),
    );
    let again = client.request(
        lsp_types::request::GotoDefinition::METHOD,
        json!({ "textDocument": { "uri": uri }, "position": { "line": 0, "character": 24 } }),
    );
    assert_eq!(again["uri"], definition["uri"], "{again}");
    client.stop(handle);
}

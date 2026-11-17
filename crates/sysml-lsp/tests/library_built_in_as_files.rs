//! The standard library built in, written out where a client can go.
//!
//! The copy inside the binary is no file, so where there is a filesystem
//! it is written out to a cache directory, and answers that point into
//! the library point there. These load that copy in every test, which is
//! why they are a binary apart from `library_without_being_told`: under
//! coverage instrumentation the tests of one binary run no faster in
//! parallel than one after another, and seven loads of the library in
//! one binary take longer than the client waits for an answer.

mod common;

use common::serving;
use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use serde_json::json;

/// The copy built in is written out to a cache so that it has places for
/// a workspace-symbol answer to go to, and the project's own names are
/// ranked before it. Were the search cut to its limit before the names
/// with nowhere to go were dropped, a query as short as `e`, which
/// matches a great many of the library's names, would spend the limit on
/// them and the answer would hold one of this document's three. What has
/// nowhere to go is passed over before anything is counted.
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
    // the library may follow, since it has files to go to, but the
    // document's own come first, every one of them
    let mut names = all[..3.min(all.len())].to_vec();
    names.sort_unstable();
    assert_eq!(names, ["Engine", "Mine", "MyValueThing"], "{all:?}");
    client.stop(handle);
}

/// A name from the copy built in can be jumped to.
///
/// That copy is no file, so a definition inside it would have no URL and
/// go-to-definition on `Part` would answer null, while the crate says that
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

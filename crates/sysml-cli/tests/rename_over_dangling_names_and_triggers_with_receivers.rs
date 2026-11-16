//! A rename in a model that already has a name that dangles, and a
//! trigger that says what it is received through.
use std::collections::HashMap;
use std::io::Cursor;

use serde_json::{json, Value};
use sysml_cli::textual::write_with;
use sysml_model::ElementId;

fn call(name: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
    })
}

fn drive(lines: &[Value]) -> Vec<Value> {
    let mut server = sysml_cli::mcp::Server::without_library(None);
    let input: String = lines.iter().map(|line| format!("{line}\n")).collect();
    let mut out: Vec<u8> = Vec::new();
    sysml_cli::mcp::serve(&mut server, Cursor::new(input.as_bytes()), &mut out).unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON"))
        .collect()
}

fn answered(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("a text content: {response}"));
    serde_json::from_str(text).unwrap()
}

#[test]
fn a_name_that_dangled_before_and_dangles_after_is_not_introduced() {
    let text = "package P { part def A; part x : A; part y : Gone; part z : Gone; }";
    let answers = drive(&[call(
        "rename",
        json!({ "text": text, "element": "P::A", "new_name": "B" }),
    )]);
    let said = answered(&answers[0]);
    let checked = &said["checked"];
    assert_eq!(checked["unresolved_before"], 2, "{said}");
    assert_eq!(checked["unresolved"], 2, "{said}");
    assert_eq!(checked["introduced"], json!([]), "{said}");
    assert_eq!(checked["ok"], true, "{said}");
}

fn written(text: &str) -> String {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file("m.sysml", text);
    ws.resolve_all();
    let mut extras = sysml_interchange::Extras::default();
    let ids: Vec<ElementId> = ws.model().ids().collect();
    for id in ids {
        if let Some(target) = ws.imported_alias(id).or_else(|| ws.import_of(id)) {
            extras.import_targets.insert(id, target);
        }
    }
    let doc = sysml_interchange::to_json_with(ws.model(), &extras);
    let bytes = serde_json::to_vec(&doc).unwrap();
    let (model, roots, _) = sysml_interchange::read_json(&bytes).unwrap();
    write_with(&model, &roots, &HashMap::new(), true).sysml
}

#[test]
fn an_accept_that_says_what_it_comes_through_is_written_with_via() {
    let out = written("package P { part def Sig; port p; action a { accept Sig via p; } }");
    assert!(out.contains("accept Sig via p"), "{out}");
}

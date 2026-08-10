//! The server as a client meets it: a session over the stdio transport,
//! and every way a call can be refused.

use std::io::{Cursor, Write};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

/// Drive `serve` over the lines given and read the answers back.
fn session(lines: &[Value]) -> Vec<Value> {
    let input: String = lines
        .iter()
        .map(|line| format!("{line}\n"))
        .collect::<String>();
    talk(&input)
}

fn talk(input: &str) -> Vec<Value> {
    let mut server = sysml_mcp::Server::new(None);
    let mut out: Vec<u8> = Vec::new();
    sysml_mcp::serve(&mut server, Cursor::new(input.as_bytes()), &mut out).unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).expect("every answer is JSON"))
        .collect()
}

/// The JSON a tool answered with, out of its content.
fn answered(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("a text content");
    serde_json::from_str(text).expect("the content is JSON")
}

fn call(name: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
    })
}

#[test]
fn a_client_handshakes_lists_and_calls() {
    let answers = session(&[
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": {"protocolVersion": "2024-11-05"}}),
        // a notification is told, not asked, and is not answered
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}),
    ]);
    assert_eq!(answers.len(), 3, "the notification drew no answer");

    // the version the client asked for is the one it is answered in
    assert_eq!(answers[0]["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(answers[0]["result"]["serverInfo"]["name"], "sysml-mcp");
    assert!(answers[0]["result"]["capabilities"]["tools"].is_object());
    assert_eq!(answers[1]["result"], json!({}));

    let tools: Vec<&str> = answers[2]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(tools, ["check", "visible_names", "library_search"]);
    // a client builds its call from the schema, so it has to be there
    assert!(answers[2]["result"]["tools"][1]["inputSchema"]["properties"]["line"].is_object());
}

#[test]
fn initialize_without_a_version_is_answered_in_ours() {
    let answers = session(&[json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"})]);
    assert_eq!(answers[0]["result"]["protocolVersion"], "2025-06-18");
}

#[test]
fn check_answers_for_the_syntax_first_and_the_names_after() {
    // what does not parse is said to not parse, and nothing is claimed
    // about names in a tree that was never built
    let answers = session(&[call(
        "check",
        json!({ "text": "package P {\n\tpart def A;\n" }),
    )]);
    let found = answered(&answers[0]);
    assert_eq!(found["ok"], false);
    assert_eq!(found["parseErrors"][0]["line"], 3);
    assert!(found["parseErrors"][0]["message"].is_string());
    assert!(found["unresolved"].is_null());

    // and what parses is answered for by name
    let answers = session(&[call(
        "check",
        json!({ "text": "package P {\n\tpart def A;\n\tpart b : Missing;\n}\n" }),
    )]);
    let found = answered(&answers[0]);
    assert_eq!(found["ok"], false);
    assert_eq!(found["unresolved"][0]["name"], "Missing");
    assert_eq!(found["unresolved"][0]["line"], 3);
    assert_eq!(found["unresolved"][0]["column"], 11);
    assert_eq!(found["references"], 1);

    // a model with nothing wrong says so
    let answers = session(&[call(
        "check",
        json!({ "text": "package P {\n\tpart def A;\n\tpart b : A;\n}\n" }),
    )]);
    let found = answered(&answers[0]);
    assert_eq!(found["ok"], true);
    assert_eq!(found["resolved"], 1);
}

#[test]
fn a_file_is_read_from_disk_and_its_name_picks_the_dialect() {
    let dir = std::env::temp_dir().join("sysml-mcp-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("read.kerml");
    std::fs::write(&path, "package P {\n\tclassifier A;\n}\n").unwrap();
    let answers = session(&[call("check", json!({ "path": path.to_str().unwrap() }))]);
    assert_eq!(answered(&answers[0])["ok"], true);

    // and a file that is not there is refused by name
    let answers = session(&[call("check", json!({ "path": "/nowhere/x.sysml" }))]);
    assert_eq!(answers[0]["result"]["isError"], true);
    assert!(answered(&answers[0])["error"]
        .as_str()
        .unwrap()
        .contains("cannot read"));
}

#[test]
fn visible_names_answers_for_the_point_it_is_asked_about() {
    let text = "package P {\n\tpart def Vehicle;\n\tpart def Wheel;\n\tpart car : \n}\n";
    let answers = session(&[call(
        "visible_names",
        json!({ "text": text, "line": 4, "column": 13 }),
    )]);
    let visible = answered(&answers[0]);
    let names: Vec<&str> = visible["visible"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"Vehicle"), "{names:?}");
    assert!(names.contains(&"Wheel"), "{names:?}");
    assert!(visible["visible"][0]["kind"].is_string());

    // a point the text does not reach is refused, not guessed at
    for (line, column) in [(99, 1), (1, 99), (0, 1), (1, 0)] {
        let answers = session(&[call(
            "visible_names",
            json!({ "text": text, "line": line, "column": column }),
        )]);
        assert_eq!(answers[0]["result"]["isError"], true, "{line}:{column}");
    }
}

#[test]
fn library_search_answers_out_of_the_library_it_was_given() {
    // with no library there is nothing to find, and saying so is the
    // honest answer
    let answers = session(&[call("library_search", json!({ "query": "Mass" }))]);
    assert_eq!(answered(&answers[0])["found"], json!([]));

    // the query and the limit are both optional, and the limit is capped
    let answers = session(&[call("library_search", json!({ "limit": 9000 }))]);
    assert_eq!(answered(&answers[0])["found"], json!([]));
}

#[test]
fn library_search_finds_what_the_library_declares() {
    let library = std::path::Path::new("../../vendor/sysml-v2-release/sysml.library");
    if !library.is_dir() {
        return; // the corpus submodule is not checked out
    }
    let mut server = sysml_mcp::Server::new(Some(library));
    let response = server
        .handle(&call(
            "library_search",
            json!({ "query": "massvalue", "limit": 3 }),
        ))
        .expect("a request is answered");
    let found = answered(&response);
    let first = &found["found"][0];
    assert!(
        first["name"].as_str().unwrap().contains("MassValue"),
        "{found}"
    );

    // what was asked for exactly comes first: searching `Natural` and
    // being handed two SI units before `ScalarValues::Natural` is an
    // answer that has to be read through rather than used
    let response = server
        .handle(&call(
            "library_search",
            json!({ "query": "natural", "limit": 3 }),
        ))
        .expect("a request is answered");
    assert_eq!(
        answered(&response)["found"][0]["name"],
        "ScalarValues::Natural",
        "{}",
        answered(&response)
    );
    assert!(first["kind"].is_string());
    assert!(first["documentation"].is_string());

    // and a model that names the library resolves against it
    let response = server
        .handle(&call(
            "check",
            json!({ "text": "package P {\n\tprivate import ScalarValues::*;\n\tattribute m : Real;\n}\n" }),
        ))
        .expect("a request is answered");
    assert_eq!(answered(&response)["ok"], true);
}

#[test]
fn what_the_server_cannot_do_it_says_rather_than_guesses() {
    let answers = session(&[
        json!({"jsonrpc": "2.0", "id": 1, "method": "no/such/method"}),
        call("no_such_tool", json!({})),
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {}}),
        call("check", json!({})),
        call("visible_names", json!({ "text": "package P;\n" })),
    ]);
    assert_eq!(answers[0]["error"]["code"], -32601);
    // an unknown tool is the client's mistake, a refused call the model's
    assert_eq!(answers[1]["error"]["code"], -32602);
    assert_eq!(answers[2]["error"]["code"], -32602);
    assert!(answered(&answers[3])["error"]
        .as_str()
        .unwrap()
        .contains("`text` or `path`"));
    assert!(answered(&answers[4])["error"]
        .as_str()
        .unwrap()
        .contains("`line`"));
}

#[test]
fn a_line_that_is_not_a_message_does_not_end_the_session() {
    let answers = talk("not json at all\n\n{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}\n");
    assert_eq!(answers.len(), 2, "the blank line was passed over");
    assert_eq!(answers[0]["error"]["code"], -32700);
    assert_eq!(answers[1]["id"], 7);

    // a message that is no request at all is passed over in silence
    assert!(talk("{\"jsonrpc\":\"2.0\"}\n").is_empty());
    assert!(talk("{\"method\":42}\n").is_empty());
}

#[test]
fn the_binary_speaks_it_over_its_own_stdio() {
    let binary = env!("CARGO_BIN_EXE_sysml-mcp");

    let out = Command::new(binary).arg("--help").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Model Context Protocol"));

    let out = Command::new(binary).arg("--nonsense").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unexpected argument"));

    // a library small enough to load in a test, said both ways a client
    // may say it
    let dir = std::env::temp_dir().join("sysml-mcp-library");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("tiny.sysml"),
        "package Tiny {\n\tpart def Widget;\n}\n",
    )
    .unwrap();
    for spoken in [vec!["--library", dir.to_str().unwrap()], vec![]] {
        let mut child = Command::new(binary)
            .args(&spoken)
            .env("SYSML_LIBRARY_PATH", &dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":\
                  {\"name\":\"library_search\",\"arguments\":{\"query\":\"widget\"}}}\n",
            )
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success());
        let answer: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(answered(&answer)["found"][0]["name"], "Tiny::Widget");
    }
}

/// A model is rarely one file. `alongside` names the others so that the
/// references between them resolve; without it the server would report
/// every one of them as a name it cannot find, which is the answer an
/// agent would act on.
#[test]
fn a_model_that_spans_files_is_checked_against_the_rest_of_it() {
    let dir = std::env::temp_dir().join("sysml-mcp-alongside");
    std::fs::create_dir_all(&dir).unwrap();
    let other = dir.join("parts.sysml");
    std::fs::write(&other, "package Parts {\n\tpart def Wheel;\n}\n").unwrap();
    let text = "package Car {\n\tprivate import Parts::*;\n\tpart w : Wheel;\n}\n";

    // on its own, `Wheel` is a name nothing answers to
    let alone = answered(&session(&[call("check", json!({ "text": text }))])[0]);
    assert_eq!(alone["ok"], false, "{alone}");
    assert!(
        alone["unresolved"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["name"] == "Wheel"),
        "{alone}"
    );

    // told where the rest of the model is, it resolves
    let together = answered(
        &session(&[call(
            "check",
            json!({ "text": text, "alongside": [other.to_str().unwrap()] }),
        )])[0],
    );
    assert_eq!(together["ok"], true, "{together}");

    // and the same file is what `visible_names` can see from inside it
    let names = answered(
        &session(&[call(
            "visible_names",
            json!({
                "text": text, "line": 3, "column": 13,
                "alongside": [other.to_str().unwrap()],
            }),
        )])[0],
    );
    assert!(
        names["visible"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["name"] == "Wheel"),
        "{names}"
    );

    // a companion that is not there is named, not passed over: a check
    // that quietly dropped it would answer about a different model
    for tool in ["check", "visible_names"] {
        let missing = session(&[call(
            tool,
            json!({ "text": text, "line": 1, "column": 1, "alongside": ["/nowhere/parts.sysml"] }),
        )]);
        assert_eq!(missing[0]["result"]["isError"], true, "{missing:?}");
        assert!(
            answered(&missing[0])["error"]
                .as_str()
                .unwrap()
                .contains("cannot read"),
            "{missing:?}"
        );
    }
}

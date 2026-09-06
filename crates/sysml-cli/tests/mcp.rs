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
    let mut server = sysml_cli::mcp::Server::new(None);
    let mut out: Vec<u8> = Vec::new();
    sysml_cli::mcp::serve(&mut server, Cursor::new(input.as_bytes()), &mut out).unwrap();
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
    let mut server = sysml_cli::mcp::Server::new(Some(library));
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
    let binary = env!("CARGO_BIN_EXE_sysml");

    let out = Command::new(binary)
        .args(["mcp", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Model Context Protocol"));

    let out = Command::new(binary)
        .args(["mcp", "--nonsense"])
        .output()
        .unwrap();
    assert!(!out.status.success());
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
            .arg("mcp")
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

/// JSON-RPC lets a server fall silent only for a notification. Anything
/// else that carries an id is waited on, so an answer has to come back
/// even when there is nothing sensible to answer.
#[test]
fn a_message_that_carries_an_id_is_always_answered() {
    // an id and no method: nothing to do, and something to say
    let answers = session(&[json!({"jsonrpc": "2.0", "id": 1})]);
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0]["error"]["code"], -32600);
    assert_eq!(answers[0]["id"], 1);

    // a method that is not a name is no method at all
    let answers = session(&[json!({"jsonrpc": "2.0", "id": 2, "method": 42})]);
    assert_eq!(answers[0]["error"]["code"], -32600);
    assert_eq!(answers[0]["id"], 2);

    // MCP carries no batches, and a client that sent one is waiting
    for message in [
        "[]",
        "[{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}]",
        "7",
    ] {
        let answers = talk(&format!("{message}\n"));
        assert_eq!(answers.len(), 1, "{message}");
        assert_eq!(answers[0]["error"]["code"], -32600, "{message}");
        assert_eq!(answers[0]["id"], Value::Null, "{message}");
    }
}

/// A line of bytes that are not UTF-8 is not JSON either, and saying so
/// is the answer; ending the session would take down whatever else the
/// client had in flight.
#[test]
fn a_line_that_is_not_text_is_a_parse_error_not_the_end() {
    let mut server = sysml_cli::mcp::Server::new(None);
    let mut out: Vec<u8> = Vec::new();
    let mut input: Vec<u8> = b"\xff\xfe not text\n".to_vec();
    input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}\n");
    sysml_cli::mcp::serve(&mut server, Cursor::new(input), &mut out).unwrap();
    let answers: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(answers.len(), 2);
    assert_eq!(answers[0]["error"]["code"], -32700);
    assert_eq!(answers[1]["id"], 7, "the session went on");
}

/// A version this server cannot speak is not promised back.
#[test]
fn only_a_revision_the_server_speaks_is_echoed() {
    let answers = session(&[
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": {"protocolVersion": "2999-01-01"}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "initialize",
               "params": {"protocolVersion": "2025-03-26"}}),
    ]);
    assert_eq!(answers[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(answers[1]["result"]["protocolVersion"], "2025-03-26");
}

/// Where the library is, and whether there is one at all, decides how
/// much of an answer is worth believing -- so `check` says.
#[test]
fn check_says_which_library_it_answered_against() {
    let without = answered(&session(&[call("check", json!({ "text": "package P;\n" }))])[0]);
    assert!(without["library"].is_null(), "{without}");

    let dir = std::env::temp_dir().join("sysml-mcp-library-said");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tiny.sysml"), "package Tiny {\n\tpart def W;\n}\n").unwrap();
    let mut server = sysml_cli::mcp::Server::new(Some(&dir));
    let response = server
        .handle(&call("check", json!({ "text": "package P;\n" })))
        .expect("a request is answered");
    assert_eq!(answered(&response)["library"], dir.to_str().unwrap());

    // and one that will not load leaves the server without a library:
    // a path that is not there, and a file that will not open
    let mut nowhere = sysml_cli::mcp::Server::new(Some(std::path::Path::new("/nowhere/library")));
    let response = nowhere
        .handle(&call("check", json!({ "text": "package P;\n" })))
        .expect("a request is answered");
    assert!(answered(&response)["library"].is_null());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let shut = std::env::temp_dir().join("sysml-mcp-library-shut");
        std::fs::create_dir_all(&shut).unwrap();
        let hidden = shut.join("hidden.sysml");
        std::fs::write(&hidden, "package Hidden;\n").unwrap();
        std::fs::set_permissions(&hidden, std::fs::Permissions::from_mode(0o000)).unwrap();
        let mut server = sysml_cli::mcp::Server::new(Some(&shut));
        let response = server
            .handle(&call("check", json!({ "text": "package P;\n" })))
            .expect("a request is answered");
        assert!(answered(&response)["library"].is_null());
        std::fs::set_permissions(&hidden, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
}

/// The reference counts cover every file the check opened, so the
/// findings do too -- each under the path of the file it is in. An
/// `alongside` file whose own names resolve to nothing used to be
/// counted and then left out, so the answer read `ok` with a reference
/// missing from it.
#[test]
fn a_finding_in_a_companion_file_is_reported_under_its_own_path() {
    let dir = std::env::temp_dir().join("sysml-mcp-companion");
    std::fs::create_dir_all(&dir).unwrap();
    let other = dir.join("parts.sysml");
    std::fs::write(&other, "package Parts {\n\tpart w : Nowhere;\n}\n").unwrap();
    let text = "package Car {\n\tpart def Body;\n}\n";

    let found = answered(
        &session(&[call(
            "check",
            json!({ "text": text, "alongside": [other.to_str().unwrap()] }),
        )])[0],
    );
    assert_eq!(found["ok"], false, "{found}");
    assert_eq!(found["references"], 1, "{found}");
    assert_eq!(found["unresolved"][0]["name"], "Nowhere", "{found}");
    assert_eq!(found["unresolved"][0]["path"], other.to_str().unwrap());
    assert_eq!(found["unresolved"][0]["line"], 2, "{found}");

    // and a syntax error in one is placed in that file, not in this one
    std::fs::write(&other, "package Parts {\n\tpart w : ;\n").unwrap();
    let found = answered(
        &session(&[call(
            "check",
            json!({ "text": text, "alongside": [other.to_str().unwrap()] }),
        )])[0],
    );
    assert_eq!(found["ok"], false, "{found}");
    assert_eq!(found["parseErrors"][0]["path"], other.to_str().unwrap());
}

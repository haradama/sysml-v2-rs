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

/// Drive a server that holds a project, the way a launcher starts one.
fn in_project(dir: &std::path::Path, lines: &[Value]) -> Vec<Value> {
    let input: String = lines
        .iter()
        .map(|line| format!("{line}\n"))
        .collect::<String>();
    let mut server = sysml_cli::mcp::Server::with_project(None, Some(dir));
    let mut out: Vec<u8> = Vec::new();
    sysml_cli::mcp::serve(&mut server, Cursor::new(input.as_bytes()), &mut out).unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).expect("every answer is JSON"))
        .collect()
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
    assert_eq!(
        tools,
        [
            "check",
            "visible_names",
            "outline",
            "generate_rust",
            "import_rust",
            "library_search"
        ]
    );
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

/// A directory of model files, emptied first so a run does not read what
/// the last one left.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A model of any size is spread over files, and a call that has to
/// restate all of them is one a client gets wrong: a file left out reads
/// as a page of unresolved names, and what the client believes then is
/// that the model is broken.
#[test]
fn a_project_is_the_model_a_call_that_names_no_source_is_about() {
    let dir = scratch("sysml-mcp-project");
    std::fs::write(
        dir.join("parts.sysml"),
        "package Parts {\n\tpart def Wheel;\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("car.sysml"),
        "package Car {\n\tprivate import Parts::*;\n\tpart w : Wheel;\n}\n",
    )
    .unwrap();

    // `Wheel` is declared in one file and used in the other, so it
    // resolving at all is the proof that both were opened
    let answer = answered(&in_project(&dir, &[call("check", json!({}))])[0]);
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["references"], 1, "{answer}");
    assert_eq!(answer["project"], dir.display().to_string(), "{answer}");
}

/// The file a call brings replaces the one on disk rather than joining
/// it. Added beside it, every declaration in it would be made twice.
#[test]
fn a_file_the_call_brings_stands_in_for_the_one_on_disk() {
    let dir = scratch("sysml-mcp-overlay");
    std::fs::write(
        dir.join("parts.sysml"),
        "package Parts {\n\tpart def Wheel;\n}\n",
    )
    .unwrap();
    let car = dir.join("car.sysml");
    std::fs::write(
        &car,
        "package Car {\n\tprivate import Parts::*;\n\tpart w : Wheel;\n}\n",
    )
    .unwrap();

    let edited = "package Car {\n\tprivate import Parts::*;\n\tpart w : Wheel;\n\tpart b : NoSuchThing;\n}\n";
    let answer = answered(
        &in_project(
            &dir,
            &[call(
                "check",
                json!({ "path": car.to_str().unwrap(), "text": edited, "name": car.to_str().unwrap() }),
            )],
        )[0],
    );

    // the edit is what was checked: `Wheel` and `NoSuchThing`, and not
    // the third reference the copy on disk would have brought with it
    assert_eq!(answer["references"], 2, "{answer}");
    let unresolved = answer["unresolved"].as_array().unwrap();
    assert_eq!(unresolved.len(), 1, "{answer}");
    assert_eq!(unresolved[0]["name"], "NoSuchThing", "{answer}");
    // and the sibling on disk still answers for `Wheel`
    assert_eq!(answer["resolved"], 1, "{answer}");
}

/// The agent asking these questions is the one writing the files.
#[test]
fn the_project_is_read_again_after_it_changes() {
    let dir = scratch("sysml-mcp-refresh");
    let car = dir.join("car.sysml");
    std::fs::write(&car, "package Car {\n\tpart def Wheel;\n}\n").unwrap();
    assert_eq!(
        answered(&in_project(&dir, &[call("check", json!({}))])[0])["ok"],
        true
    );

    std::fs::write(&car, "package Car {\n\tpart w : NoSuchThing;\n}\n").unwrap();
    let answer = answered(&in_project(&dir, &[call("check", json!({}))])[0]);
    assert_eq!(answer["ok"], false, "{answer}");
}

/// A position is in a file, and a project is a great many of them.
#[test]
fn a_position_has_to_name_the_file_it_is_in() {
    let dir = scratch("sysml-mcp-position");
    std::fs::write(
        dir.join("car.sysml"),
        "package Car {\n\tpart def Wheel;\n}\n",
    )
    .unwrap();

    let refused = in_project(
        &dir,
        &[call("visible_names", json!({ "line": 1, "column": 1 }))],
    );
    assert_eq!(refused[0]["result"]["isError"], true, "{refused:?}");
    assert!(
        answered(&refused[0])["error"]
            .as_str()
            .unwrap()
            .contains("a position is in a file"),
        "{refused:?}"
    );
}

/// With no project and no source there is nothing to answer about, and
/// the refusal says both ways out rather than only the one.
#[test]
fn a_call_with_nothing_to_answer_about_says_both_ways_out() {
    let refused = session(&[call("check", json!({}))]);
    assert_eq!(refused[0]["result"]["isError"], true, "{refused:?}");
    let why = answered(&refused[0])["error"].as_str().unwrap().to_string();
    assert!(why.contains("`text` or `path`"), "{why}");
    assert!(why.contains("--project"), "{why}");
}

/// The library, where the submodule is checked out.
fn library() -> Option<&'static std::path::Path> {
    let library = std::path::Path::new("../../vendor/sysml-v2-release/sysml.library");
    library.is_dir().then_some(library)
}

/// Re-reading a model file answers a different question: what a
/// definition inherits is not written in the text.
#[test]
fn an_outline_says_what_a_definition_inherits_as_well_as_what_it_declares() {
    let text = "package P {\n\tpart def Source;\n\tpart def Car :> Source {\n\t\tpart w : Source;\n\t}\n}\n";
    let outline = answered(&session(&[call("outline", json!({ "text": text, "depth": 3 }))])[0]);
    let package = &outline["elements"][0];
    assert_eq!(package["kind"], "Package", "{outline}");
    let car = package["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|it| it["name"] == "P::Car")
        .expect("the definition is described");
    // what the text says it specializes, and where it is written
    assert_eq!(car["specializes"][0], "P::Source", "{car}");
    assert_eq!(car["line"], 3, "{car}");
    // and what is inside it, with the type resolved to its full name
    assert_eq!(car["members"][0]["name"], "P::Car::w", "{car}");
    assert_eq!(car["members"][0]["type"], "P::Source", "{car}");

    // depth is what stops it: a level past it is named, not described
    let shallow = answered(&session(&[call("outline", json!({ "text": text, "depth": 1 }))])[0]);
    assert!(shallow["elements"][0]["members"][0]["members"].is_null());

    // a directed feature says which way it goes: an action's parameters
    // are half of what its signature is
    let action = answered(
        &session(&[call(
            "outline",
            json!({
                "text": "package P {\n\tattribute def Metres;\n\taction def Go {\n\t\tin speed : Metres;\n\t}\n}\n",
                "depth": 3,
            }),
        )])[0],
    );
    let parameter = &action["elements"][0]["members"][1]["members"][0];
    assert_eq!(parameter["name"], "P::Go::speed", "{action}");
    assert_eq!(parameter["direction"], "in", "{action}");

    // and a definition's inherited features are asked for, not assumed
    let deep = answered(
        &session(&[call(
            "outline",
            json!({ "text": text, "within": "P::Car", "depth": 1, "inherited": true }),
        )])[0],
    );
    assert!(
        deep["elements"][0]["members"]
            .as_array()
            .unwrap()
            .iter()
            .any(|it| it["name"] == "P::Car::w"),
        "{deep}"
    );
}

/// A name in the standard library is something to look at before it is
/// used, and doing so needs no model of one's own.
#[test]
fn an_outline_describes_the_library_with_no_model_at_all() {
    let Some(library) = library() else { return };
    let mut server = sysml_cli::mcp::Server::new(Some(library));
    let described = answered(
        &server
            .handle(&call(
                "outline",
                json!({ "within": "Parts::Part", "depth": 1 }),
            ))
            .expect("a request is answered"),
    );
    let part = &described["elements"][0];
    assert_eq!(part["kind"], "PartDefinition", "{described}");
    assert_eq!(part["specializes"][0], "Items::Item", "{described}");

    // a name nothing answers to is refused, not guessed at
    let refused = server
        .handle(&call("outline", json!({ "within": "Parts::NoSuchThing" })))
        .expect("a request is answered");
    assert_eq!(refused["result"]["isError"], true, "{refused}");

    // and with neither a model nor a name there is nothing to describe
    let nothing = server
        .handle(&call("outline", json!({})))
        .expect("a request is answered");
    assert_eq!(nothing["result"]["isError"], true, "{nothing}");
}

/// The skeleton is the generator's to write; the gaps are not. What
/// comes back second is the work list.
#[test]
fn generate_rust_says_what_the_model_left_for_a_person() {
    let text = "package P {\n\
                \tattribute def Metres;\n\
                \tabstract calc def Fade {\n\t\tin heat : Metres;\n\t\treturn : Metres;\n\t}\n\
                \tcalc def Odd {\n\t\tin xs : Metres[*];\n\t\treturn : Metres = xs->reduce max;\n\t}\n\
                }\n";
    let answer = answered(&session(&[call("generate_rust", json!({ "text": text }))])[0]);
    assert!(answer["rust"].as_str().unwrap().contains("pub trait Fade"));

    let open = answer["open"].as_array().unwrap();
    let trait_left = open
        .iter()
        .find(|it| it["kind"] == "trait")
        .expect("the abstract definition is on the list");
    assert_eq!(trait_left["sysml"], "Fade", "{answer}");
    assert!(trait_left["rust"].as_str().unwrap().contains("Fade"));
    let todo = open
        .iter()
        .find(|it| it["kind"] == "todo")
        .expect("the untranslatable formula is on the list");
    assert_eq!(todo["sysml"], "Odd", "{answer}");
    // the model's own words, so what has to be written is stated
    assert!(todo["why"].as_str().unwrap().contains("reduce"), "{answer}");

    // a state machine's decisions are the same kind of debt, under a
    // name of their own because they come with defaults
    let machine = answered(
        &session(&[call(
            "generate_rust",
            json!({ "text": "package P {\n\tstate def Machine {\n\t\tentry; then off;\n\t\tstate off;\n\t\tstate on;\n\t\ttransition first off then on;\n\t}\n}\n" }),
        )])[0],
    );
    assert_eq!(machine["open"][0]["kind"], "hooks", "{machine}");
    assert_eq!(machine["open"][0]["rust"], "MachineHooks", "{machine}");

    // and what has no generated shape at all is named rather than left
    // to be found by reading the file for comments
    let dropped = answered(
        &session(&[call(
            "generate_rust",
            json!({ "text": "package P {\n\tpart def Car {\n\t\tport p;\n\t}\n}\n" }),
        )])[0],
    );
    assert_eq!(dropped["open"][0]["kind"], "skipped", "{dropped}");
    assert_eq!(dropped["open"][0]["sysml"], "Car", "{dropped}");
}

/// Generated code would quietly be missing whatever the model misspelt,
/// so a model that does not resolve is refused rather than written from.
#[test]
fn generate_rust_refuses_a_model_that_does_not_resolve() {
    let refused = session(&[call(
        "generate_rust",
        json!({ "text": "package P {\n\tpart def Car { attribute a : NoSuchType; }\n}\n" }),
    )]);
    assert_eq!(refused[0]["result"]["isError"], true, "{refused:?}");
    let why = answered(&refused[0])["error"].as_str().unwrap().to_string();
    assert!(why.contains("NoSuchType"), "{why}");
    assert!(why.contains("check"), "{why}");
}

/// The mapping from a Rust API to SysML is not obvious, and a model that
/// gets it wrong says so only when the generated code will not compile.
#[test]
fn import_rust_states_a_crate_and_says_what_it_could_not() {
    let fixture = std::path::Path::new("../sysml-rust/tests/fixtures/inventory_store.rustdoc.json");
    let Ok(json) = std::fs::read_to_string(fixture) else {
        return; // the fixture belongs to the crate that owns the importer
    };

    // from a path, against the library, so the answer can say whether
    // every name in what it wrote resolves
    if let Some(library) = library() {
        let mut server = sysml_cli::mcp::Server::new(Some(library));
        let imported = answered(
            &server
                .handle(&call(
                    "import_rust",
                    json!({ "rustdoc": fixture.to_str().unwrap(), "package": "Warehouse" }),
                ))
                .expect("a request is answered"),
        );
        assert!(imported["sysml"]
            .as_str()
            .unwrap()
            .contains("package Warehouse"));
        assert_eq!(imported["checked"]["ok"], true, "{}", imported["checked"]);
        // and what had no SysML shape is named rather than missing
        assert!(!imported["skipped"].as_array().unwrap().is_empty());
    }

    // or written out in the call
    let inline = answered(&session(&[call("import_rust", json!({ "json": json }))])[0]);
    assert!(inline["sysml"]
        .as_str()
        .unwrap()
        .contains("metadata def rust"));

    // with neither, the refusal says what writes the file
    let refused = session(&[call("import_rust", json!({}))]);
    assert_eq!(refused[0]["result"]["isError"], true, "{refused:?}");
    assert!(
        answered(&refused[0])["error"]
            .as_str()
            .unwrap()
            .contains("cargo +nightly rustdoc"),
        "{refused:?}"
    );

    // and a file that is not rustdoc's is refused by the importer
    for bad in [
        json!({ "json": "not json" }),
        json!({ "rustdoc": "/nowhere.json" }),
    ] {
        let refused = session(&[call("import_rust", bad)]);
        assert_eq!(refused[0]["result"]["isError"], true, "{refused:?}");
    }
}

/// Modelling a system of any size means asking whether the concept is
/// already there, and the answer is not in the library.
#[test]
fn a_search_can_be_asked_of_the_model_as_well_as_the_library() {
    let text = "package P {\n\tpart def PowerSource;\n}\n";

    let mine = answered(
        &session(&[call(
            "library_search",
            json!({ "query": "powersource", "scope": "model", "text": text }),
        )])[0],
    );
    assert_eq!(mine["found"][0]["name"], "P::PowerSource", "{mine}");
    assert_eq!(mine["found"][0]["kind"], "PartDefinition", "{mine}");

    // both scopes answer from one call, and the library half of it is
    // empty here because nothing in it is called that
    let both = answered(
        &session(&[call(
            "library_search",
            json!({ "query": "powersource", "scope": "both", "text": text }),
        )])[0],
    );
    assert_eq!(both["found"].as_array().unwrap().len(), 1, "{both}");

    // asking about a model without giving one is refused
    let refused = session(&[call(
        "library_search",
        json!({ "query": "powersource", "scope": "model" }),
    )]);
    assert_eq!(refused[0]["result"]["isError"], true, "{refused:?}");
}

/// One file of a project that cannot be read is not the answer being
/// about a different model without saying so.
#[test]
fn a_project_file_that_cannot_be_read_is_left_out_and_said_so() {
    let dir = scratch("sysml-mcp-unreadable");
    std::fs::write(
        dir.join("car.sysml"),
        "package Car {\n\tpart def Wheel;\n}\n",
    )
    .unwrap();
    let locked = dir.join("locked.sysml");
    std::fs::write(&locked, "package Locked {}\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    }
    if std::fs::read_to_string(&locked).is_ok() {
        return; // running as root, and there is nothing to prove
    }

    // the rest of the project is still the model
    let answer = answered(&in_project(&dir, &[call("check", json!({}))])[0]);
    assert_eq!(answer["ok"], true, "{answer}");
}

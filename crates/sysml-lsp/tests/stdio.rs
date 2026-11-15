//! Drives the `sysml-lsp` binary over real stdio with LSP framing.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn frame(body: &str) -> String {
    format!("Content-Length: {}\r\n\r\n{body}", body.len())
}

fn read_message(reader: &mut impl BufRead) -> String {
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length: ") {
            length = value.parse().unwrap();
        }
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).unwrap();
    String::from_utf8(body).unwrap()
}

#[test]
fn binary_speaks_lsp_over_stdio() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sysml-lsp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    stdin
        .write_all(
            frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#)
                .as_bytes(),
        )
        .unwrap();
    let response = read_message(&mut stdout);
    assert!(response.contains("capabilities"), "{response}");

    stdin
        .write_all(frame(r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#).as_bytes())
        .unwrap();
    stdin
        .write_all(
            frame(r#"{"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}"#).as_bytes(),
        )
        .unwrap();
    let response = read_message(&mut stdout);
    assert!(response.contains("\"id\":2"), "{response}");
    stdin
        .write_all(frame(r#"{"jsonrpc":"2.0","method":"exit","params":null}"#).as_bytes())
        .unwrap();
    drop(stdin);

    let status = child.wait().unwrap();
    assert!(status.success());
}

/// A message the server cannot read is answered, and the conversation
/// goes on: `lsp_server`'s own transport ended the process at the first
/// one, with no answer to anything.
#[test]
fn a_message_that_cannot_be_read_is_answered_and_the_server_goes_on() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sysml-lsp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut send = |body: &str| stdin.write_all(frame(body).as_bytes()).unwrap();

    send(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"initializationOptions":{"noLibrary":true}}}"#,
    );
    assert!(read_message(&mut stdout).contains("capabilities"));
    send(r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#);

    send("{not json");
    let answer = read_message(&mut stdout);
    assert!(
        answer.contains("-32700") && answer.contains("\"id\":null"),
        "{answer}"
    );
    // an id the protocol's types cannot hold is still echoed
    send(r#"{"jsonrpc":"2.0","id":2.5,"method":"shutdown"}"#);
    let answer = read_message(&mut stdout);
    assert!(
        answer.contains("-32600") && answer.contains("\"id\":2.5"),
        "{answer}"
    );
    // half a surrogate pair, as `JSON.stringify` writes it, is read
    send(
        r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"untitled:a","languageId":"sysml","version":1,"text":"// \ud83d\npart def B;\npart a : B;\n"}}}"#,
    );
    let published = read_message(&mut stdout);
    assert!(
        published.contains("publishDiagnostics") && published.contains("\"diagnostics\":[]"),
        "{published}"
    );

    send(r#"{"jsonrpc":"2.0","id":3,"method":"shutdown","params":null}"#);
    assert!(read_message(&mut stdout).contains("\"id\":3"));
    send(r#"{"jsonrpc":"2.0","method":"exit","params":null}"#);
    drop(stdin);
    assert!(child.wait().unwrap().success());
}

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

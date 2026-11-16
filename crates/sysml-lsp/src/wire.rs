//! Reading messages off the wire, and what to say about one that cannot
//! be read.
//!
//! `lsp_server`'s stdio transport ends the conversation at the first
//! message it cannot parse: its reader thread returns the error, the
//! channel closes, and the server exits with no answer to anything. One
//! bad message is not a reason to lose every name in every open file,
//! and the commonest bad message is not even the client's fault -- a
//! document holding half of a UTF-16 surrogate pair, which JavaScript's
//! `JSON.stringify` writes as `"\ud83d"` and which no Rust string can
//! hold. So this reads the frames itself, mends what can be mended, and
//! answers what cannot with the JSON-RPC error the specification gives
//! for it.

use std::borrow::Cow;
use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex};
use std::thread;

use lsp_server::{Connection, Message};

/// A message as it came, made into one the server can read where it
/// can be.
///
/// `Err` holds the answer to give instead: a JSON-RPC error response,
/// carrying the request's id where one can be found in it.
pub fn read(body: &[u8]) -> Result<Message, serde_json::Value> {
    // Bytes that are not UTF-8 are one character each that no client
    // meant; the rest of the message is still what it said.
    let text = String::from_utf8_lossy(body);
    let text = mended(&text);
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(why) => return Err(unreadable(&text, &why.to_string())),
    };
    // A request whose id is neither a whole number nor a string reads,
    // to `lsp_server`, as a notification -- and `{"id": 2.5, "method":
    // "shutdown"}` would shut the server down without a word to the
    // client waiting on it. Whatever carries an id is a request or a
    // response, or nothing the server can read.
    let has_id = value.get("id").is_some();
    // and serde reads a struct out of an array as readily as out of an
    // object, so `[1, 2]` -- a batch, which the protocol does not have
    // -- came out as a response to request 1
    if !value.is_object() {
        return Err(unreadable(&text, "a message is a JSON object"));
    }
    match serde_json::from_value::<Message>(value) {
        Ok(Message::Notification(_)) if has_id => {
            Err(unreadable(&text, "an id is a whole number or a string"))
        }
        Ok(message) => Ok(message),
        Err(why) => Err(unreadable(&text, &why.to_string())),
    }
}

/// The answer to a message that is not one the server can read.
///
/// JSON that does not parse is a parse error (`-32700`), and JSON that
/// is not a message -- `{}`, a batch, an id that is neither a whole
/// number nor a string -- an invalid request (`-32600`). Either way the
/// id is echoed where there is one to echo, so that a client waiting on
/// that request hears that it failed rather than waiting for ever.
pub fn unreadable(text: &str, why: &str) -> serde_json::Value {
    let (code, id) = match serde_json::from_str::<serde_json::Value>(text) {
        Err(_) => (-32700, serde_json::Value::Null),
        Ok(value) => (
            -32600,
            value
                .get("id")
                .cloned()
                .filter(|id| !id.is_object() && !id.is_array())
                .unwrap_or(serde_json::Value::Null),
        ),
    };
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": format!("sysml-lsp could not read the message: {why}") },
    })
}

/// JSON text with each lone UTF-16 surrogate escape replaced by U+FFFD.
///
/// `😀` is one character written as a pair, and kept. A half
/// of a pair on its own names no character, and serde_json refuses the
/// whole message for it; U+FFFD is the character Unicode keeps for one
/// that is not there. It is one UTF-16 unit, as the lone surrogate was,
/// so every position the client counts after it is still right.
pub fn mended(text: &str) -> Cow<'_, str> {
    if !text.contains("\\u") {
        return Cow::Borrowed(text);
    }
    let bytes = text.as_bytes();
    let unit = |at: usize| -> Option<u16> {
        if bytes.get(at) != Some(&b'\\') || bytes.get(at + 1) != Some(&b'u') {
            return None;
        }
        let hex = text.get(at + 2..at + 6)?;
        u16::from_str_radix(hex, 16).ok()
    };
    let high = |u: u16| (0xD800..0xDC00).contains(&u);
    let low = |u: u16| (0xDC00..0xE000).contains(&u);
    let mut out = String::with_capacity(text.len());
    let mut changed = false;
    let mut at = 0;
    let mut copied = 0;
    while at < bytes.len() {
        if bytes[at] != b'\\' {
            at += 1;
            continue;
        }
        let Some(u) = unit(at) else {
            // any other escape is two characters, and the second may be
            // a backslash that starts nothing: `"\\ud83d"` is text
            at += 2;
            continue;
        };
        if high(u) && unit(at + 6).is_some_and(low) {
            at += 12;
            continue;
        }
        if high(u) || low(u) {
            out.push_str(&text[copied..at]);
            out.push_str("\\ufffd");
            changed = true;
            copied = at + 6;
        }
        at += 6;
    }
    if !changed {
        return Cow::Borrowed(text);
    }
    out.push_str(&text[copied..]);
    Cow::Owned(out)
}

/// One frame's body, or `None` at the end of the input.
///
/// A header that cannot be read leaves no way to find where the next
/// message begins, so that alone is an error.
fn frame(input: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut length = None;
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.trim().eq_ignore_ascii_case("Content-Length") {
                length = value.trim().parse::<usize>().ok();
            }
        }
    }
    let length = length.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "a frame with no Content-Length")
    })?;
    let mut body = vec![0; length];
    input.read_exact(&mut body)?;
    Ok(Some(body))
}

fn write_frame(output: &Mutex<io::Stdout>, text: &str) -> io::Result<()> {
    // a writer that panicked left the lock poisoned, and what it held is
    // still a stdout
    let mut output = output.lock().unwrap_or_else(|it| it.into_inner());
    write!(output, "Content-Length: {}\r\n\r\n{text}", text.len())?;
    output.flush()
}

/// The threads behind [`stdio`].
pub struct Threads {
    writer: thread::JoinHandle<io::Result<()>>,
}

impl Threads {
    /// Wait for every answer to be written.
    ///
    /// Only the writer is waited for: it ends once the server's side of
    /// the connection is dropped, while the reader may sit on an input
    /// the client never closes, and the process ending takes it along.
    pub fn join(self) -> io::Result<()> {
        self.writer
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("the thread writing answers panicked")))
    }
}

/// A connection over standard input and output, as
/// `lsp_server::Connection::stdio` makes, that outlives a message it
/// cannot read.
pub fn stdio() -> (Connection, Threads) {
    let (server, client) = Connection::memory();
    let output = Arc::new(Mutex::new(io::stdout()));
    let Connection { sender, receiver } = client;
    let to_client = Arc::clone(&output);
    let writer = thread::Builder::new()
        .name("sysml-lsp writer".into())
        .spawn(move || {
            for message in receiver {
                let mut value = serde_json::to_value(&message)?;
                if let Some(fields) = value.as_object_mut() {
                    fields.insert("jsonrpc".into(), "2.0".into());
                }
                write_frame(&to_client, &value.to_string())?;
            }
            Ok(())
        })
        .expect("a thread to write answers");
    thread::Builder::new()
        .name("sysml-lsp reader".into())
        .spawn(move || -> io::Result<()> {
            let stdin = io::stdin();
            let mut input = stdin.lock();
            while let Some(body) = frame(&mut input)? {
                match read(&body) {
                    Ok(message) => {
                        let exit =
                            matches!(&message, Message::Notification(it) if it.method == "exit");
                        if sender.send(message).is_err() || exit {
                            break;
                        }
                    }
                    Err(answer) => write_frame(&output, &answer.to_string())?,
                }
            }
            Ok(())
        })
        .expect("a thread to read messages");
    (server, Threads { writer })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lone_surrogate_is_the_character_that_is_not_there() {
        assert_eq!(mended(r#""a\ud83db""#), r#""a\ufffdb""#);
        assert_eq!(mended(r#""\ude00""#), r#""\ufffd""#);
        // a pair is one character and stays as it is
        assert_eq!(mended(r#""😀""#), r#""😀""#);
        // a high half followed by another high half is two lone ones
        assert_eq!(mended(r#""\ud83d\ud83d""#), r#""\ufffd\ufffd""#);
        // an escaped backslash before `u` is text, not an escape
        assert_eq!(mended(r#""\\ud83d""#), r#""\\ud83d""#);
        assert_eq!(mended(r#""A""#), r#""A""#);
    }

    #[test]
    fn a_message_with_half_a_character_in_it_is_still_read() {
        let body = br#"{"jsonrpc":"2.0","method":"textDocument/didChange","params":{"text":"// \ud83d\n"}}"#;
        assert!(
            matches!(
                read(body),
                Ok(Message::Notification(note)) if note.params["text"] == "// \u{fffd}\n"
            ),
            "the change is read, with the half a character mended"
        );
    }

    #[test]
    fn what_cannot_be_read_is_answered_with_the_error_for_it() {
        let answer = |body: &[u8]| read(body).unwrap_err();
        let parse = answer(b"{not json");
        assert_eq!(parse["error"]["code"], -32700);
        assert!(parse["id"].is_null());
        // a request whose id is no id the server can answer by is still
        // answered by it, so that its client stops waiting
        let float = answer(br#"{"jsonrpc":"2.0","id":1.5,"method":"shutdown"}"#);
        assert_eq!(float["error"]["code"], -32600);
        assert_eq!(float["id"], 1.5);
        assert_eq!(answer(b"{}")["error"]["code"], -32600);
        assert_eq!(answer(b"[1, 2]")["error"]["code"], -32600);
        // and bytes that are not UTF-8 cost only themselves
        let mut body = br#"{"jsonrpc":"2.0","method":"x","params":{"t":""#.to_vec();
        body.push(0xff);
        body.extend_from_slice(br#""}}"#);
        assert!(read(&body).is_ok());
    }
}

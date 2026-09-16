//! The SysML v2 language server, as a WebAssembly module.
//!
//! `sysmlv2-lsp` is the whole server; this is the fifty lines that let a
//! browser drive it. The VSCode extension loads the module in a worker,
//! and every message of the Language Server Protocol goes in and comes
//! back out through [`handle`].
//!
//! # The surface
//!
//! Four exports and one buffer. A host writes a message into the buffer
//! and calls [`handle`], which leaves the answers -- a JSON array, since
//! one message may be answered by several -- in the same buffer and says
//! how long they are.
//!
//! ```js
//! const bytes = new TextEncoder().encode(JSON.stringify(message));
//! new Uint8Array(wasm.memory.buffer, wasm.take(bytes.length), bytes.length).set(bytes);
//! const length = wasm.handle(bytes.length);
//! const answers = JSON.parse(
//!   new TextDecoder().decode(new Uint8Array(wasm.memory.buffer, wasm.answers(), length))
//! );
//! ```
//!
//! There is no binding generator: what crosses is one string each way,
//! and generating that would mean a tool pinned to a crate's version on
//! every machine that builds the extension. `rustup target add
//! wasm32-unknown-unknown` and `cargo build` are the whole toolchain.
//!
//! Having no filesystem and no threads is the server's own arrangement
//! -- `sysmlv2_lsp::Files` and `Session` -- and is documented there.

use std::cell::RefCell;

use lsp_server::Message;
use sysml_lsp::{Files, Flow, Session};

thread_local! {
    /// The server, which loads and resolves the standard library the
    /// first time a client says `initialize` and holds it from then on.
    static SERVER: RefCell<Session> = RefCell::new(Session::new(Files::handed()));
    /// The one buffer, which carries a message in and the answers out.
    static BUFFER: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// What the last panic said, for a host that has caught the trap one
    /// leaves behind.
    static TRAP: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Take one message and hand back the answers, as a JSON array.
pub fn handle(message: &[u8]) -> Vec<u8> {
    let Ok(message) = serde_json::from_slice::<Message>(message) else {
        // a host with a bug of its own, and stopping here would cost the
        // editor every name in the file and say nothing about why
        return said(&[]);
    };
    let mut out = Vec::new();
    let flow = SERVER.with(|server| server.borrow_mut().handle(message, &mut out));
    if let Flow::Over(Err(why)) = flow {
        // the session ends either way; this is where a client hears why
        out.push(Message::Notification(lsp_server::Notification {
            method: "window/logMessage".into(),
            params: serde_json::json!({ "type": 1, "message": format!("sysml-lsp: {why}") }),
        }));
    }
    said(&out)
}

/// Whether the conversation is over, so that a host can let the worker go.
pub fn over() -> bool {
    SERVER.with(|server| server.borrow().over())
}

/// The messages as a client reads them: JSON-RPC, which the protocol
/// says carries its version, and an array, since one message in is
/// however many messages out.
fn said(messages: &[Message]) -> Vec<u8> {
    let wire: Vec<serde_json::Value> = messages
        .iter()
        .map(|message| {
            let mut value = serde_json::to_value(message).unwrap_or(serde_json::Value::Null);
            if let Some(fields) = value.as_object_mut() {
                fields.insert("jsonrpc".into(), "2.0".into());
            }
            value
        })
        .collect();
    serde_json::to_vec(&wire).unwrap_or_else(|_| b"[]".to_vec())
}

/// Remember what a panic said, since standard error goes nowhere here.
///
/// A panic reaches the host as a trap with nothing in it --
/// `RuntimeError: unreachable executed`, no line and no message. The
/// hook runs before the trap, so this is the one moment the message
/// exists; `exports::trap` is how the host asks for it afterwards.
fn remember_traps() {
    // once, not on every message: setting a hook allocates one and drops
    // the last, and a host calls this as often as somebody types
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|panic| {
            let said = panic.to_string();
            TRAP.with(|trap| *trap.borrow_mut() = Some(said));
        }));
    });
}

/// What a browser calls.
///
/// `#[no_mangle]` is an unsafe attribute -- it promises the symbol is
/// unique across the whole program, and nothing checks that -- and a
/// `cdylib` has no other way to name what it exports. The exception is
/// on this module rather than on the crate so that it covers what needs
/// it and nothing else.
#[allow(unsafe_code)]
mod exports {
    use super::{BUFFER, TRAP};

    /// Make room for `length` bytes and say where to write them.
    ///
    /// The pointer is good until the next call to anything here, which
    /// may move it.
    #[no_mangle]
    pub extern "C" fn take(length: usize) -> *mut u8 {
        // the first thing any host does
        super::remember_traps();
        BUFFER.with(|buffer| {
            let mut buffer = buffer.borrow_mut();
            *buffer = vec![0; length];
            buffer.as_mut_ptr()
        })
    }

    /// Answer the `length` bytes of message now in the buffer, leaving
    /// the answers there, and say how long they are.
    #[no_mangle]
    pub extern "C" fn handle(length: usize) -> usize {
        let message = BUFFER.with(|buffer| {
            let buffer = buffer.borrow();
            buffer[..length.min(buffer.len())].to_vec()
        });
        let answers = super::handle(&message);
        let length = answers.len();
        BUFFER.with(|buffer| *buffer.borrow_mut() = answers);
        length
    }

    /// Where the answers begin.
    #[no_mangle]
    pub extern "C" fn answers() -> *const u8 {
        BUFFER.with(|buffer| buffer.borrow().as_ptr())
    }

    /// Whether the session is finished with.
    #[no_mangle]
    pub extern "C" fn finished() -> usize {
        usize::from(super::over())
    }

    /// What the last panic said, left in the buffer; the return is how
    /// long it is, and zero is nothing to say.
    #[no_mangle]
    pub extern "C" fn trap() -> usize {
        let said = TRAP.with(|trap| trap.borrow_mut().take());
        let said = said.unwrap_or_default().into_bytes();
        let length = said.len();
        BUFFER.with(|buffer| *buffer.borrow_mut() = said);
        length
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(message: serde_json::Value) -> Vec<serde_json::Value> {
        let answers = handle(&serde_json::to_vec(&message).expect("a message"));
        serde_json::from_slice(&answers).expect("an array of messages")
    }

    /// What a host does, in the order it does it. Without the standard
    /// library: this is about the plumbing, and loading one costs half a
    /// second under the instrumentation coverage runs with.
    #[test]
    fn a_conversation_through_the_buffer() {
        let answers = ask(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "capabilities": {},
                "initializationOptions": {
                    "noLibrary": true,
                    "files": [{ "uri": "file:///m/lib.sysml", "text": "part def Wheel;\n" }],
                },
                "workspaceFolders": [{ "uri": "file:///m", "name": "m" }],
            },
        }));
        assert_eq!(answers.len(), 1, "{answers:?}");
        assert_eq!(answers[0]["jsonrpc"], "2.0");
        assert!(answers[0]["result"]["capabilities"]["renameProvider"].as_bool() == Some(true));

        assert!(ask(
            serde_json::json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} })
        )
        .is_empty());

        // a document that names what the handed-over file declares: it
        // resolves, so nothing is said to be wrong with it
        let answers = ask(serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": {
                "uri": "file:///m/car.sysml",
                "languageId": "sysml",
                "version": 1,
                "text": "part def Car { part w : Wheel[4]; }\n",
            }},
        }));
        let published = answers
            .iter()
            .find(|message| message["method"] == "textDocument/publishDiagnostics")
            .expect("diagnostics for the document that was opened");
        assert_eq!(
            published["params"]["diagnostics"].as_array().map(Vec::len),
            Some(0),
            "{published}"
        );
        assert!(!over());

        assert!(
            !ask(serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" })).is_empty()
        );
        ask(serde_json::json!({ "jsonrpc": "2.0", "method": "exit" }));
        assert!(over());
    }

    /// A host with a bug of its own is not a reason to take the editor's
    /// language support away.
    #[test]
    fn nonsense_is_not_fatal() {
        assert!(handle(b"not a message") == b"[]");
    }
}

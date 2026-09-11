//! The client the end-to-end tests drive the server with.
//!
//! Six of these had grown, one per test file, and they had drifted: the
//! same request was `request` in three and `ask` in two, returning a value
//! in one and a `Result` in the other; one waited ten seconds for an
//! answer and the rest thirty; `diagnostics` meant three different things.
//! None of that was a difference between the tests, and reading one meant
//! first working out which client it had.
//!
//! So there is one client, and the two shapes of each question keep
//! separate names: `ask` hands back the server's error, `request` asserts
//! there is none.

// Each test binary compiles its own copy of this module and drives the
// server its own way, so what one of them does not call is dead code in
// that binary and live in the next. Every one of these is called by
// some test in the directory.
#![allow(dead_code)]

use std::collections::HashMap;

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use serde_json::{json, Value};

/// How long a test waits on the server before calling it hung. Long
/// enough that a corpus-sized project loading under coverage
/// instrumentation is not mistaken for a deadlock.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(30);

pub struct Client {
    connection: Connection,
    next_id: i32,
}

/// A server on a thread of its own, and the client wired to it.
///
/// The tests that watch how the server *stops* spawn their own thread
/// instead, because they read what `run` returned.
pub fn serving() -> (Client, std::thread::JoinHandle<()>) {
    let (server_side, client_side) = Connection::memory();
    let handle = std::thread::spawn(move || sysml_lsp::run(&server_side).unwrap());
    (Client::new(client_side), handle)
}

impl Client {
    pub fn new(connection: Connection) -> Client {
        Client {
            connection,
            next_id: 1,
        }
    }

    /// Send a request and hand back what came of it, the server's error
    /// included -- which is the answer, for a test about bad input.
    pub fn ask(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.send(method, params)?;
        loop {
            let got = self
                .connection
                .receiver
                .recv_timeout(PATIENCE)
                .map_err(|_| format!("no answer to {method}"))?;
            if let Message::Response(Response {
                id: at,
                result,
                error,
            }) = got
            {
                if at == id {
                    return match error {
                        Some(err) => Err(err.message),
                        None => Ok(result.unwrap_or(Value::Null)),
                    };
                }
            }
        }
    }

    /// The same, where an error response is the test failing.
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let resp = self.send_request(method, params);
        assert!(resp.error.is_none(), "error response: {:?}", resp.error);
        resp.result.unwrap_or(Value::Null)
    }

    /// The whole response, for a test that is about the error in it.
    pub fn send_request(&mut self, method: &str, params: Value) -> Response {
        let id = self.send(method, params).expect("the server is listening");
        loop {
            match self.recv() {
                Message::Response(resp) if resp.id == id => return resp,
                _ => continue,
            }
        }
    }

    fn send(&mut self, method: &str, params: Value) -> Result<RequestId, String> {
        let id = RequestId::from(self.next_id);
        self.next_id += 1;
        self.connection
            .sender
            .send(Message::Request(Request {
                id: id.clone(),
                method: method.into(),
                params,
            }))
            .map_err(|err| err.to_string())?;
        Ok(id)
    }

    /// A message of any shape, for a test about what the server does
    /// with one it never asked for.
    pub fn send_raw(&mut self, message: Message) {
        self.connection.sender.send(message).unwrap();
    }

    pub fn notify(&mut self, method: &str, params: Value) {
        self.connection
            .sender
            .send(Message::Notification(Notification {
                method: method.into(),
                params,
            }))
            .unwrap();
    }

    pub fn recv(&mut self) -> Message {
        self.connection
            .receiver
            .recv_timeout(PATIENCE)
            .expect("server did not answer")
    }

    /// The next publication, whatever file it is about.
    pub fn wait_diagnostics(&mut self) -> Value {
        loop {
            if let Message::Notification(n) = self.recv() {
                if n.method == lsp_types::notification::PublishDiagnostics::METHOD {
                    return n.params;
                }
            }
        }
    }

    /// The next publication about `uri`, discarding the ones about other
    /// files on the way.
    pub fn diagnostics_for(&mut self, uri: &str) -> Value {
        loop {
            if let Message::Notification(n) = self.recv() {
                if n.method == lsp_types::notification::PublishDiagnostics::METHOD
                    && n.params["uri"] == uri
                {
                    return n.params;
                }
            }
        }
    }

    /// The next `count` publications, by the file they are about.
    ///
    /// One arrives per open document per change, and in no particular
    /// order, so taking them one at a time and discarding what does not
    /// match throws away the answer to the next question.
    pub fn diagnostics(&mut self, count: usize) -> HashMap<String, Vec<Value>> {
        let mut out = HashMap::new();
        while out.len() < count {
            if let Message::Notification(n) = self.recv() {
                if n.method == lsp_types::notification::PublishDiagnostics::METHOD {
                    let uri = n.params["uri"].as_str().unwrap().to_string();
                    let found = n.params["diagnostics"].as_array().unwrap().clone();
                    out.insert(uri, found);
                }
            }
        }
        out
    }

    /// Just the messages of a file's diagnostics, which is what a test
    /// about what the server noticed is asserting on.
    pub fn messages(found: &[Value]) -> Vec<&str> {
        found
            .iter()
            .map(|it| it["message"].as_str().unwrap())
            .collect()
    }

    /// Shut down the way a client does, and wait for the server to go.
    pub fn stop(mut self, handle: std::thread::JoinHandle<()>) {
        self.request(lsp_types::request::Shutdown::METHOD, Value::Null);
        self.notify(lsp_types::notification::Exit::METHOD, Value::Null);
        handle.join().unwrap();
    }

    /// Initialize with no capabilities, which is what a test that is not
    /// about capabilities sends -- and with no standard library.
    ///
    /// Loading and resolving a library is the whole of what starting the
    /// server costs, and a test about hover, rename or a malformed
    /// notification is not asking about a library name. The tests that
    /// are about the library hand `initialize` its own options.
    pub fn initialize(&mut self) {
        self.initialize_with(json!({ "noLibrary": true }));
    }

    pub fn initialize_with(&mut self, options: Value) {
        self.request(
            lsp_types::request::Initialize::METHOD,
            json!({ "capabilities": {}, "initializationOptions": options }),
        );
        self.notify(lsp_types::notification::Initialized::METHOD, json!({}));
    }
}

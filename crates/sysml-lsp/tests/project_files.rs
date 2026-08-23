//! A model is written across files that import each other, and only one
//! of them is in front of you at a time. The server has to see the rest
//! of the project on disk, or every import into a sibling file reads as
//! an unresolved reference in a file that is perfectly correct.

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use serde_json::{json, Value};

struct Client {
    connection: Connection,
    next_id: i32,
}

impl Client {
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = RequestId::from(self.next_id);
        self.next_id += 1;
        self.connection
            .sender
            .send(Message::Request(Request {
                id: id.clone(),
                method: method.into(),
                params,
            }))
            .unwrap();
        loop {
            match self.recv() {
                Message::Response(Response {
                    id: got,
                    result,
                    error,
                }) if got == id => {
                    assert!(error.is_none(), "error response: {error:?}");
                    return result.unwrap_or(Value::Null);
                }
                _ => continue,
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.connection
            .sender
            .send(Message::Notification(Notification {
                method: method.into(),
                params,
            }))
            .unwrap();
    }

    fn recv(&mut self) -> Message {
        self.connection
            .receiver
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("server did not answer")
    }

    /// The next `count` publications, by the file they are about.
    ///
    /// One arrives per open document per change, and in no particular
    /// order, so taking them one at a time and discarding what does not
    /// match throws away the answer to the next question.
    fn diagnostics(&mut self, count: usize) -> std::collections::HashMap<String, Vec<Value>> {
        let mut out = std::collections::HashMap::new();
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

    fn messages(found: &[Value]) -> Vec<&str> {
        found
            .iter()
            .map(|d| d["message"].as_str().unwrap())
            .collect()
    }
}

#[test]
fn a_sibling_file_resolves_without_being_open() {
    let root = std::env::temp_dir().join("sysml-lsp-project-test");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("model")).unwrap();
    std::fs::create_dir_all(root.join("lib")).unwrap();
    // the server answers with canonical paths, so the test has to ask
    // with them too -- a temp directory is a symlink on some systems
    let root = std::fs::canonicalize(&root).unwrap();
    // a library, so the exclusion of the library directory is exercised
    // even though it sits inside the workspace folder
    std::fs::write(
        root.join("lib/Base.sysml"),
        "package Base {\n    part def Anything;\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("model/hardware.sysml"),
        "package Hardware {\n    private import Base::*;\n    part def Board :> Anything;\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("model/app.sysml"),
        "package App {\n    private import Hardware::*;\n    part uno : Board;\n}\n",
    )
    .unwrap();

    let (server_side, client_side) = Connection::memory();
    let handle = std::thread::spawn(move || sysml_lsp::run(&server_side).unwrap());
    let mut client = Client {
        connection: client_side,
        next_id: 1,
    };
    let root_uri = lsp_types::Url::from_file_path(&root).unwrap();
    client.request(
        lsp_types::request::Initialize::METHOD,
        json!({
            "capabilities": {},
            "workspaceFolders": [{ "uri": root_uri, "name": "project" }],
            "initializationOptions": { "libraryPath": root.join("lib").to_str().unwrap() }
        }),
    );
    client.notify(lsp_types::notification::Initialized::METHOD, json!({}));

    // only `app.sysml` is open; `Board` lives in a file nobody opened
    let app = lsp_types::Url::from_file_path(root.join("model/app.sysml")).unwrap();
    let text = std::fs::read_to_string(root.join("model/app.sysml")).unwrap();
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": { "uri": app, "languageId": "sysml", "version": 1, "text": text } }),
    );
    let found = client.diagnostics(1);
    assert!(found[app.as_str()].is_empty(), "{found:?}");

    // and it can be jumped to, in the file on disk
    let definition = client.request(
        lsp_types::request::GotoDefinition::METHOD,
        json!({
            "textDocument": { "uri": app },
            "position": { "line": 2, "character": 16 }
        }),
    );
    assert!(
        definition["uri"]
            .as_str()
            .unwrap()
            .ends_with("hardware.sysml"),
        "{definition}"
    );

    // a rename reaches the declaration in a file nobody opened: refusing
    // every rename that crosses a closed file would refuse nearly all of
    // them, and doing it in the open files alone would break the model
    let hardware = lsp_types::Url::from_file_path(root.join("model/hardware.sysml")).unwrap();
    let rename = client.request(
        lsp_types::request::Rename::METHOD,
        json!({
            "textDocument": { "uri": app },
            "position": { "line": 2, "character": 16 },
            "newName": "Panel"
        }),
    );
    let changes = rename["changes"].as_object().unwrap();
    assert_eq!(changes.len(), 2, "{rename}");
    assert_eq!(changes[app.as_str()].as_array().unwrap().len(), 1);
    assert_eq!(changes[hardware.as_str()].as_array().unwrap().len(), 1);

    // opening the sibling must not declare it twice: the file is read
    // from its buffer now instead of from disk
    let hardware_text = std::fs::read_to_string(root.join("model/hardware.sysml")).unwrap();
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": { "uri": hardware, "languageId": "sysml", "version": 1, "text": hardware_text } }),
    );
    let found = client.diagnostics(2);
    assert!(found[app.as_str()].is_empty(), "{found:?}");
    assert!(found[hardware.as_str()].is_empty(), "{found:?}");

    // an edit in the open sibling reaches the file that imports it
    client.notify(
        lsp_types::notification::DidChangeTextDocument::METHOD,
        json!({
            "textDocument": { "uri": hardware, "version": 2 },
            "contentChanges": [{ "text":
                "package Hardware {\n    private import Base::*;\n    part def Panel :> Anything;\n}\n" }]
        }),
    );
    let found = client.diagnostics(2);
    assert_eq!(
        Client::messages(&found[app.as_str()]),
        ["unresolved reference `Board`"]
    );

    // closing it puts the file back the way it is on disk
    client.notify(
        lsp_types::notification::DidCloseTextDocument::METHOD,
        json!({ "textDocument": { "uri": hardware } }),
    );
    client.notify(
        lsp_types::notification::DidChangeTextDocument::METHOD,
        json!({
            "textDocument": { "uri": app, "version": 2 },
            "contentChanges": [{ "text": "package App {\n    private import Hardware::*;\n    part uno : Board;\n}\n" }]
        }),
    );
    let found = client.diagnostics(1);
    assert!(found[app.as_str()].is_empty(), "{found:?}");

    client.request(lsp_types::request::Shutdown::METHOD, Value::Null);
    client.notify(lsp_types::notification::Exit::METHOD, json!({}));
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn excluded_directories_are_not_the_project() {
    let root = std::env::temp_dir().join("sysml-lsp-exclude-test");
    let _ = std::fs::remove_dir_all(&root);
    for dir in ["model", "vendor", "elsewhere"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    let root = std::fs::canonicalize(&root).unwrap();
    std::fs::write(
        root.join("vendor/Vendored.sysml"),
        "package Vendored {\n    part def Borrowed;\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("elsewhere/Other.sysml"),
        "package Other {\n    part def Foreign;\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("model/mine.sysml"),
        "package Mine {\n    part def Own;\n}\n",
    )
    .unwrap();
    // the right extension, and not text: a file can be listed and still
    // be unreadable, whether because it is binary or because it is gone
    std::fs::write(root.join("model/binary.sysml"), [0xff, 0xfe, 0x00]).unwrap();

    let (server_side, client_side) = Connection::memory();
    let handle = std::thread::spawn(move || sysml_lsp::run(&server_side).unwrap());
    let mut client = Client {
        connection: client_side,
        next_id: 1,
    };
    let root_uri = lsp_types::Url::from_file_path(&root).unwrap();
    client.request(
        lsp_types::request::Initialize::METHOD,
        json!({
            "capabilities": {},
            "workspaceFolders": [{ "uri": root_uri, "name": "project" }],
            // one relative to the workspace folder, one absolute
            "initializationOptions": { "excludePaths": [
                "vendor",
                root.join("elsewhere").to_str().unwrap(),
            ] }
        }),
    );
    client.notify(lsp_types::notification::Initialized::METHOD, json!({}));

    let app = lsp_types::Url::from_file_path(root.join("model/app.sysml")).unwrap();
    let text = "package App {\n    part a : Mine::Own;\n    part b : Vendored::Borrowed;\n    \
                part c : Other::Foreign;\n}\n";
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": { "uri": app, "languageId": "sysml", "version": 1, "text": text } }),
    );
    let found = client.diagnostics(1);
    assert_eq!(
        Client::messages(&found[app.as_str()]),
        [
            "unresolved reference `Vendored::Borrowed`",
            "unresolved reference `Other::Foreign`",
        ]
    );

    client.request(lsp_types::request::Shutdown::METHOD, Value::Null);
    client.notify(lsp_types::notification::Exit::METHOD, json!({}));
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

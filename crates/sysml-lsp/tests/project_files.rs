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
    // only the newly opened one is published: what this server has to
    // say about `app` has not changed, and saying it again is work an
    // editor does over
    let found = client.diagnostics(1);
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
    // closing publishes an empty set for the file that was closed, so
    // the editor stops showing what this server last said about it
    let found = client.diagnostics(2);
    assert!(found[hardware.as_str()].is_empty(), "{found:?}");
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

/// A root package of one's own named after one of the standard
/// library's still resolves -- each side reads its own -- but the name
/// then means one thing here and another in the library, and nothing in
/// the file says so.
#[test]
fn a_package_named_after_a_library_one_is_pointed_out() {
    let root = std::env::temp_dir().join("sysml-lsp-collision-test");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("model")).unwrap();
    std::fs::create_dir_all(root.join("lib")).unwrap();
    let root = std::fs::canonicalize(&root).unwrap();
    std::fs::write(
        root.join("lib/Requirements.sysml"),
        "standard library package Requirements {\n    part def RequirementCheck;\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("model/mine.sysml"),
        "package Requirements {\n    part def Safe;\n}\n",
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

    let mine = lsp_types::Url::from_file_path(root.join("model/mine.sysml")).unwrap();
    let text = std::fs::read_to_string(root.join("model/mine.sysml")).unwrap();
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": { "uri": mine, "languageId": "sysml", "version": 1, "text": text } }),
    );
    let found = client.diagnostics(1);
    assert_eq!(
        Client::messages(&found[mine.as_str()]),
        ["`Requirements` is also a root package of the standard library"]
    );

    client.request(lsp_types::request::Shutdown::METHOD, Value::Null);
    client.notify(lsp_types::notification::Exit::METHOD, json!({}));
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

/// A fresh directory to be a workspace folder, with every link resolved
/// -- a temporary directory is one on some systems.
fn workspace(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::canonicalize(&root).unwrap()
}

/// A server serving `root` as its one workspace folder.
fn serving(root: &std::path::Path) -> (Client, std::thread::JoinHandle<()>) {
    let (server_side, client_side) = Connection::memory();
    let handle = std::thread::spawn(move || sysml_lsp::run(&server_side).unwrap());
    let mut client = Client {
        connection: client_side,
        next_id: 1,
    };
    let root_uri = lsp_types::Url::from_file_path(root).unwrap();
    client.request(
        lsp_types::request::Initialize::METHOD,
        json!({
            "capabilities": {},
            "workspaceFolders": [{ "uri": root_uri, "name": "project" }],
        }),
    );
    client.notify(lsp_types::notification::Initialized::METHOD, json!({}));
    (client, handle)
}

fn stop(mut client: Client, handle: std::thread::JoinHandle<()>) {
    client.request(lsp_types::request::Shutdown::METHOD, Value::Null);
    client.notify(lsp_types::notification::Exit::METHOD, json!({}));
    handle.join().unwrap();
}

/// How many places in the whole workspace answer to a name.
fn declarations(client: &mut Client, name: &str) -> usize {
    client
        .request(
            lsp_types::request::WorkspaceSymbolRequest::METHOD,
            json!({ "query": name }),
        )
        .as_array()
        .unwrap()
        .len()
}

#[test]
fn a_uri_encoded_the_way_vscode_encodes_it_names_the_file_the_scan_found() {
    let root = workspace("sysml-lsp-encoded-uri-test");
    let path = root.join("plant (v2)+a@b,'c'.sysml");
    let text = "package Weird {\n    part def Thing;\n}\n";
    std::fs::write(&path, text).unwrap();

    let (mut client, handle) = serving(&root);
    // VSCode percent-encodes characters this server's URL type leaves
    // alone; compared as strings the buffer and the scanned file are
    // then two files, and everything in them is declared twice
    let spelled = lsp_types::Url::from_file_path(&path).unwrap().to_string();
    let encoded = spelled
        .replace('(', "%28")
        .replace(')', "%29")
        .replace('+', "%2B")
        .replace('@', "%40")
        .replace('\'', "%27")
        .replace(',', "%2C");
    assert_ne!(encoded, spelled, "nothing in the name needed encoding");
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": {
            "uri": encoded, "languageId": "sysml", "version": 1, "text": text
        }}),
    );
    client.diagnostics(1);
    assert_eq!(declarations(&mut client, "Thing"), 1);
    stop(client, handle);
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn a_file_reached_through_a_link_is_still_one_file() {
    let root = workspace("sysml-lsp-linked-workspace-test");
    std::fs::create_dir_all(root.join("real")).unwrap();
    std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
    let text = "package M {\n    part def Thing;\n    part t : Thing;\n}\n";
    std::fs::write(root.join("real/m.sysml"), text).unwrap();

    // the folder is opened through the link, the document through the
    // path the link points at -- which is how an editor that resolves
    // one and not the other made this server analyse the file twice
    let (mut client, handle) = serving(&root.join("link"));
    let uri = lsp_types::Url::from_file_path(root.join("real/m.sysml")).unwrap();
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": {
            "uri": uri, "languageId": "sysml", "version": 1, "text": text
        }}),
    );
    client.diagnostics(1);
    assert_eq!(declarations(&mut client, "Thing"), 1);

    // and the answer names the file the way the client asked about it,
    // not the way the scan happened to reach it
    let definition = client.request(
        lsp_types::request::GotoDefinition::METHOD,
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 2, "character": 14 }
        }),
    );
    assert_eq!(definition["uri"], Value::String(uri.to_string()));
    stop(client, handle);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_file_written_after_startup_belongs_to_the_project() {
    let root = workspace("sysml-lsp-fresh-file-test");
    let app = root.join("app.sysml");
    std::fs::write(&app, "package App {\n    part x : New::Thing;\n}\n").unwrap();
    let (mut client, handle) = serving(&root);
    let app_uri = lsp_types::Url::from_file_path(&app).unwrap();
    let open = |client: &mut Client, uri: &lsp_types::Url, text: &str| {
        client.notify(
            lsp_types::notification::DidOpenTextDocument::METHOD,
            json!({ "textDocument": {
                "uri": uri, "languageId": "sysml", "version": 1, "text": text
            }}),
        );
    };
    open(
        &mut client,
        &app_uri,
        "package App {\n    part x : New::Thing;\n}\n",
    );
    let found = client.diagnostics(1);
    assert_eq!(
        Client::messages(&found[app_uri.as_str()]),
        ["unresolved reference `New::Thing`"]
    );

    // the project was scanned once, at startup; this file was not there
    let fresh = root.join("new.sysml");
    std::fs::write(&fresh, "package New {\n    part def Thing;\n}\n").unwrap();
    client.notify(
        lsp_types::notification::DidChangeWatchedFiles::METHOD,
        json!({ "changes": [{ "uri": lsp_types::Url::from_file_path(&fresh).unwrap(), "type": 1 }] }),
    );
    let found = client.diagnostics(1);
    assert!(found[app_uri.as_str()].is_empty(), "{found:?}");

    // a buffer opened with unsaved changes already in it -- an editor
    // restored from an earlier session -- says what the file does not
    let fresh_uri = lsp_types::Url::from_file_path(&fresh).unwrap();
    open(
        &mut client,
        &fresh_uri,
        "package New {\n    part def Other;\n}\n",
    );
    let found = client.diagnostics(2);
    assert_eq!(
        Client::messages(&found[app_uri.as_str()]),
        ["unresolved reference `New::Thing`"]
    );
    assert!(found[fresh_uri.as_str()].is_empty(), "{found:?}");

    // and closing it hands the file on disk back its say
    client.notify(
        lsp_types::notification::DidCloseTextDocument::METHOD,
        json!({ "textDocument": { "uri": fresh_uri } }),
    );
    let found = client.diagnostics(2);
    assert!(found[app_uri.as_str()].is_empty(), "{found:?}");
    stop(client, handle);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_internal_view_draws_the_element_in_front_of_you() {
    // `Part` names a definition in every other file of a project as
    // readily as the one being looked at, and the drawing used to be of
    // whichever the model happened to hold first -- which, with the
    // standard library loaded, was never the reader's.
    let root = workspace("sysml-lsp-internal-view-test");
    std::fs::write(
        root.join("elsewhere.sysml"),
        "package Elsewhere {\n    part def Part;\n}\n",
    )
    .unwrap();
    let (mut client, handle) = serving(&root);
    let uri = lsp_types::Url::from_file_path(root.join("mine.sysml")).unwrap();
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": { "uri": uri, "languageId": "sysml", "version": 1, "text":
            "package Mine {\n    part def Part {\n        part w : Wheel;\n    }\n    part def Wheel;\n}\n" }}),
    );
    client.diagnostics(1);

    let drawn = |client: &mut Client, element: &str| {
        client.request(
            "sysml/diagram",
            json!({ "uri": uri, "view": "internal", "element": element }),
        )["svg"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert!(drawn(&mut client, "Part").contains("w : Wheel"));
    // and a qualified name says which one, wherever it is
    assert!(!drawn(&mut client, "Elsewhere::Part").contains("w : Wheel"));
    stop(client, handle);
    let _ = std::fs::remove_dir_all(&root);
}

//! What the editor is not showing: a project file that changed on disk
//! since the last analysis, and a declaration that is not where the name
//! was clicked.
//!
//! Both are places where the server answered about one text while
//! measuring against another.

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use serde_json::{json, Value};

struct Client {
    connection: Connection,
    next_id: i32,
}

impl Client {
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
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
        loop {
            let got = self
                .connection
                .receiver
                .recv_timeout(std::time::Duration::from_secs(30))
                .map_err(|_| format!("no answer to {method}"))?;
            if let Message::Response(Response {
                id: answered,
                result,
                error,
            }) = got
            {
                if answered == id {
                    return match error {
                        Some(err) => Err(err.message),
                        None => Ok(result.unwrap_or(Value::Null)),
                    };
                }
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
}

/// A server serving `root`, with `open` opened as `sysml`.
fn serving(
    root: &std::path::Path,
    open: &std::path::Path,
) -> (Client, std::thread::JoinHandle<()>, lsp_types::Url) {
    let (server_side, client_side) = Connection::memory();
    let handle = std::thread::spawn(move || sysml_lsp::run(&server_side).unwrap());
    let mut client = Client {
        connection: client_side,
        next_id: 1,
    };
    client
        .request(
            lsp_types::request::Initialize::METHOD,
            json!({
                "capabilities": {},
                "workspaceFolders": [
                    { "uri": lsp_types::Url::from_file_path(root).unwrap(), "name": "project" }
                ],
            }),
        )
        .unwrap();
    client.notify(lsp_types::notification::Initialized::METHOD, json!({}));
    let uri = lsp_types::Url::from_file_path(open).unwrap();
    let text = std::fs::read_to_string(open).unwrap();
    client.notify(
        lsp_types::notification::DidOpenTextDocument::METHOD,
        json!({ "textDocument": {
            "uri": uri, "languageId": "sysml", "version": 1, "text": text
        }}),
    );
    (client, handle, uri)
}

fn stop(mut client: Client, handle: std::thread::JoinHandle<()>) {
    client
        .request(lsp_types::request::Shutdown::METHOD, Value::Null)
        .unwrap();
    client.notify(lsp_types::notification::Exit::METHOD, json!({}));
    handle.join().unwrap();
}

#[test]
fn a_project_file_rewritten_on_disk_does_not_take_the_server_down() {
    let root = std::env::temp_dir().join("sysml-lsp-stale-test");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let root = std::fs::canonicalize(&root).unwrap();
    let declaring = root.join("a.sysml");
    let using = root.join("b.sysml");
    std::fs::write(&declaring, "package A {\n    part def Thing;\n}\n").unwrap();
    std::fs::write(&using, "package B {\n    part t : A::Thing;\n}\n").unwrap();

    let (mut client, handle, uri) = serving(&root, &using);
    let where_ = |answer: &Value| {
        (answer["range"]["start"]["line"].as_u64().unwrap(), {
            answer["range"]["start"]["character"].as_u64().unwrap()
        })
    };
    let asked = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 17 },
    });
    let first = client
        .request(lsp_types::request::GotoDefinition::METHOD, asked.clone())
        .unwrap();
    assert_eq!(where_(&first), (1, 13));

    // The file the answer points into is rewritten behind the editor's
    // back, and every byte after the first line moves. Read afresh, the
    // offset the analysis holds lands inside a character of the new
    // text -- which is where slicing it killed the server.
    std::fs::write(
        &declaring,
        "package A { /* \u{3042}\u{3042}\u{3042}\u{3042}\u{3042} */ part def Thing; }\n",
    )
    .unwrap();
    let again = client
        .request(lsp_types::request::GotoDefinition::METHOD, asked)
        .expect("the server is still answering");
    // and it still answers about the text it analysed, not the new one
    assert_eq!(where_(&again), (1, 13));
    stop(client, handle);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn renaming_a_borrowed_name_rewrites_the_declaration_it_came_from() {
    let root = std::env::temp_dir().join("sysml-lsp-borrowed-name-test");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let root = std::fs::canonicalize(&root).unwrap();
    let path = root.join("m.sysml");
    let source = "package P {\n\
                  \x20   part def Comp;\n\
                  \x20   part def Logical { part component : Comp; }\n\
                  \x20   part l : Logical { part :>> component; }\n\
                  }\n";
    std::fs::write(&path, source).unwrap();

    let (mut client, handle, uri) = serving(&root, &path);
    // asked of the `component` in `:>> component`, which declares no
    // name of its own: there is nothing there to rewrite
    let edit = client
        .request(
            lsp_types::request::Rename::METHOD,
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 3, "character": 32 },
                "newName": "unit",
            }),
        )
        .unwrap();
    let edits = edit["changes"][uri.as_str()].as_array().unwrap();
    let mut spans: Vec<(u64, u64, u64)> = edits
        .iter()
        .map(|edit| {
            (
                edit["range"]["start"]["line"].as_u64().unwrap(),
                edit["range"]["start"]["character"].as_u64().unwrap(),
                edit["range"]["end"]["character"].as_u64().unwrap(),
            )
        })
        .collect();
    spans.sort_unstable();
    // the declaration on line 2 and the mention on line 3, each the
    // width of the name and nothing more -- the whole `part :>>
    // component;` used to be replaced by `unit`
    assert_eq!(spans, [(2, 28, 37), (3, 32, 41)]);
    assert!(
        edits.iter().all(|edit| edit["newText"] == "unit"),
        "{edits:?}"
    );
    stop(client, handle);
    let _ = std::fs::remove_dir_all(&root);
}

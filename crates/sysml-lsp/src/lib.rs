//! Language server for SysML v2 / KerML.
//!
//! Features: parse and name-resolution diagnostics, go-to-definition,
//! hover, document symbols, and whole-file formatting. The standard
//! library is preloaded so references into it resolve and definitions
//! inside it can be jumped to: the copy built into this binary, or the
//! directory given in `initializationOptions.libraryPath` (or
//! `SYSML_LIBRARY_PATH`). `initializationOptions.noLibrary` asks for none
//! at all -- loading and resolving one is the whole of what starting
//! costs.
//!
//! Analysis is three layers, each rebuilt by a rarer event than the one
//! above it. The standard library is parsed and resolved once at startup.
//! On top of it sit the project's own files, rebuilt when a document is
//! opened or closed, since a file open in the editor is read from its
//! buffer. On top of those sit the open buffers, rebuilt on every
//! keystroke. A model is written across several files that import each
//! other, so without the middle layer a file would resolve only against
//! whatever happened to be open in a tab.
//!
//! The middle layer leaves out `initializationOptions.excludePaths` -- a
//! vendored corpus is not the project's to read on every open -- and takes
//! in the model files beside each open document, so a file opened out of
//! an excluded directory is still read in the company it was written in.
//!
//! Run the binary (`sysml-lsp`) over stdio, or drive [`run`] with an
//! in-memory [`Connection`] for testing.
//!
//! # Two hosts
//!
//! Nothing here reads a message from anywhere. [`Session`] takes one
//! message and hands back the messages that answer it; [`run`] is that
//! session with `lsp_server`'s two threads and a channel around it, and
//! `sysmlv2-wasm` is the same session in a browser's worker, which has
//! one thread and may block it for nothing. Where the project's files
//! come from differs the same way, and that is [`Files`].

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::path::{Path, PathBuf};

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use lsp_types::{
    CodeActionProviderCapability, CompletionItemKind, CompletionOptions, Diagnostic,
    DiagnosticSeverity, DocumentSymbol, NumberOrString, OneOf, PublishDiagnosticsParams,
    ServerCapabilities, SignatureHelpOptions, SymbolKind, TextDocumentSyncCapability,
    TextDocumentSyncKind, Url,
};
use sysml_model::ElementKind;
use sysml_semantics::Workspace;
use sysml_syntax::{TextRange, TextSize};

mod files;
mod line_index;
pub use files::Files;
use line_index::LineIndex;

/// Capabilities advertised by this server.
pub fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::INCREMENTAL,
        )),
        definition_provider: Some(OneOf::Left(true)),
        hover_provider: Some(lsp_types::HoverProviderCapability::Simple(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        document_formatting_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        rename_provider: Some(OneOf::Left(true)),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![":".into(), ">".into()]),
            ..Default::default()
        }),
        workspace_symbol_provider: Some(OneOf::Left(true)),
        // what a name that resolved to nothing might have meant, as
        // edits a client can apply
        code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
        signature_help_provider: Some(SignatureHelpOptions {
            trigger_characters: Some(vec!["(".into(), ",".into()]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Serve on `connection` until the client says to stop.
///
/// The standard-library directory comes from
/// `initializationOptions.libraryPath` or `SYSML_LIBRARY_PATH`; with
/// neither, the copy built into this binary is used, and
/// `initializationOptions.noLibrary` asks for none at all.
pub fn run(connection: &Connection) -> Result<(), Box<dyn Error + Sync + Send>> {
    let mut session = Session::new(Files::Disk);
    let mut out = Vec::new();
    for message in &connection.receiver {
        let flow = session.handle(message, &mut out);
        for answer in out.drain(..) {
            connection.sender.send(answer)?;
        }
        if let Flow::Over(how) = flow {
            return how.map_err(Into::into);
        }
    }
    // the client hung up without saying so, which is not a failure: an
    // editor that was closed takes its server with it
    Ok(())
}

/// What a [`Session`] wants next.
#[derive(Debug)]
pub enum Flow {
    /// Another message, whenever one comes.
    Go,
    /// Nothing more: the session is over.
    ///
    /// `Err` says it ended badly, which a program returns as its exit
    /// status. The specification asks a server told to leave before it
    /// was asked to shut down to fail, so that a client can tell a stop
    /// it asked for from one it did not.
    Over(Result<(), String>),
}

/// A language server driven one message at a time.
///
/// The conversation the specification describes is a sequence -- nothing
/// before `initialize`, nothing but `initialized` after it, nothing but
/// `exit` after `shutdown` -- so a session is that sequence, and a
/// message that does not belong to the phase it arrives in ends it.
pub struct Session {
    phase: Phase,
}

/// How far through that conversation a session is.
enum Phase {
    /// Nothing has been agreed. What the client says in `initialize`
    /// decides what the server is built with, so the files it will read
    /// wait here until then.
    Starting(Files),
    /// `initialize` is answered, and `initialized` is what the
    /// specification says comes next.
    Greeting(Box<Server>),
    /// Open for questions.
    Open(Box<Server>),
    /// `shutdown` is answered, and `exit` is what may follow it.
    Closing,
    /// Over. Nothing here answers anything.
    Over,
}

impl Session {
    /// A session that has yet to hear from a client, reading the
    /// project's files from `files`.
    pub fn new(files: Files) -> Session {
        Session {
            phase: Phase::Starting(files),
        }
    }

    /// Whether the conversation is finished with.
    pub fn over(&self) -> bool {
        matches!(self.phase, Phase::Over)
    }

    /// Take one message and put whatever answers it in `out`.
    pub fn handle(&mut self, message: Message, out: &mut Vec<Message>) -> Flow {
        // Taken rather than borrowed, since what a message mostly does
        // is move the session from one phase into the next and the
        // server has to move with it. A phase that is not put back is
        // `Over`, which is where a failure leaves it anyway.
        let (phase, flow) = match std::mem::replace(&mut self.phase, Phase::Over) {
            Phase::Starting(files) => starting(files, message, out),
            Phase::Greeting(server) => greeting(server, message),
            Phase::Open(server) => open(server, message, out),
            Phase::Closing => (Phase::Over, closing(message)),
            Phase::Over => (Phase::Over, Flow::Over(Ok(()))),
        };
        self.phase = phase;
        flow
    }
}

/// Before `initialize`: the one request that may come, and what the
/// specification says to do with anything else.
fn starting(files: Files, message: Message, out: &mut Vec<Message>) -> (Phase, Flow) {
    match message {
        Message::Request(req) if req.method == lsp_types::request::Initialize::METHOD => {
            match begin(files, req.params) {
                Ok(mut server) => {
                    let result = serde_json::json!({ "capabilities": server_capabilities() });
                    out.push(Response::new_ok(req.id, result).into());
                    // what went wrong while the server was built, now
                    // that there is a client to tell: nothing may be
                    // said before the handshake is answered
                    for complaint in std::mem::take(&mut server.complaints) {
                        log(out, &complaint);
                    }
                    (Phase::Greeting(server), Flow::Go)
                }
                // the session is over either way, but a client left
                // waiting for an answer reports a server that would not
                // start and nothing about why
                Err(why) => {
                    out.push(
                        Response::new_err(
                            req.id,
                            lsp_server::ErrorCode::InvalidParams as i32,
                            why.clone(),
                        )
                        .into(),
                    );
                    (Phase::Over, Flow::Over(Err(why)))
                }
            }
        }
        // a client that asks something else first is told so, and may
        // still say `initialize` afterwards
        Message::Request(req) => {
            out.push(
                Response::new_err(
                    req.id,
                    lsp_server::ErrorCode::ServerNotInitialized as i32,
                    format!("expected an `initialize` request, got `{}`", req.method),
                )
                .into(),
            );
            (Phase::Starting(files), Flow::Go)
        }
        Message::Notification(note) if note.method == lsp_types::notification::Exit::METHOD => (
            Phase::Over,
            Flow::Over(Err("`exit` before `initialize`".into())),
        ),
        // anything else a client says before the handshake is not this
        // server's to answer, and is not a reason to refuse to start
        Message::Notification(_) => (Phase::Starting(files), Flow::Go),
        Message::Response(_) => (
            Phase::Over,
            Flow::Over(Err(
                "an answer before `initialize`, to a question nothing asked".into(),
            )),
        ),
    }
}

/// Between `initialize` and `initialized`, where the specification lets
/// nothing else through.
fn greeting(server: Box<Server>, message: Message) -> (Phase, Flow) {
    match message {
        Message::Notification(note)
            if note.method == lsp_types::notification::Initialized::METHOD =>
        {
            (Phase::Open(server), Flow::Go)
        }
        // `$/...` is the protocol's own traffic, which the
        // specification says a server may ignore and a client may send
        // whenever it likes: refusing to start over one would be a
        // server some clients cannot start at all.
        Message::Notification(note) if note.method.starts_with("$/") => {
            (Phase::Greeting(server), Flow::Go)
        }
        other => (
            Phase::Over,
            Flow::Over(Err(format!("expected `initialized`, got {other:?}"))),
        ),
    }
}

/// Open: a question is answered, a notification is taken in, and `exit`
/// without a `shutdown` before it is a failure.
fn open(mut server: Box<Server>, message: Message, out: &mut Vec<Message>) -> (Phase, Flow) {
    match message {
        Message::Request(req) if req.method == lsp_types::request::Shutdown::METHOD => {
            out.push(Response::new_ok(req.id, ()).into());
            (Phase::Closing, Flow::Go)
        }
        Message::Request(req) => {
            let response = server.handle_request(&req);
            out.push(Message::Response(response));
            (Phase::Open(server), Flow::Go)
        }
        Message::Notification(note) if note.method == lsp_types::notification::Exit::METHOD => (
            Phase::Over,
            Flow::Over(Err("`exit` without a `shutdown` before it".into())),
        ),
        Message::Notification(note) => {
            server.handle_notification(out, note);
            (Phase::Open(server), Flow::Go)
        }
        // this server asks the client nothing, so an answer is nothing
        // it is waiting for
        Message::Response(_) => (Phase::Open(server), Flow::Go),
    }
}

/// After `shutdown`, where `exit` is the message the specification names
/// and no other is.
fn closing(message: Message) -> Flow {
    match message {
        Message::Notification(note) if note.method == lsp_types::notification::Exit::METHOD => {
            Flow::Over(Ok(()))
        }
        other => Flow::Over(Err(format!(
            "expected `exit` after `shutdown`, got {other:?}"
        ))),
    }
}

/// Build the server the client's `initialize` asks for.
fn begin(mut files: Files, params: serde_json::Value) -> Result<Box<Server>, String> {
    let init: lsp_types::InitializeParams = serde_json::from_value(params)
        .map_err(|err| format!("`initialize` cannot be read: {err}"))?;

    let option = |name: &str| {
        init.initialization_options
            .as_ref()
            .and_then(|o| o.get(name))
            .and_then(|v| v.as_str())
            .map(String::from)
    };

    // What the project is made of, where the host has no filesystem to
    // find out for itself. In the handshake rather than a notification
    // after it: files sent afterwards would race the client's own
    // `didOpen`, and the document that opened would be diagnosed
    // against a project of nothing.
    if let Some(handed) = init
        .initialization_options
        .as_ref()
        .and_then(|o| o.get("files"))
    {
        files.provide_all(handed_files(handed));
    }

    let library_path = option("libraryPath").or_else(|| std::env::var("SYSML_LIBRARY_PATH").ok());
    // A client that wants no library at all says so, rather than naming
    // a path it hopes is unreadable.
    let without = init
        .initialization_options
        .as_ref()
        .and_then(|o| o.get("noLibrary"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let library = match (without, library_path) {
        (true, _) => crate::project::Library::None,
        (_, Some(path)) => crate::project::Library::At(std::path::PathBuf::from(path)),
        (_, None) => crate::project::Library::BuiltIn,
    };

    // `workspaceFolders` is what a modern client sends; `rootUri` is what
    // an older one sends and what a client with no folder open sends
    let roots: Vec<std::path::PathBuf> = init
        .workspace_folders
        .iter()
        .flatten()
        .map(|folder| &folder.uri)
        .chain(init.root_uri.iter())
        .filter_map(|uri| files.path_of(uri))
        .collect();

    // a workspace may contain models that are not the project's own --
    // a vendored corpus, a copy of someone else's model -- and loading
    // them costs time on every open for names nobody is editing
    let excluded: Vec<std::path::PathBuf> = init
        .initialization_options
        .as_ref()
        .and_then(|o| o.get("excludePaths"))
        .and_then(|v| v.as_array())
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| entry.as_str())
        .flat_map(|entry| {
            let path = std::path::Path::new(entry);
            // an entry that is already a whole URI names one place, the
            // way an absolute path does -- which is how a client says it
            // where the workspace is not a filesystem
            if path.is_absolute() || Url::parse(entry).is_ok() {
                vec![path.to_path_buf()]
            } else {
                // a relative entry is one per workspace folder, the way a
                // client writes `"vendor"` and means its own `vendor`
                roots.iter().map(|root| root.join(path)).collect()
            }
        })
        .collect();

    let mut server = Box::new(Server::new(files, library, &roots, &excluded));
    // How wide a line may be before formatting breaks it. Read once, as
    // the library path is: a client that changes it restarts the server,
    // which is what changing any of these takes.
    if let Some(width) = init
        .initialization_options
        .as_ref()
        .and_then(|o| o.get("formatWidth"))
        .and_then(serde_json::Value::as_u64)
    {
        server.layout = sysml_syntax::fmt::Layout {
            width: width as usize,
        };
    }
    // How the preview is painted: the name of a skin, or one written
    // out. A skin that will not read leaves the drawing as it was and
    // says why where the client collects this server's log -- a preview
    // that came back unpainted and silent would be a setting nobody
    // could debug.
    if let Some(said) = init
        .initialization_options
        .as_ref()
        .and_then(|o| o.get("skin"))
    {
        match sysml_diagram::skin::read(said) {
            Ok(skin) => server.skin = skin,
            Err(why) => server
                .complaints
                .push(format!("the skin is not read: {why}")),
        }
    }
    Ok(server)
}

/// The `[{ "uri": …, "text": … }]` a client hands a project's files over
/// as, in `initializationOptions.files` and in `sysml/files`.
fn handed_files(said: &serde_json::Value) -> Vec<(PathBuf, String)> {
    serde_json::from_value::<Vec<HandedFile>>(said.clone())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|file| Some((PathBuf::from(file.uri), file.text?)))
        .collect()
}

/// Say something in the client's log.
///
/// This was `eprintln!`, and a worker has no standard error: what is
/// written there goes nowhere, so a library path that would not open was
/// a setting that went wrong in silence. VSCode shows this in the
/// channel it showed standard error in.
fn log(out: &mut Vec<Message>, said: &str) {
    let params = serde_json::json!({
        "type": lsp_types::MessageType::WARNING,
        "message": format!("sysml-lsp: {said}"),
    });
    out.push(Message::Notification(Notification {
        method: lsp_types::notification::LogMessage::METHOD.into(),
        params,
    }));
}

mod project;
mod requests;

/// The server's whole state: the layers a question is answered
/// against, the documents the client holds open, and what was last
/// published about each of them.
pub struct Server {
    /// where the project's own files are read from
    files: Files,
    /// what went wrong before there was a client to tell, said as soon
    /// as the handshake is answered
    complaints: Vec<String>,
    /// library workspace, parsed and fully resolved once at startup; the
    /// layers above clone it, resolution caches and all
    library: Workspace,
    /// the workspace folders, rescanned whenever the project layer is
    /// rebuilt
    roots: Vec<PathBuf>,
    /// directories the project does not reach into: the library's own --
    /// it is already in `library` -- and whatever the client excluded
    excluded: Vec<PathBuf>,
    /// the library's own directory on its own, which is the one
    /// exclusion an open document cannot talk the project out of
    library_dir: Option<PathBuf>,
    /// every model file in the workspace folders, as of the last scan
    project_files: Vec<PathBuf>,
    /// library + the project files no buffer contradicts, resolved
    project: Option<Workspace>,
    /// where each file the project layer read from disk ended up, by the
    /// path with every link resolved
    project_index: HashMap<PathBuf, usize>,
    /// project files the layer left out because an open buffer says
    /// something else about them
    substituted: HashSet<PathBuf>,
    /// open documents
    docs: HashMap<Url, String>,
    /// the language each open document was declared to be, which is all
    /// there is to go on where its URI carries no suffix
    languages: HashMap<Url, sysml_syntax::Dialect>,
    /// cached analysis, invalidated on document changes
    analysis: Option<Analysis>,
    /// what was last published about each open document, so that what
    /// has not changed is not said again
    published: HashMap<Url, Vec<Diagnostic>>,
    /// what the formatter is told, where the client has an opinion
    layout: sysml_syntax::fmt::Layout,
    /// and what the preview is painted in
    skin: sysml_diagram::Skin,
}

/// One analysis pass over the library + the project + all open documents.
struct Analysis {
    ws: Workspace,
    doc_files: HashMap<Url, usize>,
    /// files this pass has already been asked to place something in,
    /// each with the text every range was measured against and its
    /// lines found once
    indexed: HashMap<usize, Placed>,
}

/// A file as the client knows it: what it is called, what it says, and
/// where its lines begin.
struct Placed {
    url: Url,
    text: String,
    index: LineIndex,
}

/// The parameters of a notification, or nothing when they are not what the
/// method says they are.
///
/// A client is not supposed to send such a thing, and one that does is not
/// a reason to stop: an editor whose language server exits mid-session
/// leaves the file without diagnostics or completion until the window is
/// reloaded. The complaint goes to the client's log, which is where a
/// client collects a server's.
fn taken<T: serde::de::DeserializeOwned>(note: Notification, out: &mut Vec<Message>) -> Option<T> {
    let method = note.method.clone();
    match serde_json::from_value(note.params) {
        Ok(params) => Some(params),
        Err(err) => {
            log(
                out,
                &format!("ignoring a `{method}` this server cannot read: {err}"),
            );
            None
        }
    }
}

impl Server {
    fn handle_notification(&mut self, out: &mut Vec<Message>, note: Notification) {
        use lsp_types::notification::*;
        match note.method.as_str() {
            DidOpenTextDocument::METHOD => {
                let Some(params) = taken::<lsp_types::DidOpenTextDocumentParams>(note, out) else {
                    return;
                };
                let uri = params.text_document.uri;
                let language = params.text_document.language_id;
                self.languages.insert(
                    uri.clone(),
                    if language.eq_ignore_ascii_case("kerml") {
                        sysml_syntax::Dialect::KerML
                    } else {
                        sysml_syntax::Dialect::SysML
                    },
                );
                self.docs.insert(uri.clone(), params.text_document.text);
                // a buffer opened with unsaved changes already in it --
                // an editor restored from a previous session -- says
                // something the file does not; and a document from
                // outside the project brings the files beside it in,
                // which the layer below has yet to read
                if self.contradicted(&uri) || self.outside_the_project(&uri) {
                    self.project = None;
                }
                self.analysis = None;
                self.publish_diagnostics(out, Some(&uri));
            }
            DidChangeTextDocument::METHOD => {
                let Some(params) = taken::<lsp_types::DidChangeTextDocumentParams>(note, out)
                else {
                    return;
                };
                let uri = params.text_document.uri;
                if let Some(text) = self.docs.get_mut(&uri) {
                    for change in params.content_changes {
                        apply_change(text, change);
                    }
                }
                // only the first keystroke costs this: once the layer
                // has been built without the file, it no longer holds a
                // copy of it to be contradicted
                if self.contradicted(&uri) {
                    self.project = None;
                }
                self.analysis = None;
                self.publish_diagnostics(out, Some(&uri));
            }
            // a file written, renamed or deleted outside the editor
            // changes what the project is and what it says
            DidChangeWatchedFiles::METHOD => {
                let Some(_) = taken::<lsp_types::DidChangeWatchedFilesParams>(note, out) else {
                    return;
                };
                self.project = None;
                self.analysis = None;
                self.publish_diagnostics(out, None);
            }
            DidCloseTextDocument::METHOD => {
                let Some(params) = taken::<lsp_types::DidCloseTextDocumentParams>(note, out) else {
                    return;
                };
                self.docs.remove(&params.text_document.uri);
                self.languages.remove(&params.text_document.uri);
                self.published.remove(&params.text_document.uri);
                // the buffer that spoke for the file is gone, so the
                // file on disk speaks for it again
                if self
                    .file_of(&params.text_document.uri)
                    .is_some_and(|path| self.substituted.contains(&path))
                {
                    self.project = None;
                }
                self.analysis = None;
                // what the editor still shows for a file it has closed is
                // whatever this server last said about it, and nothing
                // would ever replace it
                let cleared = PublishDiagnosticsParams {
                    uri: params.text_document.uri,
                    diagnostics: Vec::new(),
                    version: None,
                };
                let cleared = serde_json::to_value(cleared).expect("diagnostics serialize");
                out.push(Message::Notification(lsp_server::Notification {
                    method: PublishDiagnostics::METHOD.into(),
                    params: cleared,
                }));
                // what the closed file said reached the others: a
                // sibling that imported it was reading the buffer and
                // now reads the file, and nothing else would tell it so
                self.publish_diagnostics(out, None);
            }
            // How the set the handshake brought changes afterwards: a
            // file written, renamed or deleted, or the files beside a
            // document opened from outside the workspace. A `text` of
            // null is a file the client says is gone.
            "sysml/files" => {
                let Some(params) = taken::<HandedFiles>(note, out) else {
                    return;
                };
                let mut news = false;
                for file in params.files {
                    news |= self.files.provide(PathBuf::from(file.uri), file.text);
                }
                // a client may hand over what the server already has --
                // a save of an unedited buffer, a watcher that fired
                // twice -- and rebuilding for that costs the project
                if news {
                    self.project = None;
                    self.analysis = None;
                    self.publish_diagnostics(out, None);
                }
            }
            _ => {}
        }
    }

    /// Say what is wrong with each open document, where that has changed.
    ///
    /// `changed` is the document the client has just told this server about,
    /// which is published either way: a client that waits to hear back about
    /// the keystroke it sent has to hear something. Every keystroke in one
    /// file used to republish every open file, and an editor takes each of
    /// those apart and lays it out again.
    fn publish_diagnostics(&mut self, out: &mut Vec<Message>, changed: Option<&Url>) {
        let docs = self.docs.clone();
        let analysis = self.analysis();
        // Taken by value: what the constraints are asked of needs the
        // workspace itself, and the map cannot be borrowed across that.
        let open: Vec<(Url, usize)> = analysis
            .doc_files
            .iter()
            .map(|(url, file)| (url.clone(), *file))
            .collect();
        let mut fresh: Vec<(Url, Vec<Diagnostic>)> = Vec::new();
        for (url, file) in open {
            let url = &url;
            let text = &docs[url];
            let index = LineIndex::new(text);
            // an editor shows both halves: what does not parse yet is
            // not a reason to stop saying what does not resolve
            let diagnosed = analysis.ws.diagnose(&[file]);
            let found = &diagnosed.found;
            let mut diagnostics = Vec::new();
            for (findings, severity, say) in [
                (
                    &found.syntax,
                    DiagnosticSeverity::ERROR,
                    &(|what: &str| what.to_string()) as &dyn Fn(&str) -> String,
                ),
                (
                    &found.names,
                    DiagnosticSeverity::WARNING,
                    &(|what: &str| format!("unresolved reference `{what}`"))
                        as &dyn Fn(&str) -> String,
                ),
                // a root package named after one of the library's
                // resolves, but not to what the library means by it
                (
                    &found.collisions,
                    DiagnosticSeverity::WARNING,
                    &(|what: &str| what.to_string()) as &dyn Fn(&str) -> String,
                ),
            ] {
                for finding in findings {
                    diagnostics.push(Diagnostic {
                        range: index.range(text, finding.range),
                        severity: Some(severity),
                        source: Some("sysml".into()),
                        message: say(&finding.what),
                        ..Default::default()
                    });
                }
            }
            // What the specification requires, over and above every name resolving.
            // Asked of one document it costs about a millisecond, so an editor can be
            // told while it is typed. Whether it was worth asking is `diagnose`'s to
            // decide, the same way for all three front ends.
            //
            // A violation names an element, which may be one the notation never
            // wrote; `element_place` walks out to the nearest thing that was.
            for violation in &diagnosed.rules.violations {
                // Asked of this document, every violation is about an
                // element under it, and the walk out to the nearest
                // written thing stays inside it. The one element of a
                // workspace under no file at all is its root, which no
                // constraint is about.
                let (_, range) = analysis
                    .ws
                    .element_place(violation.element)
                    .unwrap_or_default();
                diagnostics.push(Diagnostic {
                    range: index.range(text, range),
                    severity: Some(DiagnosticSeverity::WARNING),
                    source: Some("sysml".into()),
                    code: Some(NumberOrString::String(violation.rule.into())),
                    message: violation.says.into(),
                    ..Default::default()
                });
            }
            fresh.push((url.clone(), diagnostics));
        }
        for (url, diagnostics) in fresh {
            let said = self.published.get(&url);
            if said.is_some_and(|last| *last == diagnostics) && changed != Some(&url) {
                continue;
            }
            self.published.insert(url.clone(), diagnostics.clone());
            let params = PublishDiagnosticsParams {
                uri: url,
                diagnostics,
                version: None,
            };
            let params = serde_json::to_value(params).expect("diagnostics serialize");
            out.push(Message::Notification(Notification {
                method: lsp_types::notification::PublishDiagnostics::METHOD.into(),
                params,
            }));
        }
    }

    fn handle_request(&mut self, req: &Request) -> Response {
        use lsp_types::request::*;
        let id = req.id.clone();
        // Nine requests, each of which begins by reading its own
        // parameter type out of the same value and answering the client
        // where it cannot. Written out, that was nine copies of one
        // five-line block and the greater part of this function.
        macro_rules! asked {
            ($params:ty) => {
                match serde_json::from_value::<$params>(req.params.clone()) {
                    Ok(params) => params,
                    Err(error) => return error_response(id, error),
                }
            };
        }
        match req.method.as_str() {
            GotoDefinition::METHOD => {
                let position =
                    asked!(lsp_types::GotoDefinitionParams).text_document_position_params;
                ok_response(
                    id,
                    self.definition(&position.text_document.uri, position.position),
                )
            }
            HoverRequest::METHOD => {
                let position = asked!(lsp_types::HoverParams).text_document_position_params;
                ok_response(
                    id,
                    self.hover(&position.text_document.uri, position.position),
                )
            }
            DocumentSymbolRequest::METHOD => {
                let params = asked!(lsp_types::DocumentSymbolParams);
                ok_response(id, self.document_symbols(&params.text_document.uri))
            }
            References::METHOD => {
                let params = asked!(lsp_types::ReferenceParams);
                let include_declaration = params.context.include_declaration;
                let position = params.text_document_position;
                ok_response(
                    id,
                    self.references(
                        &position.text_document.uri,
                        position.position,
                        include_declaration,
                    ),
                )
            }
            Rename::METHOD => {
                let params = asked!(lsp_types::RenameParams);
                let position = params.text_document_position;
                match self.rename(
                    &position.text_document.uri,
                    position.position,
                    &params.new_name,
                ) {
                    // the reason is the useful part: an editor shows it
                    // where it would otherwise say only that nothing
                    // happened
                    Ok(edit) => ok_response(id, Some(edit)),
                    Err(reason) => error_response(id, reason),
                }
            }
            Completion::METHOD => {
                let position = asked!(lsp_types::CompletionParams).text_document_position;
                ok_response(
                    id,
                    self.completion(&position.text_document.uri, position.position),
                )
            }
            WorkspaceSymbolRequest::METHOD => {
                let params = asked!(lsp_types::WorkspaceSymbolParams);
                ok_response(id, self.workspace_symbols(&params.query))
            }
            SignatureHelpRequest::METHOD => {
                let position = asked!(lsp_types::SignatureHelpParams).text_document_position_params;
                ok_response(
                    id,
                    self.signature_help(&position.text_document.uri, position.position),
                )
            }
            Formatting::METHOD => {
                let params = asked!(lsp_types::DocumentFormattingParams);
                ok_response(id, self.format(&params.text_document.uri))
            }
            CodeActionRequest::METHOD => {
                let params = asked!(lsp_types::CodeActionParams);
                ok_response(
                    id,
                    self.code_actions(&params.text_document.uri, params.range),
                )
            }
            // custom: the diagram of one open document, as a standalone
            // SVG -- what the `sysml diagram` CLI draws, served from the
            // buffer so a preview can follow unsaved edits
            "sysml/diagram" => {
                let params = asked!(DiagramParams);
                ok_response(id, self.diagram(&params))
            }
            _ => Response::new_err(
                id,
                lsp_server::ErrorCode::MethodNotFound as i32,
                format!("unhandled method {}", req.method),
            ),
        }
    }
}

/// Where inside a recorded range the name being renamed is written.
///
/// What was recorded is not always the name alone. A mention can be a
/// feature chain -- `system.sub1`, ending on the step that names what is
/// being renamed -- and a declaration whose tree holds no name node of its
/// own is recorded as the whole of it: `then fork F { ... }`, which the
/// new name would replace, body and all.
fn name_token(ws: &Workspace, file: usize, range: TextRange, name: &str) -> Option<TextRange> {
    let syntax = ws.file_parse(file).syntax();
    let first = syntax.token_at_offset(range.start()).right_biased();
    std::iter::successors(first, sysml_syntax::SyntaxToken::next_token)
        .take_while(|token| range.contains_range(token.text_range()))
        .find(|token| sysml_syntax::unquote(token.text()) == name)
        .map(|token| token.text_range())
}

/// The element an `internal` view is asked for, by the name a reader
/// typed.
///
/// The document's own before anything else's: `Part` names the standard
/// library's `Parts::Part` as readily as the definition in front of
/// you, and drawing whichever the model happened to hold first drew the
/// library's. A qualified name says which one is meant and is taken as
/// written.
fn element_named(ws: &Workspace, file: usize, name: &str) -> Option<sysml_model::ElementId> {
    let leaf = name.rsplit("::").next().unwrap_or(name);
    let mut candidates: Vec<sysml_model::ElementId> = ws
        .named_elements()
        .filter(|(_, declared)| *declared == leaf)
        .map(|(id, _)| id)
        .collect();
    if leaf != name {
        let tail = format!("::{name}");
        candidates.retain(|id| {
            let qualified = ws.qualified_name_of(*id);
            qualified == name || qualified.ends_with(&tail)
        });
    }
    candidates
        .iter()
        .find(|id| ws.element_file(**id) == Some(file))
        .or(candidates.first())
        .copied()
}

impl Server {
    /// The file a URL names, in the one spelling this host agrees on --
    /// see [`Files::path_of`], which is where the two hosts differ.
    fn file_of(&self, url: &Url) -> Option<PathBuf> {
        self.files.path_of(url)
    }
}

/// The parameters of the custom `sysml/files` notification.
#[derive(serde::Deserialize)]
struct HandedFiles {
    files: Vec<HandedFile>,
}

/// One file a client hands over: what it is called, and what it says --
/// or nothing, where it says the file is gone.
#[derive(serde::Deserialize)]
struct HandedFile {
    uri: String,
    text: Option<String>,
}

/// Parameters of the custom `sysml/diagram` request.
#[derive(serde::Deserialize)]
struct DiagramParams {
    uri: String,
    /// `definitions` (default), `internal` or `browser`.
    view: Option<String>,
    /// The element an `internal` view is of.
    element: Option<String>,
    /// `file` (default) draws the document on its own; `directory` draws
    /// every model file in its directory and below alongside it.
    scope: Option<String>,
}

/// Result of the custom `sysml/diagram` request.
#[derive(serde::Serialize)]
struct DiagramResult {
    svg: String,
}

/// Apply one LSP content change (ranged or whole-document) to `text`.
fn apply_change(text: &mut String, change: lsp_types::TextDocumentContentChangeEvent) {
    match change.range {
        Some(range) => {
            let index = LineIndex::new(text);
            let (Some(start), Some(end)) = (
                index.offset(text, range.start),
                index.offset(text, range.end),
            ) else {
                *text = change.text;
                return;
            };
            let (start, end) = (usize::from(start), usize::from(end));
            if start <= end && end <= text.len() {
                text.replace_range(start..end, &change.text);
            } else {
                *text = change.text;
            }
        }
        None => *text = change.text,
    }
}

/// Whether the name written at `at` in `file` is looked up in the scope
/// that surrounds it.
///
/// A segment that follows `::` or `.` is looked up inside whatever the
/// segment before it named, so nothing about the scope it is written in
/// decides where it lands.
fn looked_up_in_place(ws: &Workspace, file: usize, at: TextSize) -> bool {
    let syntax = ws.file_parse(file).syntax();
    let mut token = syntax.token_at_offset(at).left_biased();
    while token.as_ref().is_some_and(|t| t.kind().is_trivia()) {
        token = token.and_then(|t| t.prev_token());
    }
    !token.is_some_and(|t| {
        matches!(
            t.kind(),
            sysml_syntax::SyntaxKind::COLON_COLON | sysml_syntax::SyntaxKind::DOT
        )
    })
}

/// How an editor should picture an element, in the two vocabularies it
/// asks for it in.
///
/// One decision written once: kept apart, the completion list and the
/// symbol tree drifted into disagreeing about what a thing is.
fn pictured(kind: ElementKind) -> (CompletionItemKind, SymbolKind) {
    if kind.is_a(ElementKind::Package) || kind == ElementKind::Namespace {
        (CompletionItemKind::MODULE, SymbolKind::MODULE)
    } else if kind.is_a(ElementKind::Classifier) {
        (CompletionItemKind::CLASS, SymbolKind::CLASS)
    } else if kind.is_a(ElementKind::Feature) {
        (CompletionItemKind::FIELD, SymbolKind::FIELD)
    } else {
        (CompletionItemKind::VALUE, SymbolKind::OBJECT)
    }
}

fn symbol_for(
    ws: &Workspace,
    elem: sysml_model::ElementId,
    text: &str,
    index: &LineIndex,
) -> Option<DocumentSymbol> {
    let model = ws.model();
    let name = model.name(elem)?.to_string();
    let (full, name_range) = ws.element_ranges(elem)?;
    let children: Vec<DocumentSymbol> = model
        .owned(elem)
        .iter()
        .filter_map(|c| symbol_for(ws, *c, text, index))
        .collect();
    #[allow(deprecated)]
    Some(DocumentSymbol {
        name,
        detail: Some(model.kind(elem).name().to_string()),
        kind: pictured(model.kind(elem)).1,
        tags: None,
        deprecated: None,
        range: index.range(text, full),
        selection_range: index.range(text, name_range),
        children: if children.is_empty() {
            None
        } else {
            Some(children)
        },
    })
}

fn ok_response<T: serde::Serialize>(id: RequestId, value: Option<T>) -> Response {
    Response::new_ok(
        id,
        value
            .map(|v| serde_json::to_value(v).unwrap_or(serde_json::Value::Null))
            .unwrap_or(serde_json::Value::Null),
    )
}

fn error_response(id: RequestId, err: impl std::fmt::Display) -> Response {
    Response::new_err(
        id,
        lsp_server::ErrorCode::InvalidParams as i32,
        err.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sequence the specification describes, and what a message
    /// that does not belong to the phase it arrives in does.
    ///
    /// Driven as a [`Session`] rather than over a connection: these are
    /// the answers a client gets for talking out of turn, and none of
    /// them needs a server that has read a library.
    fn waiting() -> (Session, Vec<Message>) {
        (Session::new(Files::handed()), Vec::new())
    }

    fn request(id: i32, method: &str, params: serde_json::Value) -> Message {
        Message::Request(Request {
            id: RequestId::from(id),
            method: method.into(),
            params,
        })
    }

    fn notification(method: &str) -> Message {
        Message::Notification(Notification {
            method: method.into(),
            params: serde_json::Value::Null,
        })
    }

    /// What the server said, as JSON -- which is how a client reads it,
    /// and how a test can look at it without a branch that never runs.
    fn said(out: &[Message]) -> Vec<serde_json::Value> {
        out.iter()
            .map(|message| serde_json::to_value(message).expect("a message is JSON"))
            .collect()
    }

    fn started(session: &mut Session, out: &mut Vec<Message>) {
        let params = serde_json::json!({
            "capabilities": {},
            "initializationOptions": { "noLibrary": true },
        });
        session.handle(request(1, "initialize", params), out);
        session.handle(notification("initialized"), out);
        out.clear();
    }

    #[test]
    fn a_question_before_the_handshake_is_answered_and_the_handshake_still_comes() {
        let (mut session, mut out) = waiting();
        let flow = session.handle(request(1, "shutdown", serde_json::Value::Null), &mut out);
        assert!(matches!(flow, Flow::Go), "{flow:?}");
        let answers = said(&out);
        assert_eq!(
            answers[0]["error"]["code"],
            lsp_server::ErrorCode::ServerNotInitialized as i32,
            "{answers:?}"
        );
        // and it is still a session: the client may say `initialize` now
        out.clear();
        started(&mut session, &mut out);
        assert!(!session.over());
    }

    #[test]
    fn a_notification_before_the_handshake_is_nobody_s_to_answer() {
        let (mut session, mut out) = waiting();
        let flow = session.handle(notification("$/setTrace"), &mut out);
        assert!(matches!(flow, Flow::Go));
        assert!(out.is_empty());
    }

    #[test]
    fn exit_before_the_handshake_is_the_end_of_it() {
        let (mut session, mut out) = waiting();
        let flow = session.handle(notification("exit"), &mut out);
        assert!(matches!(flow, Flow::Over(Err(_))), "{flow:?}");
        assert!(session.over());
    }

    /// This server asks the client nothing, so an answer is an answer to
    /// a question nobody asked.
    #[test]
    fn an_answer_nothing_asked_for_is_refused_before_the_handshake_and_ignored_after() {
        let answer = || {
            Message::Response(Response {
                id: RequestId::from(9),
                result: None,
                error: None,
            })
        };
        let (mut session, mut out) = waiting();
        assert!(matches!(
            session.handle(answer(), &mut out),
            Flow::Over(Err(_))
        ));

        let (mut session, mut out) = waiting();
        started(&mut session, &mut out);
        assert!(matches!(session.handle(answer(), &mut out), Flow::Go));
    }

    #[test]
    fn a_handshake_that_cannot_be_read_is_answered_before_the_session_ends() {
        let (mut session, mut out) = waiting();
        let flow = session.handle(request(1, "initialize", serde_json::json!(7)), &mut out);
        assert!(matches!(flow, Flow::Over(Err(_))), "{flow:?}");
        // a client left waiting hears nothing about why
        let answers = said(&out);
        assert!(answers[0]["error"]["message"].is_string(), "{answers:?}");
    }

    #[test]
    fn nothing_but_initialized_follows_the_handshake() {
        let (mut session, mut out) = waiting();
        let params = serde_json::json!({ "capabilities": {} });
        session.handle(request(1, "initialize", params), &mut out);
        let flow = session.handle(notification("textDocument/didSave"), &mut out);
        assert!(matches!(flow, Flow::Over(Err(_))), "{flow:?}");
    }

    #[test]
    fn nothing_but_exit_follows_a_shutdown() {
        let (mut session, mut out) = waiting();
        started(&mut session, &mut out);
        assert!(matches!(
            session.handle(request(2, "shutdown", serde_json::Value::Null), &mut out),
            Flow::Go
        ));
        let flow = session.handle(notification("initialized"), &mut out);
        assert!(matches!(flow, Flow::Over(Err(_))), "{flow:?}");
    }

    /// The project is what the client hands over, and it may change
    /// while the client is talking.
    #[test]
    fn a_handed_over_file_is_the_project_and_may_change() {
        let handed = |uri: &str, text: &str| {
            Message::Notification(Notification {
                method: "sysml/files".into(),
                params: serde_json::json!({ "files": [{ "uri": uri, "text": text }] }),
            })
        };
        let (mut session, mut out) = waiting();
        let params = serde_json::json!({
            "capabilities": {},
            "workspaceFolders": [{ "uri": "file:///m", "name": "m" }],
            "initializationOptions": {
                "noLibrary": true,
                "files": [{ "uri": "file:///m/parts.sysml", "text": "part def Wheel;\n" }],
            },
        });
        session.handle(request(1, "initialize", params), &mut out);
        session.handle(notification("initialized"), &mut out);
        out.clear();

        // a document naming what the file beside it declares
        session.handle(
            Message::Notification(Notification {
                method: "textDocument/didOpen".into(),
                params: serde_json::json!({ "textDocument": {
                    "uri": "file:///m/car.sysml",
                    "languageId": "sysml",
                    "version": 1,
                    "text": "part def Car {\n    part w : Wheel[4];\n}\n",
                }}),
            }),
            &mut out,
        );
        assert_eq!(said_about(&out, "file:///m/car.sysml").len(), 0, "{out:?}");

        // where that name is declared is a file the client never opened,
        // and the answer has to name it as the client would
        out.clear();
        session.handle(
            request(
                2,
                "textDocument/references",
                serde_json::json!({
                    "textDocument": { "uri": "file:///m/car.sysml" },
                    "position": { "line": 1, "character": 13 },
                    "context": { "includeDeclaration": true },
                }),
            ),
            &mut out,
        );
        let answers = said(&out);
        let places = answers[0]["result"].as_array().cloned().unwrap_or_default();
        assert!(
            places
                .iter()
                .any(|place| place["uri"] == "file:///m/parts.sysml"),
            "{answers:?}"
        );

        // the file changes under the client, which says so
        out.clear();
        session.handle(
            handed("file:///m/parts.sysml", "part def Wheeel;\n"),
            &mut out,
        );
        assert_eq!(said_about(&out, "file:///m/car.sysml").len(), 1, "{out:?}");

        // and handing over what the server already has costs nothing
        out.clear();
        session.handle(
            handed("file:///m/parts.sysml", "part def Wheeel;\n"),
            &mut out,
        );
        assert!(out.is_empty(), "{out:?}");
    }

    /// The diagnostics published about one document, as they were last
    /// published.
    fn said_about(out: &[Message], uri: &str) -> Vec<serde_json::Value> {
        said(out)
            .iter()
            .filter(|said| {
                said["method"] == "textDocument/publishDiagnostics" && said["params"]["uri"] == uri
            })
            .map(|said| {
                said["params"]["diagnostics"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .next_back()
            .unwrap_or_default()
    }

    /// A client that goes on talking to a session that is over is
    /// answered nothing, rather than panicked at.
    #[test]
    fn a_session_that_is_over_stays_over() {
        let (mut session, mut out) = waiting();
        started(&mut session, &mut out);
        session.handle(request(2, "shutdown", serde_json::Value::Null), &mut out);
        assert!(matches!(
            session.handle(notification("exit"), &mut out),
            Flow::Over(Ok(()))
        ));
        out.clear();
        let flow = session.handle(notification("textDocument/didSave"), &mut out);
        assert!(matches!(flow, Flow::Over(Ok(()))), "{flow:?}");
        assert!(out.is_empty());
    }
}

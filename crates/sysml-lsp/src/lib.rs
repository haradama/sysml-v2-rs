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

// Nothing here needs `unsafe`, and saying so is what keeps it that way.
#![forbid(unsafe_code)]
// Every public item carries a line saying what it is for. The two
// crates that do not turn this on are `sysml-syntax`, whose public
// surface is two hundred and seventy-nine syntax kinds whose names are
// the documentation, and `sysml-model`, whose is generated from the
// metamodel and would want the generator to write it.
#![warn(missing_docs)]
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::path::{Path, PathBuf};

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::Notification as _;
use lsp_types::{
    CompletionItemKind, CompletionOptions, Diagnostic, DiagnosticSeverity, DocumentSymbol,
    NumberOrString, OneOf, PublishDiagnosticsParams, ServerCapabilities, SignatureHelpOptions,
    SymbolKind, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};
use sysml_model::ElementKind;
use sysml_semantics::Workspace;
use sysml_syntax::{TextRange, TextSize};

mod line_index;
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
        signature_help_provider: Some(SignatureHelpOptions {
            trigger_characters: Some(vec!["(".into(), ",".into()]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Perform the initialize handshake on `connection` and serve until exit.
/// The standard-library directory comes from
/// `initializationOptions.libraryPath` or `SYSML_LIBRARY_PATH`; with
/// neither, the copy built into this binary is used, and
/// `initializationOptions.noLibrary` asks for none at all.
pub fn run(connection: &Connection) -> Result<(), Box<dyn Error + Sync + Send>> {
    let init_params = connection.initialize(serde_json::to_value(server_capabilities())?)?;
    let init: lsp_types::InitializeParams = serde_json::from_value(init_params)?;

    let option = |name: &str| {
        init.initialization_options
            .as_ref()
            .and_then(|o| o.get(name))
            .and_then(|v| v.as_str())
            .map(String::from)
    };
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
        .filter_map(|uri| uri.to_file_path().ok())
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
            if path.is_absolute() {
                vec![path.to_path_buf()]
            } else {
                // a relative entry is one per workspace folder, the way a
                // client writes `"vendor"` and means its own `vendor`
                roots.iter().map(|root| root.join(path)).collect()
            }
        })
        .collect();

    let mut server = Server::new(library, &roots, &excluded);
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
    server.serve(connection)
}

mod project;
mod requests;

/// The server's whole state: the layers a question is answered
/// against, the documents the client holds open, and what was last
/// published about each of them.
pub struct Server {
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
/// reloaded. The complaint goes to stderr, where a client collects a
/// server's log.
fn taken<T: serde::de::DeserializeOwned>(note: Notification) -> Option<T> {
    let method = note.method.clone();
    match serde_json::from_value(note.params) {
        Ok(params) => Some(params),
        Err(err) => {
            eprintln!("ignoring a `{method}` this server cannot read: {err}");
            None
        }
    }
}

impl Server {
    fn serve(&mut self, connection: &Connection) -> Result<(), Box<dyn Error + Sync + Send>> {
        for msg in &connection.receiver {
            match msg {
                Message::Request(req) => {
                    if connection.handle_shutdown(&req)? {
                        return Ok(());
                    }
                    let response = self.handle_request(&req);
                    connection.sender.send(Message::Response(response))?;
                }
                Message::Notification(note) => {
                    // The spec: a server told to leave before it was
                    // asked to shut down leaves with a failure, which is
                    // how a client tells a stop it asked for from one it
                    // did not. An `exit` that follows a `shutdown` never
                    // reaches here -- `handle_shutdown` takes it.
                    if note.method == lsp_types::notification::Exit::METHOD {
                        return Err("`exit` without a `shutdown` before it".into());
                    }
                    self.handle_notification(connection, note)?;
                }
                Message::Response(_) => {}
            }
        }
        Ok(())
    }

    fn handle_notification(
        &mut self,
        connection: &Connection,
        note: Notification,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        use lsp_types::notification::*;
        match note.method.as_str() {
            DidOpenTextDocument::METHOD => {
                let Some(params) = taken::<lsp_types::DidOpenTextDocumentParams>(note) else {
                    return Ok(());
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
                self.publish_diagnostics(connection, Some(&uri))?;
            }
            DidChangeTextDocument::METHOD => {
                let Some(params) = taken::<lsp_types::DidChangeTextDocumentParams>(note) else {
                    return Ok(());
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
                self.publish_diagnostics(connection, Some(&uri))?;
            }
            // a file written, renamed or deleted outside the editor
            // changes what the project is and what it says
            DidChangeWatchedFiles::METHOD => {
                let Some(_) = taken::<lsp_types::DidChangeWatchedFilesParams>(note) else {
                    return Ok(());
                };
                self.project = None;
                self.analysis = None;
                self.publish_diagnostics(connection, None)?;
            }
            DidCloseTextDocument::METHOD => {
                let Some(params) = taken::<lsp_types::DidCloseTextDocumentParams>(note) else {
                    return Ok(());
                };
                self.docs.remove(&params.text_document.uri);
                self.languages.remove(&params.text_document.uri);
                self.published.remove(&params.text_document.uri);
                // the buffer that spoke for the file is gone, so the
                // file on disk speaks for it again
                if file_of(&params.text_document.uri)
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
                let cleared = serde_json::to_value(cleared)?;
                let notification = lsp_server::Notification {
                    method: PublishDiagnostics::METHOD.into(),
                    params: cleared,
                };
                connection
                    .sender
                    .send(Message::Notification(notification))?;
                // what the closed file said reached the others: a
                // sibling that imported it was reading the buffer and
                // now reads the file, and nothing else would tell it so
                self.publish_diagnostics(connection, None)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Say what is wrong with each open document, where that has changed.
    ///
    /// `changed` is the document the client has just told this server about,
    /// which is published either way: a client that waits to hear back about
    /// the keystroke it sent has to hear something. Every keystroke in one
    /// file used to republish every open file, and an editor takes each of
    /// those apart and lays it out again.
    fn publish_diagnostics(
        &mut self,
        connection: &Connection,
        changed: Option<&Url>,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
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
            let params = serde_json::to_value(params)?;
            let notification = Notification {
                method: lsp_types::notification::PublishDiagnostics::METHOD.into(),
                params,
            };
            connection
                .sender
                .send(Message::Notification(notification))?;
        }
        Ok(())
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

/// The file a URL names, with every link resolved -- the one spelling
/// two clients' URLs for the same file agree on.
///
/// VSCode percent-encodes characters (`(`, `)`, `+`, `@`, `'`, `,`)
/// that this crate's URL type leaves alone, and an editor may resolve a
/// symlinked workspace folder where the scan did not. Compared as
/// strings, one file then looks like two, and the project reads it from
/// disk while a buffer declares it as well -- every name in it
/// colliding with itself.
fn file_of(url: &Url) -> Option<PathBuf> {
    url.to_file_path().ok().map(|path| resolved(&path))
}

/// The model files directly in `dir`, without the tree below it.
fn model_files_in(dir: &Path) -> Vec<PathBuf> {
    // a buffer can stand where no directory does: a file the editor
    // holds and has never written
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && sysml_semantics::is_model_file(path))
        .collect()
}

/// The file a workspace file's name stands for.
///
/// The project layer names a file by its path and the layer above names
/// a buffer by its URL, so a question about where a workspace file sits
/// has both to answer for.
fn file_named(name: &str) -> PathBuf {
    let path = name
        .strip_prefix("file://")
        .and_then(|_| Url::parse(name).ok())
        .and_then(|url| url.to_file_path().ok())
        .unwrap_or_else(|| PathBuf::from(name));
    resolved(&path)
}

/// `path` with every link resolved, or `path` itself when it names
/// nothing on disk -- a document the editor holds but has never written.
fn resolved(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
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

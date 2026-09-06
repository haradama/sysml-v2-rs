//! Language server for SysML v2 / KerML.
//!
//! Features: parse + name-resolution diagnostics, go-to-definition, hover
//! (kind, qualified name, documentation), document symbols, and whole-file
//! formatting. The standard library is preloaded from the directory given in
//! `initializationOptions.libraryPath` (or the `SYSML_LIBRARY_PATH`
//! environment variable) so references into the library resolve and
//! definitions inside it can be jumped to.
//!
//! Analysis is three layers, each rebuilt by a rarer event than the one
//! above it. The standard library is parsed and resolved once at startup.
//! On top of it sit the project's own files -- every `.sysml`/`.kerml`
//! under the workspace folders -- rebuilt when a document is opened or
//! closed, since a file open in the editor is read from its buffer rather
//! than from disk. On top of those sit the open buffers, rebuilt on every
//! keystroke. A model is written across several files that import each
//! other, so without the middle layer a file would resolve only against
//! whatever else happened to be open in a tab.
//!
//! Run the binary (`sysml-lsp`) over stdio, or drive [`run`] with an
//! in-memory [`Connection`] for testing.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::path::{Path, PathBuf};

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::Notification as _;
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionResponse, Diagnostic,
    DiagnosticSeverity, DocumentSymbol, GotoDefinitionResponse, Hover, HoverContents, Location,
    MarkupContent, MarkupKind, OneOf, ParameterInformation, ParameterLabel, Position,
    PublishDiagnosticsParams, ServerCapabilities, SignatureHelp, SignatureHelpOptions,
    SignatureInformation, SymbolInformation, SymbolKind, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextEdit, Url, WorkspaceEdit, WorkspaceSymbolResponse,
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
/// `initializationOptions.libraryPath` or `SYSML_LIBRARY_PATH`.
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

    let mut server = Server::new(library_path, &roots, &excluded);
    if let Some(command) = option("elkCommand") {
        server.elk_command = command;
    }
    let millis = init
        .initialization_options
        .as_ref()
        .and_then(|o| o.get("elkTimeoutMs"))
        .and_then(serde_json::Value::as_u64);
    if let Some(millis) = millis {
        server.elk_patience = std::time::Duration::from_millis(millis);
    }
    server.serve(connection)
}

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
    /// the ELK command a `layout: "elk"` diagram request runs
    elk_command: String,
    /// how long that command is given before this server draws the
    /// diagram itself
    elk_patience: std::time::Duration,
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

/// The parameters of a notification, or nothing when they are not what
/// the method says they are.
///
/// A client is not supposed to send such a thing, and one that does is
/// not a reason for this server to stop: an editor whose language server
/// exits mid-session leaves the file it was editing without diagnostics,
/// completion or anything else until the window is reloaded. The
/// complaint goes to stderr, which is where a client collects a server's
/// log.
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
    fn new(library_path: Option<String>, roots: &[PathBuf], excluded: &[PathBuf]) -> Server {
        let mut library = Workspace::new();
        let library_dir = library_path.map(PathBuf::from);
        if let Some(dir) = &library_dir {
            // a missing or unreadable library directory degrades gracefully
            // to an empty library
            let _ = library.load_dir(dir);
            // resolve the library once; the caches are cloned into every
            // layer above, so this cost is paid only at startup
            library.resolve_all();
        }
        Server {
            library,
            roots: roots.to_vec(),
            // the library is already loaded, so its own files are never
            // part of the project even when a workspace folder holds them
            excluded: library_dir
                .into_iter()
                .chain(excluded.iter().cloned())
                .collect(),
            project_files: Vec::new(),
            project: None,
            project_index: HashMap::new(),
            substituted: HashSet::new(),
            docs: HashMap::new(),
            languages: HashMap::new(),
            analysis: None,
            published: HashMap::new(),
            elk_command: "elkrs".to_string(),
            elk_patience: std::time::Duration::from_secs(10),
        }
    }

    /// Every model file under the workspace folders.
    ///
    /// Scanned again each time the project layer is built rather than
    /// once at startup: a file written, renamed or removed while the
    /// editor is open belongs to the project too, and one that was only
    /// ever scanned at startup dropped out of the analysis the moment
    /// the buffer holding it was closed.
    fn scan(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = self
            .roots
            .iter()
            .flat_map(|root| sysml_semantics::model_files(root))
            .filter(|path| !self.excluded.iter().any(|dir| path.starts_with(dir)))
            .collect();
        files.sort();
        files.dedup();
        files
    }

    /// Library + every project file whose buffer, if it has one, still
    /// says what the file says.
    ///
    /// A file an editor has changed is left out here and added by
    /// [`Server::analysis`] from its buffer instead, so that what the
    /// editor shows is what resolves -- and so that nothing is declared
    /// twice. A file merely opened is not left out: its buffer and the
    /// disk agree, so the layer below can speak for it, and rebuilding
    /// this layer on every open and close was the slowest thing this
    /// server did.
    fn project(&mut self) -> &Workspace {
        if self.project.is_none() {
            self.project_files = self.scan();
            // by the path with every link resolved, which is the one
            // spelling two clients' URLs for the same file agree on
            let open: HashMap<PathBuf, &String> = self
                .docs
                .iter()
                .filter_map(|(url, text)| Some((file_of(url)?, text)))
                .collect();
            let mut ws = self.library.clone();
            let mut added = Vec::new();
            let mut index = HashMap::new();
            let mut substituted = HashSet::new();
            for path in &self.project_files {
                // a listed file can still be unreadable: gone since the
                // scan, or never text in the first place
                let Ok(text) = std::fs::read_to_string(path) else {
                    continue;
                };
                let real = resolved(path);
                if open.get(&real).is_some_and(|buffer| **buffer != text) {
                    substituted.insert(real);
                    continue;
                }
                let file = ws.add_file(path.to_string_lossy(), &text);
                index.insert(real, file);
                added.push(file);
            }
            ws.resolve_files(&added);
            self.project_index = index;
            self.substituted = substituted;
            self.project = Some(ws);
        }
        self.project.as_ref().expect("just built")
    }

    /// Cached analysis of library + project + open documents (recomputed
    /// lazily after a document change).
    fn analysis(&mut self) -> &mut Analysis {
        if self.analysis.is_none() {
            let mut ws = self.project().clone();
            let mut doc_files = HashMap::new();
            let mut open = Vec::new();
            for (url, text) in &self.docs {
                // A document the layer below already read, byte for
                // byte, is that file rather than a second copy of it:
                // declared twice, every name in it collides with itself.
                match file_of(url).and_then(|path| self.project_index.get(&path)) {
                    Some(file) => doc_files.insert(url.clone(), *file),
                    None => {
                        let file = ws.add_file(self.workspace_name(url), text);
                        open.push(file);
                        doc_files.insert(url.clone(), file)
                    }
                };
            }
            ws.resolve_files(&open);
            self.analysis = Some(Analysis {
                ws,
                doc_files,
                indexed: HashMap::new(),
            });
        }
        self.analysis.as_mut().expect("just built")
    }

    /// Whether the project layer's copy of `uri` has stopped saying what
    /// the buffer says, which is the moment that layer has to be built
    /// again without it.
    fn contradicted(&self, uri: &Url) -> bool {
        let Some(file) = file_of(uri).and_then(|path| self.project_index.get(&path)) else {
            return false;
        };
        // the layer holds that file only while it is built, and a
        // document this server is not holding contradicts nothing
        self.project
            .as_ref()
            .zip(self.docs.get(uri))
            .is_some_and(|(ws, text)| ws.file_parse(*file).syntax().text() != text.as_str())
    }

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
                // something the file does not
                if self.contradicted(&uri) {
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
                connection
                    .sender
                    .send(Message::Notification(lsp_server::Notification {
                        method: PublishDiagnostics::METHOD.into(),
                        params: serde_json::to_value(cleared)?,
                    }))?;
                // what the closed file said reached the others: a
                // sibling that imported it was reading the buffer and
                // now reads the file, and nothing else would tell it so
                self.publish_diagnostics(connection, None)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Say what is wrong with each open document, where that has
    /// changed.
    ///
    /// `changed` is the document the client has just told this server
    /// about, which is published either way: a client that waits to
    /// hear back about the keystroke it sent has to hear something.
    /// Every keystroke in one file used to republish every open file,
    /// and an editor takes each of those apart and lays it out again.
    fn publish_diagnostics(
        &mut self,
        connection: &Connection,
        changed: Option<&Url>,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        let docs = self.docs.clone();
        let analysis = self.analysis();
        let mut fresh: Vec<(Url, Vec<Diagnostic>)> = Vec::new();
        for (url, file) in &analysis.doc_files {
            let text = &docs[url];
            let index = LineIndex::new(text);
            // an editor shows both halves: what does not parse yet is
            // not a reason to stop saying what does not resolve
            let found = analysis.ws.findings(&[*file]);
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

    fn definition(&mut self, uri: &Url, position: Position) -> Option<GotoDefinitionResponse> {
        let (file, offset) = self.locate(uri, position)?;
        let analysis = self.analysis();
        let reference = *analysis.ws.reference_at(file, offset)?;
        let target_file = analysis.ws.element_file(reference.target)?;
        let (_, name_range) = analysis.ws.element_ranges(reference.target)?;
        let location = self.location(target_file, name_range)?;
        Some(GotoDefinitionResponse::Scalar(location))
    }

    /// The document a file stands for: its URL, and the text the analysis
    /// was built from.
    ///
    /// The text comes from the workspace's own parse rather than from the
    /// buffer or the disk. Every range this server hands out was measured
    /// against that text, and a project file can change on disk between
    /// one analysis and the next: read afresh, an offset from the old text
    /// lands somewhere else in the new one -- inside a character, and the
    /// server died where it sliced.
    ///
    /// The path is answered as the workspace folder spelled it. Resolving
    /// it through its links would name the same file a second way, and the
    /// editor that followed the answer would open a file this server did
    /// not think it had.
    fn document(&mut self, file: usize) -> Option<(Url, String)> {
        let analysis = self.analysis();
        // An open document is answered in the spelling its own client
        // used. VSCode percent-encodes characters this crate's URL type
        // leaves alone, and an editor may have resolved a symlinked
        // folder the scan did not, so a URL built here from the path
        // would name a file the client does not think it has open.
        let spelled = analysis
            .doc_files
            .iter()
            .find(|(_, held)| **held == file)
            .map(|(url, _)| url.clone());
        let name = analysis.ws.file_name(file).to_string();
        let text = analysis.ws.file_parse(file).syntax().text().to_string();
        let url = match spelled {
            Some(url) => url,
            // anything else is named by its path
            None => Url::from_file_path(&name).ok()?,
        };
        Some((url, text))
    }

    /// Whether an edit may be written to the file. The project's own
    /// files may be, open or not; the standard library may not.
    fn writable(&mut self, file: usize) -> bool {
        let analysis = self.analysis();
        if analysis.doc_files.values().any(|held| *held == file) {
            return true;
        }
        let name = analysis.ws.file_name(file).to_string();
        self.project_files
            .iter()
            .any(|path| path.to_string_lossy() == name)
    }

    /// The dialect a document is written in.
    ///
    /// A URI that ends in a suffix this server knows settles it. One
    /// that does not -- an `untitled:` buffer, a file read out of
    /// version control -- has only the `languageId` its client declared
    /// when it opened the document, and taking KerML for SysML there
    /// reads `class` and `feature` as ordinary names.
    fn dialect_of(&self, uri: &Url) -> sysml_syntax::Dialect {
        match std::path::Path::new(uri.path())
            .extension()
            .and_then(|ext| ext.to_str())
        {
            Some(ext) if ext.eq_ignore_ascii_case("kerml") => sysml_syntax::Dialect::KerML,
            Some(ext) if ext.eq_ignore_ascii_case("sysml") => sysml_syntax::Dialect::SysML,
            _ => self
                .languages
                .get(uri)
                .copied()
                .unwrap_or(sysml_syntax::Dialect::SysML),
        }
    }

    /// What to call an open buffer inside the workspace.
    ///
    /// `Workspace::add_file` reads the dialect off the name it is given,
    /// and a buffer with no file behind it carries nothing it can read.
    /// A name ending in the right suffix is all it needs.
    fn workspace_name(&self, uri: &Url) -> String {
        let name = uri.to_string();
        match self.dialect_of(uri) {
            sysml_syntax::Dialect::KerML
                if sysml_syntax::Dialect::from_path(&name) != sysml_syntax::Dialect::KerML =>
            {
                format!("{name}.kerml")
            }
            _ => name,
        }
    }

    /// Location of a range within any workspace file (open doc, project
    /// file or library).
    fn location(&mut self, file: usize, range: sysml_syntax::TextRange) -> Option<Location> {
        let placed = self.placed(file)?;
        Some(Location {
            uri: placed.url.clone(),
            range: placed.index.range(&placed.text, range),
        })
    }

    /// A file with its lines found, kept for as long as the analysis it
    /// was read from.
    ///
    /// Find-references and workspace-symbol answer with a hundred places
    /// at a time, most of them in the same few files. Reading each file
    /// out of the tree and counting its lines again for every one of
    /// them was the greater part of what those two requests did.
    fn placed(&mut self, file: usize) -> Option<&Placed> {
        if !self.analysis().indexed.contains_key(&file) {
            let (url, text) = self.document(file)?;
            let index = LineIndex::new(&text);
            self.analysis()
                .indexed
                .insert(file, Placed { url, text, index });
        }
        self.analysis().indexed.get(&file)
    }

    /// The element a position points at: a resolved reference's target, or
    /// the declaration whose name covers the position.
    fn target_at(&mut self, uri: &Url, position: Position) -> Option<sysml_model::ElementId> {
        let (file, offset) = self.locate(uri, position)?;
        let analysis = self.analysis();
        analysis
            .ws
            .reference_at(file, offset)
            .map(|r| r.target)
            .or_else(|| analysis.ws.definition_at(file, offset))
    }

    fn references(
        &mut self,
        uri: &Url,
        position: Position,
        include_declaration: bool,
    ) -> Option<Vec<Location>> {
        let target = self.target_at(uri, position)?;
        let refs: Vec<(usize, sysml_syntax::TextRange)> = self
            .analysis()
            .ws
            .references_to(target)
            .map(|r| (r.file, r.range))
            .collect();
        let mut locations = Vec::new();
        if include_declaration {
            let analysis = self.analysis();
            let decl = analysis
                .ws
                .element_file(target)
                .zip(analysis.ws.element_ranges(target).map(|(_, name)| name));
            locations.extend(decl.and_then(|(file, range)| self.location(file, range)));
        }
        for (file, range) in refs {
            locations.extend(self.location(file, range));
        }
        Some(locations)
    }

    /// Renaming touches every file the workspace knows, so what makes it
    /// safe is checked across all of them before a single edit is
    /// offered: the new name has to be a name, nothing already visible
    /// where the declaration stands may answer to it, and what is being
    /// renamed has to be the project's rather than the library's.
    fn rename(
        &mut self,
        uri: &Url,
        position: Position,
        new_name: &str,
    ) -> Result<WorkspaceEdit, String> {
        // What may stand there is what this file's lexer reads as one
        // name and nothing else. Spelling the rule again here got it
        // wrong three ways: `\u{540d}\u{524d}` passed a Unicode
        // alphabetic test that the ASCII lexer refuses, so the rename
        // was applied and the file stopped lexing; a quoted `'two
        // words'` was refused although it is a name; and `frame` was
        // refused in KerML, where it is not a keyword.
        let dialect = self.dialect_of(uri);
        let (tokens, complaints) = sysml_syntax::lex_dialect(new_name, dialect);
        match tokens.as_slice() {
            [token]
                if complaints.is_empty()
                    && matches!(
                        token.kind,
                        sysml_syntax::SyntaxKind::IDENT
                            | sysml_syntax::SyntaxKind::UNRESTRICTED_NAME
                    ) => {}
            [token] if dialect.is_keyword(token.kind) => {
                return Err(format!("`{new_name}` is a keyword"))
            }
            _ => return Err(format!("`{new_name}` is not a name")),
        }

        let Some(found) = self.target_at(uri, position) else {
            return Err("there is nothing to rename here".to_string());
        };
        // A feature that declares no name answers to the name of what it
        // redefines or references, and there is nothing in it to rewrite.
        // Renaming is asked of the declaration that name came from, which
        // is what every mention of it -- this one included -- follows.
        let analysis = self.analysis();
        let Some(target) = analysis.ws.model().naming_element(found) else {
            return Err("that element declares no name to rename".to_string());
        };
        // A name reached through `alias X for Y;` resolves to Y, and
        // nothing records that X was the way in. Renaming X would move
        // the declaration and leave every use of it behind, so it is
        // refused rather than half done.
        if analysis.ws.is_alias(target) {
            return Err("renaming an alias would leave what uses it behind".to_string());
        }
        let decl_file = analysis
            .ws
            .element_file(target)
            .ok_or("that element belongs to no file".to_string())?;
        let (_, decl_range) = analysis
            .ws
            .element_ranges(target)
            .ok_or("that element declares no name to rename".to_string())?;
        let mut edits: Vec<(usize, sysml_syntax::TextRange)> = vec![(decl_file, decl_range)];
        edits.extend(
            analysis
                .ws
                .references_to(target)
                .map(|r| (r.file, r.name_range)),
        );
        // An unnamed redefinition answers to the name it redefines, so
        // what names it names this too -- `l.component` where `l` holds
        // a `:>> component`. Renaming the declaration without those is a
        // model that no longer resolves.
        for heir in analysis.ws.named_after(target) {
            edits.extend(
                analysis
                    .ws
                    .references_to(heir)
                    .map(|r| (r.file, r.name_range)),
            );
        }

        // the declaration must be somewhere the editor may write --
        // library elements are not
        if !self.writable(decl_file) {
            return Err("that element is declared outside the project".to_string());
        }

        // a name already visible where the declaration stands would
        // capture, or be captured by, the renamed one
        let start = decl_range.start();
        let taken = self
            .analysis()
            .ws
            .visible_names(decl_file, start)
            .into_iter()
            .any(|(name, _)| name == new_name);
        if taken {
            return Err(format!("`{new_name}` is already visible there"));
        }
        // The declaration's own scope is not the only place a capture
        // can happen. Every mention of the element stands in a scope of
        // its own, and one that already reaches something under the new
        // name binds to that instead the moment it is rewritten --
        // quietly, because the file still parses and still resolves.
        let wanted = [new_name.to_string()];
        for (file, range) in &edits {
            let (file, at) = (*file, range.start());
            if !looked_up_in_place(&self.analysis().ws, file, at) {
                continue;
            }
            let ws = &mut self.analysis().ws;
            let scope = ws.innermost_element(file, at);
            if ws
                .resolve_from(scope, &wanted)
                .is_some_and(|other| other != target)
            {
                return Err(format!(
                    "`{new_name}` is already visible where this one is used"
                ));
            }
        }

        // Every name this element answers to. The first is the one its
        // declaration spells and the one a rename replaces; the rest
        // stand for it some other way -- a short name, which renaming
        // the long one leaves standing.
        let model = self.analysis().ws.model();
        let names: Vec<String> = model
            .effective_name(target)
            .into_iter()
            .chain(model.effective_short_name(target))
            .map(String::from)
            .collect();
        let renamed = names.first().cloned().unwrap_or_default();

        // one text and one line index per file, however many of its
        // names this rename touches
        let mut by_file: HashMap<usize, Vec<TextRange>> = HashMap::new();
        for (file, range) in edits {
            by_file.entry(file).or_default().push(range);
        }
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        for (file, ranges) in by_file {
            let spelt: Vec<(TextRange, Option<TextRange>)> = {
                let ws = &self.analysis().ws;
                ranges
                    .into_iter()
                    .map(|range| (range, name_token(ws, file, range, &renamed)))
                    .collect()
            };
            // Every one of these is writable without being asked: what
            // holds a reference to a project element is a project file
            // or an open buffer, never the library, which resolves
            // before any of them exists and so cannot name one.
            let name = self.analysis().ws.file_name(file).to_string();
            let placed = self
                .placed(file)
                .ok_or_else(|| format!("`{name}` cannot be read"))?;
            let mut written = Vec::new();
            for (range, at) in spelt {
                let Some(at) = at else {
                    // Nothing there spells the name, so the model
                    // reaches this element some other way: by a short
                    // name, which this rename leaves standing, or
                    // through an `alias B for A;` whose own `for A`
                    // nothing records and no rename can follow.
                    let at = usize::from(range.start())..usize::from(range.end());
                    let spelled = sysml_syntax::unquote(&placed.text[at]);
                    if !names.contains(&spelled) {
                        return Err(format!(
                            "`{spelled}` stands for this one by a way no rename can follow"
                        ));
                    }
                    continue;
                };
                written.push(TextEdit {
                    range: placed.index.range(&placed.text, at),
                    new_text: new_name.to_string(),
                });
            }
            changes
                .entry(placed.url.clone())
                .or_default()
                .extend(written);
        }
        Ok(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        })
    }

    fn completion(&mut self, uri: &Url, position: Position) -> Option<CompletionResponse> {
        let (file, offset) = self.locate(uri, position)?;
        let analysis = self.analysis();
        let mut items: Vec<CompletionItem> = analysis
            .ws
            .visible_names(file, offset)
            .into_iter()
            .map(|(name, kind)| CompletionItem {
                label: name,
                kind: Some(pictured(kind).0),
                detail: Some(kind.name().to_string()),
                ..Default::default()
            })
            .collect();
        for (keyword, _, _) in sysml_syntax::KEYWORDS {
            items.push(CompletionItem {
                label: (*keyword).to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                ..Default::default()
            });
        }
        Some(CompletionResponse::Array(items))
    }

    fn hover(&mut self, uri: &Url, position: Position) -> Option<Hover> {
        let (file, offset) = self.locate(uri, position)?;
        let analysis = self.analysis();
        let reference = analysis.ws.reference_at(file, offset)?;
        let target = reference.target;
        let kind = analysis.ws.model().kind(target);
        let mut text = format!(
            "**{}** `{}`",
            kind.name(),
            analysis.ws.qualified_name_of(target)
        );
        if let Some(doc) = analysis.ws.documentation_of(target) {
            text.push_str("\n\n");
            text.push_str(&doc);
        }
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: text,
            }),
            range: None,
        })
    }

    fn document_symbols(&mut self, uri: &Url) -> Option<Vec<DocumentSymbol>> {
        let text = self.docs.get(uri)?.clone();
        let analysis = self.analysis();
        let file = *analysis.doc_files.get(uri)?;
        let index = LineIndex::new(&text);
        let symbols = analysis
            .ws
            .file_roots(file)
            .to_vec()
            .iter()
            .filter_map(|root| symbol_for(&analysis.ws, *root, &text, &index))
            .collect();
        Some(symbols)
    }

    fn format(&self, uri: &Url) -> Option<Vec<TextEdit>> {
        let text = self.docs.get(uri)?;
        let dialect = self.dialect_of(uri);
        // A tree recovered from broken source says where the formatter
        // should put things, but not where the modeller meant them: a
        // missing `;` joins two declarations, and formatting sets them
        // on one line as though that were the intent. The editor is
        // told there is nothing to do until the file parses.
        if !sysml_syntax::parse_dialect(text, dialect).ok() {
            return None;
        }
        let formatted = sysml_syntax::fmt::format_file(&self.workspace_name(uri), text);
        if formatted == *text {
            return Some(Vec::new());
        }
        let index = LineIndex::new(text);
        let full = TextRange::new(TextSize::from(0), TextSize::of(text.as_str()));
        Some(vec![TextEdit {
            range: index.range(text, full),
            new_text: formatted,
        }])
    }

    fn workspace_symbols(&mut self, query: &str) -> Option<WorkspaceSymbolResponse> {
        // the same search the MCP server's `library_search` runs, so an
        // exact match comes first here too
        let matches: Vec<(sysml_model::ElementId, String, sysml_model::ElementKind)> = {
            let analysis = self.analysis();
            analysis
                .ws
                .search_names(query, 128)
                .into_iter()
                .map(|id| {
                    let name = analysis.ws.model().name(id).unwrap_or_default().to_string();
                    (id, name, analysis.ws.model().kind(id))
                })
                .collect()
        };
        let mut symbols = Vec::new();
        for (id, name, kind) in matches {
            let place = {
                let analysis = self.analysis();
                analysis
                    .ws
                    .element_file(id)
                    .zip(analysis.ws.element_ranges(id).map(|(_, n)| n))
            };
            let location = place.and_then(|(file, range)| self.location(file, range));
            if let Some(location) = location {
                #[allow(deprecated)]
                symbols.push(SymbolInformation {
                    name,
                    kind: pictured(kind).1,
                    tags: None,
                    deprecated: None,
                    location,
                    container_name: None,
                });
            }
        }
        Some(WorkspaceSymbolResponse::Flat(symbols))
    }

    fn signature_help(&mut self, uri: &Url, position: Position) -> Option<SignatureHelp> {
        let (file, offset) = self.locate(uri, position)?;
        let analysis = self.analysis();
        let (target, active) = analysis.ws.callable_at(file, offset)?;
        let name = analysis.ws.model().name(target).unwrap_or("?").to_string();
        let params = analysis.ws.parameters_of(target);
        let label = format!("{name}({})", params.join(", "));
        Some(SignatureHelp {
            signatures: vec![SignatureInformation {
                label,
                documentation: analysis
                    .ws
                    .documentation_of(target)
                    .map(lsp_types::Documentation::String),
                parameters: Some(
                    params
                        .into_iter()
                        .map(|p| ParameterInformation {
                            label: ParameterLabel::Simple(p),
                            documentation: None,
                        })
                        .collect(),
                ),
                active_parameter: Some(active),
            }],
            active_signature: Some(0),
            active_parameter: Some(active),
        })
    }

    /// The diagram of one open document: its definitions and their
    /// relationships by default, the internal structure of one element
    /// with `view: "internal"`, the membership tree with `view: "browser"`.
    fn diagram(&mut self, params: &DiagramParams) -> Option<DiagramResult> {
        let uri = Url::parse(&params.uri).ok()?;
        let elk = params.layout.as_deref() == Some("elk");
        let command = self.elk_command.clone();
        let patience = self.elk_patience;
        let analysis = self.analysis();
        let file = *analysis.doc_files.get(&uri)?;
        let ws = &analysis.ws;
        let style = sysml_diagram::Style::default();
        // ELK when asked for and available, this crate's own layout
        // otherwise -- the preview always renders something
        let draw = |diagram: &sysml_diagram::Diagram| {
            if elk {
                match elk_within(diagram, &style, &command, patience) {
                    Some(Ok(svg)) => return svg,
                    Some(Err(error)) => {
                        eprintln!("sysml-lsp: falling back to the built-in layout: {error}")
                    }
                    None => eprintln!(
                        "sysml-lsp: ELK has not answered in {patience:?}; \
                         drawing with the built-in layout"
                    ),
                }
            }
            sysml_diagram::render(diagram, &style)
        };
        let svg = match params.view.as_deref() {
            Some("browser") => {
                let view = sysml_diagram::browser_view(ws.model(), ws.file_roots(file));
                sysml_diagram::render_browser(&view, &style)
            }
            Some("internal") => {
                let target = element_named(ws, file, params.element.as_deref()?)?;
                let diagram = sysml_diagram::interconnection_diagram(ws.model(), target);
                draw(&diagram)
            }
            _ => {
                let diagram = sysml_diagram::definition_diagram(ws.model(), ws.file_roots(file));
                let empty = diagram.nodes.is_empty();
                if empty {
                    // nothing definitional to draw: fall back to the tree,
                    // which can show any model at all
                    let view = sysml_diagram::browser_view(ws.model(), ws.file_roots(file));
                    sysml_diagram::render_browser(&view, &style)
                } else {
                    draw(&diagram)
                }
            }
        };
        Some(DiagramResult { svg })
    }

    fn locate(&mut self, uri: &Url, position: Position) -> Option<(usize, TextSize)> {
        let text = self.docs.get(uri)?.clone();
        let file = *self.analysis().doc_files.get(uri)?;
        let index = LineIndex::new(&text);
        Some((file, index.offset(&text, position)?))
    }
}

/// Where inside a recorded range the name being renamed is written.
///
/// What was recorded is not always the name alone. A mention can be a
/// feature chain -- `system.sub1`, one reference, ending on the step
/// that names what is being renamed -- and a declaration whose tree
/// holds no name node of its own is recorded as the whole of it:
/// `then fork F { ... }`, which the new name written over all of it
/// would replace, body and all.
fn name_token(ws: &Workspace, file: usize, range: TextRange, name: &str) -> Option<TextRange> {
    let syntax = ws.file_parse(file).syntax();
    let first = syntax.token_at_offset(range.start()).right_biased();
    std::iter::successors(first, sysml_syntax::SyntaxToken::next_token)
        .take_while(|token| range.contains_range(token.text_range()))
        .find(|token| sysml_syntax::unquote(token.text()) == name)
        .map(|token| token.text_range())
}

/// ELK's positions, or nothing when it has not answered in time.
///
/// The command is a child process that reads a graph and writes back
/// where things go. Run on this thread it can hang the whole server,
/// which has only the one, and an editor whose language server has
/// stopped answering has stopped doing everything -- no diagnostics, no
/// completion, no navigation, in every file at once. It is run beside
/// the loop instead and given only so long; what it does after that is
/// its own business, and the diagram is drawn here meanwhile.
fn elk_within(
    diagram: &sysml_diagram::Diagram,
    style: &sysml_diagram::Style,
    command: &str,
    patience: std::time::Duration,
) -> Option<Result<String, sysml_diagram::ElkError>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let (diagram, style, command) = (diagram.clone(), *style, command.to_string());
    std::thread::spawn(move || {
        let _ = sender.send(sysml_diagram::render_with_elk(&diagram, &style, &command));
    });
    receiver.recv_timeout(patience).ok()
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
    /// `builtin` (default) or `elk` -- who decides the positions.
    layout: Option<String>,
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

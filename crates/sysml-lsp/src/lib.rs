//! Language server for SysML v2 / KerML.
//!
//! Features: parse + name-resolution diagnostics, go-to-definition, hover
//! (kind, qualified name, documentation), document symbols, and whole-file
//! formatting. The standard library is preloaded from the directory given in
//! `initializationOptions.libraryPath` (or the `SYSML_LIBRARY_PATH`
//! environment variable) so references into the library resolve and
//! definitions inside it can be jumped to.
//!
//! The whole workspace is re-analyzed per change — parsing and resolving the
//! standard library plus open documents takes well under a second in release
//! builds, which keeps the server simple and always consistent.
//!
//! Run the binary (`sysml-lsp`) over stdio, or drive [`run`] with an
//! in-memory [`Connection`] for testing.

use std::collections::HashMap;
use std::error::Error;

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

    let library_path = init
        .initialization_options
        .as_ref()
        .and_then(|o| o.get("libraryPath"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| std::env::var("SYSML_LIBRARY_PATH").ok());

    let mut server = Server::new(library_path);
    server.serve(connection)
}

pub struct Server {
    /// library workspace, parsed and fully resolved once at startup; each
    /// re-analysis clones it (including all resolution caches) and only
    /// adds + resolves the open documents
    base: Workspace,
    /// open documents
    docs: HashMap<Url, String>,
    /// cached analysis, invalidated on document changes
    analysis: Option<Analysis>,
}

/// One analysis pass over the library + all open documents.
struct Analysis {
    ws: Workspace,
    doc_files: HashMap<Url, usize>,
}

impl Server {
    fn new(library_path: Option<String>) -> Server {
        let mut base = Workspace::new();
        if let Some(dir) = library_path {
            // a missing or unreadable library directory degrades gracefully
            // to an empty library
            let _ = base.load_dir(std::path::Path::new(&dir));
            // resolve the library once; the caches are cloned into every
            // per-change analysis, so this cost is paid only at startup
            base.resolve_all();
        }
        Server {
            base,
            docs: HashMap::new(),
            analysis: None,
        }
    }

    /// Cached analysis of library + open documents (recomputed lazily
    /// after a document change).
    fn analysis(&mut self) -> &mut Analysis {
        if self.analysis.is_none() {
            let mut ws = self.base.clone();
            let mut doc_files = HashMap::new();
            for (url, text) in &self.docs {
                let idx = ws.add_file(url.to_string(), text);
                doc_files.insert(url.clone(), idx);
            }
            let open: Vec<usize> = doc_files.values().copied().collect();
            ws.resolve_files(&open);
            self.analysis = Some(Analysis { ws, doc_files });
        }
        self.analysis.as_mut().expect("just built")
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
                let params: lsp_types::DidOpenTextDocumentParams =
                    serde_json::from_value(note.params)?;
                self.docs
                    .insert(params.text_document.uri.clone(), params.text_document.text);
                self.analysis = None;
                self.publish_diagnostics(connection)?;
            }
            DidChangeTextDocument::METHOD => {
                let params: lsp_types::DidChangeTextDocumentParams =
                    serde_json::from_value(note.params)?;
                if let Some(text) = self.docs.get_mut(&params.text_document.uri) {
                    for change in params.content_changes {
                        apply_change(text, change);
                    }
                }
                self.analysis = None;
                self.publish_diagnostics(connection)?;
            }
            DidCloseTextDocument::METHOD => {
                let params: lsp_types::DidCloseTextDocumentParams =
                    serde_json::from_value(note.params)?;
                self.docs.remove(&params.text_document.uri);
                self.analysis = None;
            }
            _ => {}
        }
        Ok(())
    }

    fn publish_diagnostics(
        &mut self,
        connection: &Connection,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        let docs = self.docs.clone();
        let analysis = self.analysis();
        for (url, file) in &analysis.doc_files {
            let text = &docs[url];
            let index = LineIndex::new(text);
            let mut diagnostics = Vec::new();
            for err in analysis.ws.file_parse(*file).errors() {
                diagnostics.push(Diagnostic {
                    range: index.range(text, err.range),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("sysml".into()),
                    message: err.message.clone(),
                    ..Default::default()
                });
            }
            for unresolved in analysis.ws.unresolved() {
                if unresolved.file == *file {
                    diagnostics.push(Diagnostic {
                        range: index.range(text, unresolved.range),
                        severity: Some(DiagnosticSeverity::WARNING),
                        source: Some("sysml".into()),
                        message: format!("unresolved reference `{}`", unresolved.name),
                        ..Default::default()
                    });
                }
            }
            let params = PublishDiagnosticsParams {
                uri: url.clone(),
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
        match req.method.as_str() {
            GotoDefinition::METHOD => {
                let params: lsp_types::GotoDefinitionParams =
                    match serde_json::from_value(req.params.clone()) {
                        Ok(p) => p,
                        Err(e) => return error_response(id, e),
                    };
                let position = params.text_document_position_params;
                ok_response(
                    id,
                    self.definition(&position.text_document.uri, position.position),
                )
            }
            HoverRequest::METHOD => {
                let params: lsp_types::HoverParams =
                    match serde_json::from_value(req.params.clone()) {
                        Ok(p) => p,
                        Err(e) => return error_response(id, e),
                    };
                let position = params.text_document_position_params;
                ok_response(
                    id,
                    self.hover(&position.text_document.uri, position.position),
                )
            }
            DocumentSymbolRequest::METHOD => {
                let params: lsp_types::DocumentSymbolParams =
                    match serde_json::from_value(req.params.clone()) {
                        Ok(p) => p,
                        Err(e) => return error_response(id, e),
                    };
                ok_response(id, self.document_symbols(&params.text_document.uri))
            }
            References::METHOD => {
                let params: lsp_types::ReferenceParams =
                    match serde_json::from_value(req.params.clone()) {
                        Ok(p) => p,
                        Err(e) => return error_response(id, e),
                    };
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
                let params: lsp_types::RenameParams =
                    match serde_json::from_value(req.params.clone()) {
                        Ok(p) => p,
                        Err(e) => return error_response(id, e),
                    };
                let position = params.text_document_position;
                ok_response(
                    id,
                    self.rename(
                        &position.text_document.uri,
                        position.position,
                        &params.new_name,
                    ),
                )
            }
            Completion::METHOD => {
                let params: lsp_types::CompletionParams =
                    match serde_json::from_value(req.params.clone()) {
                        Ok(p) => p,
                        Err(e) => return error_response(id, e),
                    };
                let position = params.text_document_position;
                ok_response(
                    id,
                    self.completion(&position.text_document.uri, position.position),
                )
            }
            WorkspaceSymbolRequest::METHOD => {
                let params: lsp_types::WorkspaceSymbolParams =
                    match serde_json::from_value(req.params.clone()) {
                        Ok(p) => p,
                        Err(e) => return error_response(id, e),
                    };
                ok_response(id, self.workspace_symbols(&params.query))
            }
            SignatureHelpRequest::METHOD => {
                let params: lsp_types::SignatureHelpParams =
                    match serde_json::from_value(req.params.clone()) {
                        Ok(p) => p,
                        Err(e) => return error_response(id, e),
                    };
                let position = params.text_document_position_params;
                ok_response(
                    id,
                    self.signature_help(&position.text_document.uri, position.position),
                )
            }
            Formatting::METHOD => {
                let params: lsp_types::DocumentFormattingParams =
                    match serde_json::from_value(req.params.clone()) {
                        Ok(p) => p,
                        Err(e) => return error_response(id, e),
                    };
                ok_response(id, self.format(&params.text_document.uri))
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

    /// Location of a range within any workspace file (open doc or library).
    fn location(&mut self, file: usize, range: sysml_syntax::TextRange) -> Option<Location> {
        let name = self.analysis().ws.file_name(file).to_string();
        let open_doc = Url::parse(&name)
            .ok()
            .and_then(|url| self.docs.get(&url).map(|text| (url, text.clone())));
        if let Some((url, text)) = open_doc {
            let index = LineIndex::new(&text);
            return Some(Location {
                uri: url,
                range: index.range(&text, range),
            });
        }
        let url = Url::from_file_path(std::path::Path::new(&name).canonicalize().ok()?).ok()?;
        let text = std::fs::read_to_string(&name).ok()?;
        let index = LineIndex::new(&text);
        Some(Location {
            uri: url,
            range: index.range(&text, range),
        })
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

    fn rename(&mut self, uri: &Url, position: Position, new_name: &str) -> Option<WorkspaceEdit> {
        let target = self.target_at(uri, position)?;
        let analysis = self.analysis();
        // the declaration must live in an open document — library elements
        // cannot be renamed
        let decl_file = analysis.ws.element_file(target)?;
        let decl_name = analysis.ws.file_name(decl_file).to_string();
        Url::parse(&decl_name)
            .ok()
            .filter(|url| analysis.doc_files.contains_key(url))?;
        let (_, decl_range) = analysis.ws.element_ranges(target)?;
        let mut edits: Vec<(usize, sysml_syntax::TextRange)> = vec![(decl_file, decl_range)];
        edits.extend(
            analysis
                .ws
                .references_to(target)
                .map(|r| (r.file, r.name_range)),
        );

        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        for (file, range) in edits {
            let name = self.analysis().ws.file_name(file).to_string();
            let url = Url::parse(&name).ok()?;
            let text = self.docs.get(&url)?.clone();
            let index = LineIndex::new(&text);
            changes.entry(url).or_default().push(TextEdit {
                range: index.range(&text, range),
                new_text: new_name.to_string(),
            });
        }
        Some(WorkspaceEdit {
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
                kind: Some(completion_kind(kind)),
                detail: Some(kind.name().to_string()),
                ..Default::default()
            })
            .collect();
        for keyword in KEYWORDS {
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
        let formatted = sysml_syntax::fmt::format_file(uri.as_str(), text);
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
        let needle = query.to_lowercase();
        let matches: Vec<(sysml_model::ElementId, String, sysml_model::ElementKind)> = {
            let analysis = self.analysis();
            let mut found: Vec<_> = analysis
                .ws
                .named_elements()
                .filter(|(_, name)| needle.is_empty() || name.to_lowercase().contains(&needle))
                .map(|(id, name)| (id, name.to_string(), analysis.ws.model().kind(id)))
                .collect();
            found.sort_by(|a, b| a.1.len().cmp(&b.1.len()).then(a.1.cmp(&b.1)));
            found.truncate(128);
            found
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
                    kind: symbol_kind(kind),
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

    fn locate(&mut self, uri: &Url, position: Position) -> Option<(usize, TextSize)> {
        let text = self.docs.get(uri)?.clone();
        let file = *self.analysis().doc_files.get(uri)?;
        let index = LineIndex::new(&text);
        Some((file, index.offset(&text, position)?))
    }
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

const KEYWORDS: &[&str] = &[
    "about",
    "abstract",
    "action",
    "attribute",
    "calc",
    "case",
    "connect",
    "connection",
    "constraint",
    "def",
    "doc",
    "end",
    "enum",
    "exhibit",
    "flow",
    "import",
    "in",
    "interface",
    "item",
    "occurrence",
    "out",
    "package",
    "part",
    "perform",
    "port",
    "private",
    "public",
    "redefines",
    "ref",
    "requirement",
    "satisfy",
    "specializes",
    "state",
    "subject",
    "subsets",
    "transition",
    "verification",
    "view",
    "viewpoint",
];

fn completion_kind(kind: ElementKind) -> CompletionItemKind {
    if kind.is_a(ElementKind::Package) || kind == ElementKind::Namespace {
        CompletionItemKind::MODULE
    } else if kind.is_a(ElementKind::Classifier) {
        CompletionItemKind::CLASS
    } else if kind.is_a(ElementKind::Feature) {
        CompletionItemKind::FIELD
    } else {
        CompletionItemKind::VALUE
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
        kind: symbol_kind(model.kind(elem)),
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

fn symbol_kind(kind: ElementKind) -> SymbolKind {
    if kind.is_a(ElementKind::Package) || kind == ElementKind::Namespace {
        SymbolKind::MODULE
    } else if kind.is_a(ElementKind::Classifier) {
        SymbolKind::CLASS
    } else if kind.is_a(ElementKind::Feature) {
        SymbolKind::FIELD
    } else {
        SymbolKind::OBJECT
    }
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

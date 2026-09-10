//! What the client asks, and what is looked up to answer it.
//!
//! Every one of these turns a position in a document into a question for
//! the resolver and its answer back into a range in a document. The two
//! halves are not the same coordinates: the client counts UTF-16 code
//! units from a line, and the model counts bytes from the start of the
//! file, so a document with a wide character in it lands in the wrong
//! place unless every crossing goes through `placed`.

use std::collections::HashMap;

use lsp_types::*;
use sysml_syntax::TextSize;

use crate::*;

impl Server {
    pub(crate) fn definition(
        &mut self,
        uri: &Url,
        position: Position,
    ) -> Option<GotoDefinitionResponse> {
        let (file, offset) = self.locate(uri, position)?;
        let analysis = self.analysis();
        let reference = *analysis.ws.reference_at(file, offset)?;
        let target_file = analysis.ws.element_file(reference.target)?;
        let (_, name_range) = analysis.ws.element_ranges(reference.target)?;
        let location = self.location(target_file, name_range)?;
        Some(GotoDefinitionResponse::Scalar(location))
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
    pub(crate) fn references(
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
    pub(crate) fn rename(
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
    pub(crate) fn completion(
        &mut self,
        uri: &Url,
        position: Position,
    ) -> Option<CompletionResponse> {
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
    pub(crate) fn hover(&mut self, uri: &Url, position: Position) -> Option<Hover> {
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
    pub(crate) fn document_symbols(&mut self, uri: &Url) -> Option<Vec<DocumentSymbol>> {
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
    pub(crate) fn format(&self, uri: &Url) -> Option<Vec<TextEdit>> {
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
    pub(crate) fn workspace_symbols(&mut self, query: &str) -> Option<WorkspaceSymbolResponse> {
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
    pub(crate) fn signature_help(
        &mut self,
        uri: &Url,
        position: Position,
    ) -> Option<SignatureHelp> {
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
    ///
    /// `scope: "directory"` draws the document's directory and the tree
    /// below it rather than the document alone.
    pub(crate) fn diagram(&mut self, params: &DiagramParams) -> Option<DiagramResult> {
        let uri = Url::parse(&params.uri).ok()?;
        let elk = params.layout.as_deref() == Some("elk");
        let command = self.elk_command.clone();
        let patience = self.elk_patience;
        let directory = params.scope.as_deref() == Some("directory");
        let within = directory
            .then(|| file_of(&uri))
            .flatten()
            .and_then(|path| path.parent().map(Path::to_path_buf));
        let analysis = self.analysis();
        let file = *analysis.doc_files.get(&uri)?;
        let ws = &analysis.ws;
        // What the drawing is of. A model is commonly written across
        // several files -- one declaring the definitions, another the
        // usages of them -- and a drawing of one file alone is then half
        // a model, or, where the file declares only usages, an empty
        // page. `scope: "directory"` draws the lot.
        let roots: Vec<sysml_model::ElementId> = match &within {
            Some(dir) => (0..ws.file_count())
                .filter(|&other| other == file || file_named(ws.file_name(other)).starts_with(dir))
                .flat_map(|other| ws.file_roots(other).iter().copied())
                .collect(),
            None => ws.file_roots(file).to_vec(),
        };
        let roots = roots.as_slice();
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
                let view = sysml_diagram::browser_view(ws.model(), roots);
                sysml_diagram::render_browser(&view, &style)
            }
            Some("internal") => {
                let target = element_named(ws, file, params.element.as_deref()?)?;
                let diagram = sysml_diagram::interconnection_diagram(ws.model(), target);
                draw(&diagram)
            }
            _ => {
                let diagram = sysml_diagram::definition_diagram(ws.model(), roots);
                let empty = diagram.nodes.is_empty();
                if empty {
                    // nothing definitional to draw -- a file of usages
                    // read on its own -- so fall back to the tree, which
                    // can show any model at all
                    let view = sysml_diagram::browser_view(ws.model(), roots);
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

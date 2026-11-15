//! What the client asks, and what is looked up to answer it.
//!
//! Every one of these turns a position in a document into a question for
//! the resolver and its answer back into a range in a document. The two
//! halves are not the same coordinates: the client counts UTF-16 code
//! units from a line, and the model counts bytes from the start of the
//! file, so a document with a wide character in it lands in the wrong
//! place unless every crossing goes through `placed`.

use std::collections::{HashMap, HashSet};

use lsp_types::*;
use sysml_syntax::TextSize;

use crate::*;

/// How many spellings are worth offering for one name. A list longer
/// than this is a list nobody reads to the end of, and the nearest are
/// first.
const ENOUGH: usize = 4;

/// Whether two ranges touch. A client asking about a diagnostic sends
/// that diagnostic's own range, and one asking about a line sends the
/// line.
fn overlaps(one: TextRange, other: TextRange) -> bool {
    one.start() <= other.end() && other.start() <= one.end()
}

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
        self.analysis().ws.target_at(file, offset)
    }
    /// Every place the element under the cursor is named, by its name
    /// alone.
    ///
    /// The same places a rename rewrites, walked the same way. This
    /// answered with the whole of `P::Wheel` or `a.b.c` where a rename
    /// replaces `Wheel` or `c`, and left out what reaches the element
    /// through a feature that borrows its name -- `l.component` where `l`
    /// holds a `:>> component` -- which a rename does rewrite. Asked
    /// before a rename, it under-reported what the rename would touch.
    pub(crate) fn references(
        &mut self,
        uri: &Url,
        position: Position,
        include_declaration: bool,
    ) -> Option<Vec<Location>> {
        let found = self.target_at(uri, position)?;
        let ws = &self.analysis().ws;
        let target = ws.model().naming_element(found).unwrap_or(found);
        let declared = include_declaration
            .then(|| ws.declared_at(target))
            .flatten();
        let places: Vec<(usize, TextRange)> =
            declared.into_iter().chain(ws.mentions(target)).collect();
        let mut locations = Vec::new();
        for (file, range) in places {
            locations.extend(self.location(file, range));
        }
        Some(locations)
    }
    /// Renaming touches every file the workspace knows, so what makes it
    /// safe is checked across all of them before a single edit is
    /// offered: the new name has to be a name, nothing already visible
    /// where the declaration stands may answer to it, and what is being
    /// renamed has to be the project's rather than the library's.
    ///
    /// The walk and each of those refusals are the workspace's, shared
    /// with the MCP server's `rename`; what is decided here is only which
    /// files this editor may write and how a range is counted in it.
    pub(crate) fn rename(
        &mut self,
        uri: &Url,
        position: Position,
        new_name: &str,
    ) -> Result<WorkspaceEdit, String> {
        let dialect = self.dialect_of(uri);
        // Refused before anything is looked up, so a name that is no name
        // is said to be one whatever the cursor is on.
        if let Some(why) = sysml_semantics::refuse_name(new_name, dialect) {
            return Err(why);
        }
        let Some((file, offset)) = self.locate(uri, position) else {
            return Err("there is nothing to rename here".to_string());
        };
        let renaming = self
            .analysis()
            .ws
            .prepare_rename(file, offset, new_name, dialect)?;

        // the declaration must be somewhere the editor may write --
        // library elements are not
        if !self.writable(renaming.declared.0) {
            return Err("that element is declared outside the project".to_string());
        }

        // An open document is in the notation its client declared for
        // it, which its URL need not say; every other file is named by
        // its path, whose suffix does.
        let opened: Vec<(usize, Url)> = self
            .analysis()
            .doc_files
            .iter()
            .map(|(url, file)| (*file, url.clone()))
            .collect();
        let declared: HashMap<usize, sysml_syntax::Dialect> = opened
            .into_iter()
            .map(|(file, url)| (file, self.dialect_of(&url)))
            .collect();
        let edits = self.analysis().ws.rename_edits(&renaming, |file, name| {
            declared
                .get(&file)
                .copied()
                .unwrap_or_else(|| sysml_syntax::Dialect::from_path(name))
        })?;

        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        for (file, written) in edits {
            // Every one of these is writable without being asked: what
            // holds a reference to a project element is a project file
            // or an open buffer, never the library, which resolves
            // before any of them exists and so cannot name one.
            let name = self.analysis().ws.file_name(file).to_string();
            let placed = self
                .placed(file)
                .ok_or_else(|| format!("`{name}` cannot be read"))?;
            let written: Vec<TextEdit> = written
                .into_iter()
                .map(|edit| TextEdit {
                    range: placed.index.range(&placed.text, edit.range),
                    new_text: edit.new_text,
                })
                .collect();
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
        let dialect = self.dialect_of(uri);
        let analysis = self.analysis();
        // What is written inside a comment, a string or a quoted name is
        // text rather than the model, and every keyword offered there
        // was one more line to scroll past.
        if written_inside_a_token(&analysis.ws, file, offset) {
            return Some(CompletionResponse::Array(Vec::new()));
        }
        let mut items: Vec<CompletionItem> = analysis
            .ws
            .visible_names(file, offset)
            .into_iter()
            .map(|(name, kind)| CompletionItem {
                // `Wheel Unit` is offered by its name and inserted the way
                // it has to be written: the bare label wrote `part z :
                // Wheel Unit`, which does not parse
                insert_text: Some(sysml_syntax::quote(&name)).filter(|it| *it != name),
                label: name,
                kind: Some(pictured(kind).0),
                detail: Some(kind.name().to_string()),
                ..Default::default()
            })
            .collect();
        // after `P::` or `x.` only a name can follow
        let keywords = if analysis.ws.follows_qualifier(file, offset) {
            &[][..]
        } else {
            sysml_syntax::KEYWORDS
        };
        // and a keyword of the other notation is a name in this one --
        // `part` offered in a KerML file is not something it can say
        let keywords = keywords
            .iter()
            .filter(|(_, kind, _)| dialect.is_keyword(*kind));
        for (keyword, _, _) in keywords {
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
    pub(crate) fn format(&self, uri: &Url, options: &FormattingOptions) -> Option<Vec<TextEdit>> {
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
        let formatted = indented(
            &sysml_syntax::fmt::format_with(text, dialect, self.layout),
            options,
        );
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
        // As many as an editor's picker shows.
        const SHOWN: usize = 128;
        // The same search the MCP server's `library_search` runs, so an
        // exact match comes first here too -- every match of it, and the
        // first that can be shown kept. Cut to the limit first, the hits
        // that have no place to go to were dropped after the cut: the
        // library built into this binary is no file, and a query as short
        // as `e` filled the limit with it and answered with one of the
        // project's own definitions where there were three.
        //
        // The library is a place to go to now that the copy built in is
        // written out where there are files, and its names would crowd
        // the project's out the same way: what the project and its open
        // buffers declare comes first, in its rank, and the library's
        // after it, in its own.
        let library_files = self.library.file_count();
        let mut ranked: Vec<sysml_model::ElementId> =
            self.analysis().ws.search_names(query, usize::MAX);
        {
            let ws = &self.analysis().ws;
            ranked.sort_by_key(|id| {
                ws.element_file(*id)
                    .is_some_and(|file| file < library_files)
            });
        }
        // a file with no place is asked once, not once per element in it
        let mut nowhere: HashSet<usize> = HashSet::new();
        let mut symbols = Vec::new();
        for id in ranked {
            if symbols.len() == SHOWN {
                break;
            }
            let place = {
                let ws = &self.analysis().ws;
                ws.element_file(id)
                    .zip(ws.element_ranges(id).map(|(_, n)| n))
            };
            let Some((file, range)) = place.filter(|(file, _)| !nowhere.contains(file)) else {
                continue;
            };
            let Some(location) = self.location(file, range) else {
                nowhere.insert(file);
                continue;
            };
            let model = self.analysis().ws.model();
            let name = crate::shown_name(model.name(id).unwrap_or_default());
            let kind = model.kind(id);
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
    /// What a name that resolved to nothing might have meant, as edits a
    /// client can apply.
    ///
    /// `Workspace::suggestions` tells two mistakes apart and this keeps
    /// them apart: a name the workspace declares somewhere is a right one
    /// that nothing brought into scope, and a name nothing declares is a
    /// wrong one. Either way the edit is a name spelled from the root,
    /// which is a name that resolves wherever it is written.
    ///
    /// Asked rather than published. The walk behind it is over every
    /// declared name -- sixty thousand with the standard library loaded
    /// -- which is nothing once, on the click that asks for it, and a
    /// stutter on every keystroke.
    pub(crate) fn code_actions(&mut self, uri: &Url, range: Range) -> Option<CodeActionResponse> {
        let text = self.docs.get(uri)?.clone();
        let index = LineIndex::new(&text);
        // A range written end first is the same stretch of text, as
        // `apply_change` reads one; `TextRange::new` asserts the order,
        // and a client that sent it backwards took the server down.
        let (start, end) = (
            index.offset(&text, range.start)?,
            index.offset(&text, range.end)?,
        );
        let asked = TextRange::new(start.min(end), start.max(end));
        let file = *self.analysis().doc_files.get(uri)?;
        let ws = &self.analysis().ws;

        // Every name that resolved to nothing under what the client
        // pointed at. A client points at a diagnostic's range, which is
        // one of these exactly; one that points at a line gets the lot.
        let wanted: Vec<sysml_semantics::Unresolved> = ws
            .unresolved()
            .iter()
            .filter(|it| it.file == file && overlaps(it.range, asked))
            .cloned()
            .collect();
        if wanted.is_empty() {
            return Some(Vec::new());
        }
        // one walk for all of them, which is what `suggestions` asks of
        // whoever calls it
        let names: Vec<String> = wanted.iter().map(|it| it.name.clone()).collect();
        let suggested = self.analysis().ws.suggestions(&names);

        let mut actions = Vec::new();
        for (missing, may) in wanted.iter().zip(suggested) {
            // a name that reaches the wrong kind of thing is spelt right
            // and in scope: putting another spelling in its place only
            // names the same thing again
            if missing.wrong_kind.is_some() {
                continue;
            }
            // Both lists are spelled from the root, so either replaces
            // the whole of what was written -- `Parts::Wheeel` becomes
            // `Parts::Wheel` without the qualifier being put back by
            // hand. The two read differently because they are not the
            // same mistake, and which one it is, is the useful half of
            // the answer: a name nothing declares was mistyped, and a
            // name something declares was simply never in scope here.
            let near = may
                .near
                .iter()
                .take(ENOUGH)
                .map(|spelling| (format!("did you mean `{spelling}`?"), spelling));
            let elsewhere = may
                .elsewhere
                .iter()
                .take(ENOUGH)
                .map(|spelling| (format!("`{spelling}`, declared elsewhere"), spelling));
            for (title, spelling) in near.chain(elsewhere) {
                let spelling = spelling.clone();
                actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                    title,
                    kind: Some(CodeActionKind::QUICKFIX),
                    edit: Some(WorkspaceEdit {
                        changes: Some(HashMap::from([(
                            uri.clone(),
                            vec![TextEdit {
                                range: index.range(&text, missing.range),
                                new_text: spelling,
                            }],
                        )])),
                        ..Default::default()
                    }),
                    ..Default::default()
                }));
            }
        }
        Some(actions)
    }

    /// The diagram of one open document: its definitions and their
    /// relationships by default, the internal structure of one element
    /// with `view: "internal"`, the membership tree with `view: "browser"`.
    ///
    /// `scope: "directory"` draws the document's directory and the tree
    /// below it rather than the document alone.
    pub(crate) fn diagram(&mut self, params: &DiagramParams) -> Option<DiagramResult> {
        let uri = Url::parse(&params.uri).ok()?;
        let directory = params.scope.as_deref() == Some("directory");
        let within = directory
            .then(|| self.file_of(&uri))
            .flatten()
            .and_then(|path| path.parent().map(Path::to_path_buf));
        let skin = self.skin.clone();
        // taken before the analysis is, which borrows the server whole
        let naming = self.files.naming();
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
                .filter(|&other| {
                    other == file || naming.path_of(ws.file_name(other)).starts_with(dir)
                })
                .flat_map(|other| ws.file_roots(other).iter().copied())
                .collect(),
            None => ws.file_roots(file).to_vec(),
        };
        let roots = roots.as_slice();
        let style = sysml_diagram::Style {
            skin,
            ..Default::default()
        };
        let draw = |diagram: &sysml_diagram::Diagram| sysml_diagram::render(diagram, &style);
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

/// Whether `offset` falls inside a comment, a string or a quoted name,
/// where nothing completion offers can be written. The end of a `//` note
/// is still inside it: the line goes on being the note until it ends.
fn written_inside_a_token(ws: &Workspace, file: usize, offset: TextSize) -> bool {
    use sysml_syntax::SyntaxKind::*;
    ws.file_parse(file)
        .syntax()
        .token_at_offset(offset)
        .any(|token| {
            let range = token.text_range();
            let to_the_end = token.kind() == LINE_NOTE && offset == range.end();
            matches!(
                token.kind(),
                LINE_NOTE | BLOCK_NOTE | COMMENT_BODY | STRING | UNRESTRICTED_NAME
            ) && range.start() < offset
                && (offset < range.end() || to_the_end)
        })
}

/// The formatter's text, indented the way the editor asked.
///
/// The formatter indents four spaces a level, and an editor set to tabs
/// or to two spaces had every line it formatted put back the other way.
/// Each line's leading spaces are taken four at a time and written as the
/// editor's own unit; what is left over, in a comment that is indented
/// inside itself, stays as spaces.
fn indented(text: &str, options: &FormattingOptions) -> String {
    let unit = match options.insert_spaces {
        true => " ".repeat(options.tab_size.max(1) as usize),
        false => "\t".to_string(),
    };
    if unit == "    " {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let rest = line.trim_start_matches(' ');
        let spaces = line.len() - rest.len();
        out.push_str(&unit.repeat(spaces / 4));
        out.push_str(&" ".repeat(spaces % 4));
        out.push_str(rest);
    }
    out
}

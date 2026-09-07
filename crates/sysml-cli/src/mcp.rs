//! A Model Context Protocol server for SysML v2, over stdio.
//!
//! SysML v2 was adopted in 2025 and hardly appears in what a language
//! model has read. Asked to write it, a model writes something that
//! looks right and names things that do not exist. This server exists to
//! answer the questions it should have asked instead:
//!
//! - `check` -- does this model parse, and does every name in it resolve?
//! - `visible_names` -- what may legally be written at this point?
//! - `library_search` -- what does the standard library actually offer?
//!
//! What they do is tell the truth about a model, which is the part a
//! language model cannot supply for itself. Where something can be
//! derived exactly -- the Rust a model implies, the SysML an existing
//! crate's API already is -- deriving it is telling the truth too, and
//! the tools that do that say in the same answer what they could not
//! derive, so what is left to write is stated rather than missing.
//!
//! The transport is the MCP stdio one: JSON-RPC 2.0, one message per
//! line. [`serve`] runs the loop over any reader and writer, so a test
//! drives it the same way a client does.

use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::path::Path;

use serde_json::{json, Value};

use sysml_model::ElementId;
use sysml_semantics::Workspace;

/// What this server speaks when the client asks for something else.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// The revisions this server can speak. A client names the one it wants
/// and that one is echoed back; a client that names anything else --
/// including a revision from after this server was written -- is told
/// what the server does speak, and decides for itself whether it can
/// live with that. Echoing back whatever was asked for promised to speak
/// revisions that do not exist.
const SPOKEN: [&str; 3] = ["2024-11-05", "2025-03-26", "2025-06-18"];

/// The library and open documents, answered against.
pub struct Server {
    /// the standard library, parsed and resolved once
    base: Workspace,
    /// Where the standard library was loaded from, or nothing when there
    /// is none. Without it every reference into the library reads as
    /// unresolved, so `check` says which it is rather than leaving a
    /// client to read a page of false alarms.
    library: Option<String>,
    /// The model being worked on, where the launcher named one.
    project: Option<Project>,
}

/// The model a call is about when it names no source of its own.
///
/// A model of any size is spread over files, and without this every call
/// has to restate all of them in `alongside`. Restating them is not slow
/// -- a call costs about eight milliseconds either way -- it is the kind
/// of thing that is got wrong: a file left out reads as a page of
/// unresolved names, and what the client then believes is that the model
/// is broken.
struct Project {
    /// Where it was loaded from, for the answer to say.
    root: String,
    /// Every file under the root: its path, and the text last read.
    files: Vec<(String, String)>,
}

impl Server {
    /// A server whose answers are grounded in the standard library at
    /// `library`. Without one it still parses and resolves, but every
    /// reference into the library reads as unresolved, so a client that
    /// has the library should say where it is.
    pub fn new(library: Option<&Path>) -> Server {
        Server::with_project(library, None)
    }

    /// A server that also holds the model being worked on, so a call
    /// that names no source of its own is about that model.
    pub fn with_project(library: Option<&Path>, project: Option<&Path>) -> Server {
        let mut base = Workspace::new();
        let mut loaded = None;
        if let Some(dir) = library {
            // A library that does not load degrades to no library, and
            // every reference into it then reads as unresolved -- a page
            // of false alarms over one wrong path. stderr is where an
            // MCP server logs, so its launcher sees this; `check`
            // carries it too, for the client that never reads the log.
            match base.load_dir(dir) {
                // the walk takes what it can reach, so a path that is
                // not there is a directory with nothing under it
                Ok(0) => eprintln!(
                    "warning: no .sysml/.kerml files under the library at {}",
                    dir.display()
                ),
                Ok(_) => {
                    base.resolve_all();
                    loaded = Some(dir.display().to_string());
                }
                Err(err) => eprintln!(
                    "warning: cannot load the standard library at {}: {err}",
                    dir.display()
                ),
            }
        }
        let mut server = Server {
            base,
            library: loaded,
            project: project.map(|root| Project {
                root: root.display().to_string(),
                files: Vec::new(),
            }),
        };
        server.refresh();
        server
    }

    /// Read the project as it is now.
    ///
    /// The agent asking these questions is the one writing the files, so
    /// a project read at startup and never again answers about a model
    /// that no longer exists. The walk is redone rather than the mtimes
    /// watched, because a file the agent has just written is as likely
    /// to be a new one as an edited one.
    fn refresh(&mut self) {
        let Some(project) = &mut self.project else {
            return;
        };
        let mut files = Vec::new();
        for path in sysml_semantics::model_files(Path::new(&project.root)) {
            // a file that cannot be read is one the answer is not about;
            // stderr is where an MCP server says so
            match std::fs::read_to_string(&path) {
                Ok(text) => files.push((canonical(&path.to_string_lossy()), text)),
                Err(err) => eprintln!("warning: cannot read {}: {err}", path.display()),
            }
        }
        project.files = files;
    }

    /// One request in, one response out -- or nothing, for a
    /// notification, which by JSON-RPC has no reply.
    pub fn handle(&mut self, request: &Value) -> Option<Value> {
        // Anything that is not a request object cannot be acted on, and
        // JSON-RPC has the server say so rather than fall silent: a
        // client that sent an id is waiting for an answer and would wait
        // for ever. MCP carries no batches, so an array -- empty or not
        // -- is refused the same way, under the null id that the spec
        // gives an unidentifiable request.
        let Some(fields) = request.as_object() else {
            return Some(invalid("a request is a JSON object", Value::Null));
        };
        let id = fields.get("id").cloned();
        let Some(method) = fields.get("method").and_then(Value::as_str) else {
            // with no id and no method there is nobody waiting, and
            // nothing to answer about
            return id.map(|id| invalid("a request names a `method`", id));
        };
        let params = fields.get("params").cloned().unwrap_or(json!({}));

        // a notification is told, not asked
        let id = id?;
        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .filter(|asked| SPOKEN.contains(asked))
                    .unwrap_or(PROTOCOL_VERSION),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "sysml-mcp", "version": env!("CARGO_PKG_VERSION") },
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => self.call(&params),
            _ => {
                return Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("no method `{method}`") },
                }))
            }
        };
        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(message) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32602, "message": message },
            }),
        })
    }

    /// A tool call. A tool that refuses answers as content marked in
    /// error rather than as a protocol error, since what it has to say
    /// is for the model to read, not for the client to handle.
    fn call(&mut self, params: &Value) -> Result<Value, String> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "a call needs the name of a tool".to_string())?;
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
        let answered = match name {
            "check" => self.check(&arguments),
            "visible_names" => self.visible_names(&arguments),
            "library_search" => self.library_search(&arguments),
            "generate_rust" => self.generate_rust(&arguments),
            "outline" => self.outline(&arguments),
            "import_rust" => self.import_rust(&arguments),
            _ => return Err(format!("no tool `{name}`")),
        };
        Ok(match answered {
            Ok(value) => json!({ "content": [text(&value)], "isError": false }),
            Err(why) => json!({
                "content": [text(&json!({ "error": why }))],
                "isError": true,
            }),
        })
    }

    /// The workspace one call is answered against: the library, the
    /// project where the launcher named one, and whatever the call
    /// brought itself.
    ///
    /// A file the call names replaces the copy the project holds rather
    /// than being added beside it -- added, everything in it would be
    /// declared twice -- so an edit can be asked about before it is
    /// written.
    fn opened(&self, arguments: &Value) -> Result<Opened, String> {
        let named = source(arguments)?;
        let alongside = alongside(arguments);
        let replaced: HashSet<String> = named
            .iter()
            .map(|(name, _)| canonical(name))
            .chain(alongside.iter().map(|path| canonical(path)))
            .collect();
        let mut ws = self.base.clone();
        let mut open = Vec::new();
        for (path, text) in self.project.iter().flat_map(|project| &project.files) {
            if !replaced.contains(path) {
                open.push(ws.add_file(path.clone(), text));
            }
        }
        for path in &alongside {
            let text = std::fs::read_to_string(path)
                .map_err(|err| format!("cannot read `{path}`: {err}"))?;
            open.push(ws.add_file(path.clone(), &text));
        }
        let focus = named.map(|(name, text)| {
            let file = ws.add_file(name, &text);
            open.push(file);
            file
        });
        Ok(Opened { ws, open, focus })
    }

    /// Does it parse, and does every name in it resolve? The findings
    /// carry the shape `sysml check --format json` reports, so a client
    /// that knows one knows the other.
    fn check(&mut self, arguments: &Value) -> Result<Value, String> {
        self.refresh();
        let Opened { mut ws, open, .. } = self.opened(arguments)?.about_something()?;

        // The answer is about the whole model that was opened -- the
        // file asked about and every file it is spread over -- because
        // that is what the reference counts are over. Each finding says
        // which file it is in, so a sibling that does not resolve is
        // reported under its own path instead of being counted in
        // `references` and then left out of `unresolved`.
        //
        // The name and the text of each open file are held here so that
        // a finding can be placed after `ws` has been borrowed to
        // resolve.
        let texts: Vec<(String, String)> = open
            .iter()
            .map(|&f| {
                (
                    ws.file_name(f).to_string(),
                    ws.file_parse(f).syntax().text().to_string(),
                )
            })
            .collect();
        let place = |finding: &sysml_semantics::Finding, mut extra: Value| {
            let which = open
                .iter()
                .position(|&f| f == finding.file)
                .expect("a finding in a file that was opened");
            let (name, text) = &texts[which];
            extra["path"] = json!(name);
            crate::at(text, usize::from(finding.range.start()), extra)
        };

        // syntax first: a file that does not parse has no names worth
        // resolving, so what would be said about them is about a tree
        // the parser guessed at
        let broken = ws.findings(&open).syntax;
        if !broken.is_empty() {
            let errors: Vec<Value> = broken
                .iter()
                .map(|f| place(f, json!({ "message": f.what })))
                .collect();
            return Ok(json!({
                "ok": false,
                "library": self.library,
                "project": self.project.as_ref().map(|it| it.root.clone()),
                "parseErrors": errors,
            }));
        }

        let stats = ws.resolve_files(&open);
        let unresolved: Vec<Value> = ws
            .findings(&open)
            .names
            .iter()
            .map(|f| place(f, json!({ "name": f.what })))
            .collect();
        Ok(json!({
            "ok": unresolved.is_empty(),
            "library": self.library,
            "project": self.project.as_ref().map(|it| it.root.clone()),
            "parseErrors": [],
            "resolved": stats.resolved,
            "references": stats.resolved + stats.unresolved,
            "unresolved": unresolved,
        }))
    }

    /// What may be written at that point: the names in scope there, by
    /// the same rules the language server completes with -- ownership,
    /// inheritance, imports and visibility.
    fn visible_names(&mut self, arguments: &Value) -> Result<Value, String> {
        self.refresh();
        let line = number(arguments, "line")?;
        let column = number(arguments, "column")?;
        let Opened {
            mut ws,
            open,
            focus,
        } = self.opened(arguments)?.about_something()?;
        // a position is in a file, and the project is a great many of
        // them: this is the one tool that has to be told which
        let file = focus
            .ok_or_else(|| "give either `text` or `path`: a position is in a file".to_string())?;
        let text = ws.file_parse(file).syntax().text().to_string();
        let offset = offset_of(&text, line, column)
            .ok_or_else(|| format!("line {line}, column {column} is past the end of the text"))?;
        ws.resolve_files(&open);
        let names: Vec<Value> = ws
            .visible_names(file, (offset as u32).into())
            .into_iter()
            .map(|(name, kind)| json!({ "name": name, "kind": kind.name() }))
            .collect();
        Ok(json!({ "visible": names }))
    }

    /// The shape a model has once it is resolved.
    ///
    /// Re-reading the text answers a different question: what a
    /// definition inherits is not written there, and neither is what a
    /// name in the standard library turns out to be.
    fn outline(&mut self, arguments: &Value) -> Result<Value, String> {
        self.refresh();
        let depth = arguments
            .get("depth")
            .and_then(Value::as_u64)
            .unwrap_or(2)
            .min(16) as usize;
        let inherited = arguments
            .get("inherited")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let opened = self.opened(arguments)?;
        // a name in the standard library is something to look at with no
        // model of one's own at all
        let Opened {
            mut ws,
            open,
            focus,
        } = if arguments.get("within").is_some() {
            opened
        } else {
            opened.about_something()?
        };
        ws.resolve_files(&open);
        let roots = match arguments.get("within").and_then(Value::as_str) {
            Some(within) => {
                // by the name it answers to, then by the whole of it:
                // the index is over declared names, and two packages can
                // declare the same one
                let declared = within.rsplit("::").next().unwrap_or(within);
                vec![ws
                    .search_names(declared, 500)
                    .into_iter()
                    .find(|&elem| ws.qualified_name_of(elem) == within)
                    .ok_or_else(|| format!("nothing is named `{within}`"))?]
            }
            // what the call named, or everything it opened
            None => focus
                .map_or(open, |file| vec![file])
                .iter()
                .flat_map(|&file| ws.file_roots(file).to_vec())
                .collect(),
        };
        let described: Vec<Value> = roots
            .iter()
            .map(|&elem| describe(&mut ws, elem, depth, inherited))
            .collect();
        Ok(json!({ "elements": described }))
    }

    /// The Rust a model implies, and what it leaves for a person.
    ///
    /// The skeleton is the generator's to write and the gaps are not:
    /// what comes back second is the work list, so that filling it is
    /// not a matter of reading the file for `todo!`.
    fn generate_rust(&mut self, arguments: &Value) -> Result<Value, String> {
        self.refresh();
        let Opened {
            mut ws,
            open,
            focus,
        } = self.opened(arguments)?.about_something()?;
        let stats = ws.resolve_files(&open);
        // Generated code would silently miss whatever did not resolve --
        // a struct without the field whose type was misspelt reads as a
        // model that never had it.
        if stats.unresolved > 0 {
            let missing: Vec<String> = ws
                .unresolved()
                .iter()
                .map(|it| format!("`{}` in {}", it.name, ws.file_name(it.file)))
                .collect();
            return Err(format!(
                "the model does not resolve, so what is generated would quietly leave \
                 things out -- {}. Ask `check` for where they are.",
                missing.join(", ")
            ));
        }
        // What is generated is what the call named; everything else that
        // was opened is there so names resolve, the way `rustgen
        // --library` loads a library it does not write.
        let written: Vec<usize> = focus.map_or(open, |file| vec![file]);
        let roots: Vec<_> = written
            .iter()
            .flat_map(|&file| ws.file_roots(file).to_vec())
            .collect();
        let generated = sysml_rust::generate(ws.model(), &roots).map_err(|err| err.to_string())?;
        let open: Vec<Value> = generated
            .open
            .iter()
            .map(|it| {
                json!({
                    "kind": it.kind.name(),
                    "sysml": it.sysml,
                    "rust": it.rust,
                    "why": it.why,
                })
            })
            .collect();
        Ok(json!({ "rust": generated.rust, "open": open }))
    }

    /// An existing Rust API stated as SysML.
    ///
    /// The mapping is not obvious -- what becomes a multiplicity, what
    /// becomes a port, which borrows can be crossed at all -- and a
    /// model that gets it wrong says so only when the generated code
    /// will not compile. So it is derived rather than written.
    fn import_rust(&mut self, arguments: &Value) -> Result<Value, String> {
        let json = match arguments.get("json").and_then(Value::as_str) {
            Some(json) => json.to_string(),
            None => {
                let path = arguments
                    .get("rustdoc")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        "give either `rustdoc` or `json`. `cargo +nightly rustdoc -- \
                         -Zunstable-options --output-format json` writes the file; this \
                         server does not run cargo."
                            .to_string()
                    })?;
                std::fs::read_to_string(path)
                    .map_err(|err| format!("cannot read `{path}`: {err}"))?
            }
        };
        let package = arguments.get("package").and_then(Value::as_str);
        let imported =
            sysml_rust::rustdoc_to_sysml(&json, package).map_err(|err| err.to_string())?;

        // The importer promises that every name in what it writes
        // resolves. Proving it costs one pass and makes the answer
        // something a client can act on without checking it first.
        let mut ws = self.base.clone();
        let file = ws.add_file("imported.sysml", &imported.sysml);
        ws.resolve_files(&[file]);
        let unresolved: Vec<String> = ws
            .findings(&[file])
            .names
            .iter()
            .map(|finding| finding.what.clone())
            .collect();
        Ok(json!({
            "sysml": imported.sysml,
            "skipped": imported.skipped,
            "checked": { "ok": unresolved.is_empty(), "unresolved": unresolved },
        }))
    }

    /// What the standard library has under that name. A model asked to
    /// use a quantity or a port will otherwise invent one.
    fn library_search(&mut self, arguments: &Value) -> Result<Value, String> {
        let query = arguments
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .min(200) as usize;
        let scope = arguments
            .get("scope")
            .and_then(Value::as_str)
            .unwrap_or("library");
        // The library is already resolved and the model is not, so the
        // two are searched in different workspaces rather than one: a
        // question about the library costs nothing it need not.
        let mut found: Vec<Value> = Vec::new();
        if scope != "model" {
            found.extend(entries(&self.base, &self.base.search_names(query, limit)));
        }
        if scope != "library" {
            self.refresh();
            let Opened { ws, open, .. } = self.opened(arguments)?.about_something()?;
            let mine: Vec<ElementId> = ws
                .search_names(query, limit)
                .into_iter()
                // the library was searched already, or was not asked for
                .filter(|&elem| ws.element_file(elem).is_some_and(|f| open.contains(&f)))
                .collect();
            found.extend(entries(&ws, &mine));
        }
        Ok(json!({ "found": found }))
    }
}

/// The other files the model is spread over. A model of any size is,
/// and checking one file of it alone reports every reference into the
/// rest as unresolved -- a false alarm on every line that reaches for
/// something the project declares elsewhere.
fn alongside(arguments: &Value) -> Vec<String> {
    arguments
        .get("alongside")
        .and_then(Value::as_array)
        .map(|paths| {
            paths
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The text a tool works on: written out in the call, or read from a
/// path, or neither -- a call that names no source is about the project
/// the server was started with. The name decides the dialect, so a
/// `.kerml` file is read as KerML whichever way it arrives.
fn source(arguments: &Value) -> Result<Option<(String, String)>, String> {
    if let Some(text) = arguments.get("text").and_then(Value::as_str) {
        let name = arguments
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("model.sysml");
        return Ok(Some((name.to_string(), text.to_string())));
    }
    let Some(path) = arguments.get("path").and_then(Value::as_str) else {
        return Ok(None);
    };
    let text =
        std::fs::read_to_string(path).map_err(|err| format!("cannot read `{path}`: {err}"))?;
    Ok(Some((path.to_string(), text)))
}

/// What each of these elements is, for an answer that only names them.
fn entries(ws: &Workspace, found: &[ElementId]) -> Vec<Value> {
    found
        .iter()
        .map(|&elem| {
            let mut entry = json!({
                "name": ws.qualified_name_of(elem),
                "kind": ws.model().kind(elem).name(),
            });
            if let Some(doc) = ws.documentation_of(elem) {
                entry["documentation"] = json!(doc);
            }
            entry
        })
        .collect()
}

/// One element as an outline: what it is, what it specializes, and what
/// is inside it down to `depth`.
fn describe(ws: &mut Workspace, elem: ElementId, depth: usize, inherited: bool) -> Value {
    let model = ws.model();
    let mut entry = json!({
        "name": ws.qualified_name_of(elem),
        "kind": model.kind(elem).name(),
    });
    if let Some(ty) = model.type_of(elem) {
        entry["type"] = json!(ws.qualified_name_of(ty));
    }
    if let Some(direction) = ws.model().direction(elem) {
        entry["direction"] = json!(direction);
    }
    if let Some(doc) = ws.documentation_of(elem) {
        entry["documentation"] = json!(doc);
    }
    // Where it is written, for a client that is going to edit it. An
    // element either has a place in a file or has none: one that was
    // materialized rather than written has no range, and saying which
    // file it would be in says nothing a client can act on.
    if let Some((file, (range, _))) = ws.element_file(elem).zip(ws.element_ranges(elem)) {
        let name = ws.file_name(file).to_string();
        let text = ws.file_parse(file).syntax().text().to_string();
        entry = crate::at(&text, usize::from(range.start()), entry);
        entry["path"] = json!(name);
    }
    let supertypes = ws.supertypes(elem);
    if !supertypes.is_empty() {
        entry["specializes"] = json!(supertypes
            .iter()
            .map(|&up| ws.qualified_name_of(up))
            .collect::<Vec<_>>());
    }
    // A definition's own members are what the text says; the inherited
    // ones are the reason to ask a resolved model rather than read it.
    let mut members: Vec<ElementId> = ws.model().owned(elem).to_vec();
    if inherited {
        for up in supertypes {
            members.extend(ws.model().owned(up).iter().copied());
        }
    }
    let named: Vec<ElementId> = members
        .into_iter()
        .filter(|&child| ws.model().name(child).is_some())
        .collect();
    let mut inside = Vec::new();
    for child in named {
        inside.push(if depth > 1 {
            describe(ws, child, depth - 1, false)
        } else {
            json!({
                "name": ws.qualified_name_of(child),
                "kind": ws.model().kind(child).name(),
            })
        });
    }
    if !inside.is_empty() {
        entry["members"] = json!(inside);
    }
    entry
}

/// The workspace one call is answered against.
struct Opened {
    ws: Workspace,
    /// Every file opened over the library: the project's and the call's.
    open: Vec<usize>,
    /// The file the call named, where it named one. What a position is
    /// measured in.
    focus: Option<usize>,
}

impl Opened {
    /// Refuse a call that named no model and had no project to fall back
    /// on. Only a question about the standard library can be answered
    /// without one.
    fn about_something(self) -> Result<Opened, String> {
        if self.open.is_empty() {
            return Err(
                "give either `text` or `path`, or start the server with `--project`".to_string(),
            );
        }
        Ok(self)
    }
}

/// A path as the file system knows it, so that one file named two ways
/// is recognised as one. A name that is not a path -- what a `text` call
/// is read under -- stands for itself.
fn canonical(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|full| full.display().to_string())
        .unwrap_or_else(|_| path.to_string())
}

fn number(arguments: &Value, key: &str) -> Result<usize, String> {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .ok_or_else(|| format!("`{key}` must be a number"))
}

/// A message JSON-RPC calls an invalid request: what came in was not
/// something the server could act on.
fn invalid(why: &str, id: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": -32600, "message": why },
    })
}

/// A one-based line and column as a byte offset, or nothing where the
/// text does not reach that far.
fn offset_of(text: &str, line: usize, column: usize) -> Option<usize> {
    let (line, column) = (line.checked_sub(1)?, column.checked_sub(1)?);
    let mut lines = text.split_inclusive('\n');
    let mut start = 0usize;
    for _ in 0..line {
        // asking past the last line is asking about nothing
        start += lines.next()?.len();
    }
    // and a column has to stay on the line it names
    let rest = lines.next().unwrap_or("");
    (column <= rest.len()).then_some(start + column)
}

/// A JSON answer as the text content a tool result carries.
fn text(value: &Value) -> Value {
    json!({
        "type": "text",
        "text": serde_json::to_string(value).expect("a built value serializes"),
    })
}

/// What the server offers, and what each wants to be given.
fn tools() -> Value {
    let source_properties = json!({
        "text": { "type": "string", "description": "The model source itself, where it is not on disk yet. Where the server holds a project and this names one of its files, this stands in for what is on disk, so an edit can be asked about before it is written." },
        "path": { "type": "string", "description": "A file to read instead of `text`. Give neither, and the call is about the whole project the server was started with." },
        "name": { "type": "string", "description": "The name `text` should be read under; the extension picks the dialect (default `model.sysml`)" },
        "alongside": {
            "type": "array",
            "items": { "type": "string" },
            "description": "The other files of the model, loaded so that references into them resolve. Give these whenever the model spans more than one file.",
        },
    });
    json!([
        {
            "name": "check",
            "description": "Parse a SysML v2 / KerML model and resolve every name in it against the standard library. Answers which references resolve to nothing, and where -- across every file that was opened, each finding under its own path. `library` and `project` say what was loaded, or are null; without the library every reference into it reads as unresolved. Use this on anything you write before believing it.",
            "inputSchema": {
                "type": "object",
                "properties": source_properties,
            },
        },
        {
            "name": "visible_names",
            "description": "The names that may legally be written at a point in the model -- what ownership, inheritance, imports and visibility make reachable there. Use this instead of guessing at a name.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": source_properties["text"],
                    "path": source_properties["path"],
                    "name": source_properties["name"],
                    "alongside": source_properties["alongside"],
                    "line": { "type": "integer", "description": "Line, counting from one" },
                    "column": { "type": "integer", "description": "Column in bytes, counting from one" },
                },
                "required": ["line", "column"],
            },
        },
        {
            "name": "outline",
            "description": "The shape a model has once it is resolved: every definition, what it specializes, and the features inside it. `within` names one element to describe instead of the whole model, and may name one of the standard library's, so a type can be looked at before it is used. Ask this rather than re-reading a model file -- what a definition inherits is not written in the text, and neither is what a library name turns out to be.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": source_properties["text"],
                    "path": source_properties["path"],
                    "name": source_properties["name"],
                    "alongside": source_properties["alongside"],
                    "within": { "type": "string", "description": "A qualified name to describe instead of the whole model" },
                    "depth": { "type": "integer", "description": "How many levels of nesting to descend (default 2, at most 16)" },
                    "inherited": { "type": "boolean", "description": "Also list the features that come from supertypes (default false)" },
                },
            },
        },
        {
            "name": "generate_rust",
            "description": "The Rust a model implies, written the one way the generator writes it: definitions become structs and enums with multiplicities as containers and inheritance flattened, ports bound to an existing Rust API become generic parameters and `perform`ed actions methods, state definitions become state machines. Do not hand-write what this returns. `open` is the other half of the answer -- every place the model deliberately left something for a person: the traits an abstract definition becomes, the `todo!`s an untranslatable expression keeps, a state machine's hooks, and everything with no generated shape, each with what it is about and why. That list is the work. A model that does not resolve is refused rather than generated from, because what came out would quietly be missing whatever the model misspelt.",
            "inputSchema": {
                "type": "object",
                "properties": source_properties,
            },
        },
        {
            "name": "import_rust",
            "description": "State an existing Rust crate's public API as a SysML package, every definition carrying the `@rust { ... }` metadata that names the item it binds to -- so a model can type its ports and `perform` its actions against the real API, and `generate_rust` can later call it instead of inventing a parallel one. The input is what `cargo +nightly rustdoc -- -Zunstable-options --output-format json` writes; this server does not run cargo. `skipped` names every item that has no monomorphic SysML shape, so what is left to model by hand is stated rather than missing, and `checked` says whether every name in the package resolves.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "rustdoc": { "type": "string", "description": "Path to the rustdoc JSON" },
                    "json": { "type": "string", "description": "The rustdoc JSON itself, where it is not on disk" },
                    "package": { "type": "string", "description": "What to call the package written (default: the crate's own name)" },
                },
            },
        },
        {
            "name": "library_search",
            "description": "Search by name. Answers the qualified name, the metaclass and the documentation of each match -- what a quantity, port or action definition is actually called. `scope` says where to look: the standard library, the model being worked on (so the same concept is not defined twice), or both.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Matched case-insensitively against the declared name" },
                    "limit": { "type": "integer", "description": "At most this many matches from each scope (default 20, at most 200)" },
                    "scope": { "type": "string", "enum": ["library", "model", "both"], "description": "Where to search (default `library`)" },
                    "text": source_properties["text"],
                    "path": source_properties["path"],
                    "name": source_properties["name"],
                    "alongside": source_properties["alongside"],
                },
                "required": ["query"],
            },
        },
    ])
}

/// Serve until the reader ends: one JSON-RPC message per line in, one
/// per line out. A line that is not JSON is answered as a parse error,
/// as JSON-RPC asks, rather than ending the session.
pub fn serve(
    server: &mut Server,
    mut input: impl BufRead,
    mut output: impl Write,
) -> std::io::Result<()> {
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        // Bytes rather than lines: a line that is not UTF-8 is not JSON
        // either, and saying so is the same answer as for any other
        // unreadable line. Ending the session over it would take down
        // whatever else the client had in flight.
        let read = std::str::from_utf8(&line)
            .map_err(|err| err.to_string())
            .and_then(|line| match line.trim() {
                "" => Ok(None),
                line => serde_json::from_str::<Value>(line)
                    .map(Some)
                    .map_err(|err| err.to_string()),
            });
        let response = match read {
            Ok(None) => continue,
            Ok(Some(request)) => server.handle(&request),
            Err(why) => Some(json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": { "code": -32700, "message": why },
            })),
        };
        if let Some(response) = response {
            writeln!(output, "{response}")?;
            output.flush()?;
        }
    }
}

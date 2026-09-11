//! A Model Context Protocol server for SysML v2, over stdio.
//!
//! SysML v2 was adopted in 2025 and hardly appears in what a language
//! model has read. Asked to write it, a model writes something that looks
//! right and names things that do not exist. This server answers the
//! questions it should have asked instead:
//!
//! - `check` -- does this model parse, and does every name in it resolve?
//! - `visible_names` -- what may legally be written at this point?
//! - `library_search` -- what does the standard library actually offer?
//! - `notation` -- how is this kind of thing written at all?
//! - `generation_plan` -- what does this model imply for code?
//!
//! Where something can be derived exactly -- the Rust a model implies, the
//! SysML a crate's API already is -- the tool says in the same answer what
//! it could not derive, so what is left to write is stated rather than
//! missing.
//!
//! The transport is the MCP stdio one: JSON-RPC 2.0, one message per line.
//! [`serve`] runs the loop over any reader and writer.

use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::path::Path;

use serde_json::{json, Value};

use sysml_model::ElementId;
use sysml_semantics::Workspace;

/// What this server speaks when the client asks for something else.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// The revisions this server can speak. A client names one and that one is
/// echoed back; any other -- including one from after this server was
/// written -- is told what the server does speak and decides for itself.
/// Echoing back whatever was asked promised revisions that do not exist.
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
/// restates them in `alongside` -- not slow, but easy to get wrong: a file
/// left out reads as a page of unresolved names.
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
        Server::holding(library, project, true)
    }

    /// A server with no standard library at all, which `sysml --no-library
    /// mcp` starts.
    ///
    /// Every reference into the library then reads as unresolved and the
    /// constraints are not put at all -- worth asking for on purpose rather
    /// than only by accident.
    pub fn without_library(project: Option<&Path>) -> Server {
        Server::holding(None, project, false)
    }

    /// Both of those: `fallback` is whether the copy built into this
    /// binary stands in for a library nobody named.
    fn holding(library: Option<&Path>, project: Option<&Path>, fallback: bool) -> Server {
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
        // Nothing said where one is, so the copy built into this binary.
        // An agent's launcher often has nowhere to put a flag, and a
        // server that answers `library_search` with nothing at all is
        // worse than useless: it reads as "the library does not declare
        // that", which is a wrong answer rather than a missing one.
        if loaded.is_none() && fallback {
            for (name, text) in sysml_stdlib::FILES {
                base.add_file(*name, text);
            }
            base.resolve_all();
            loaded = Some(format!("built in ({})", sysml_stdlib::RELEASE));
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
    /// The agent asking is the one writing the files, so a project read once
    /// at startup answers about a model that no longer exists. The walk is
    /// redone rather than mtimes watched: a file just written is as likely to
    /// be new as edited.
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
        // Anything that is not a request object cannot be acted on, and JSON-RPC
        // has the server say so rather than fall silent: a client that sent an id
        // would wait for ever. MCP carries no batches, so an array is refused the
        // same way, under the null id the spec gives an unidentifiable request.
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
            "notation" => notation(&arguments),
            "generation_plan" => self.generation_plan(&arguments),
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

    /// The workspace one call is answered against: the library, the project
    /// where the launcher named one, and whatever the call brought itself.
    ///
    /// A file the call names replaces the project's copy rather than joining
    /// it -- added, everything in it would be declared twice -- so an edit can
    /// be asked about before it is written.
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

        // The answer is about the whole model that was opened, since that is what
        // the reference counts are over. Each finding says which file it is in,
        // so a sibling that does not resolve is reported under its own path
        // rather than counted in `references` and left out of `unresolved`.
        //
        // The name and text of each open file are held here so a finding can be
        // placed after `ws` has been borrowed to resolve.
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
            crate::at(text, finding.range, extra)
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
        // The names, the root packages that collide with the library's, and --
        // if the model is in any state to be asked -- what the specification
        // requires of it. The order and the stop are `diagnose`'s: putting every
        // constraint to whatever it was handed turned one dangling reference into
        // a list of complaints about the hole.
        let diagnosed = ws.diagnose(&open);
        // A name that found nothing is reported with what it might have meant.
        // Resolution says only that both of these missed:
        //
        //     attribute capacity : VolumeValue;   -- `ISQ::VolumeValue`, un-imported
        //     attribute temp : Temperature;       -- `TemperatureValue`, misremembered
        //
        // and they are not the same mistake.
        let missed: Vec<String> = diagnosed
            .found
            .names
            .iter()
            .map(|f| f.what.clone())
            .collect();
        let might = ws.suggestions(&missed);
        let unresolved: Vec<Value> = diagnosed
            .found
            .names
            .iter()
            .zip(&might)
            .map(|(f, might)| {
                let mut said = json!({ "name": f.what });
                if !might.elsewhere.is_empty() {
                    said["declared_as"] = json!(might.elsewhere);
                }
                if !might.near.is_empty() {
                    said["did_you_mean"] = json!(might.near);
                }
                place(f, said)
            })
            .collect();
        // A root package of one's own named after one of the library's
        // resolves -- each side reads its own -- so it is said alongside
        // the names rather than counted among them. An agent writing a
        // model is exactly who needs telling.
        let collisions: Vec<Value> = diagnosed
            .found
            .collisions
            .iter()
            .map(|f| place(f, json!({ "message": f.what })))
            .collect();
        // `unevaluated` is a count and not a list: it says nothing about
        // this model -- it says which parts of the abstract syntax this
        // toolchain does not build -- and what a client acts on is the
        // violations.
        let checked = &diagnosed.rules;
        let violations: Vec<Value> = checked
            .violations
            .iter()
            .map(|violation| {
                json!({
                    "rule": violation.rule,
                    "says": violation.says,
                    "element": ws.qualified_name_of(violation.element),
                })
            })
            .collect();
        Ok(json!({
            "ok": unresolved.is_empty() && violations.is_empty(),
            "library": self.library,
            "project": self.project.as_ref().map(|it| it.root.clone()),
            "parseErrors": [],
            "resolved": stats.resolved,
            "references": stats.resolved + stats.unresolved,
            "unresolved": unresolved,
            "collisions": collisions,
            "rules": {
                // whether they were put at all: a model with a name that
                // resolves to nothing is not asked, and neither is one
                // opened without the library the constraints are written
                // against, so an empty list is not a clean bill until
                // this says it is
                "asked": diagnosed.asked,
                "violations": violations,
                "held": checked.held.len(),
                "unevaluated": checked.unevaluated.len(),
            },
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
            Some(within) => vec![ws
                .named_globally(within)
                .ok_or_else(|| format!("nothing is named `{within}`"))?],
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
    /// The mapping is not obvious -- what becomes a multiplicity, what becomes
    /// a port, which borrows can be crossed -- and a model that gets it wrong
    /// says so only when the generated code will not compile.
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

    /// What the model implies for code, in no language in particular.
    ///
    /// `generate_rust` answers the same question in Rust. This hands back what
    /// the Rust was written from, so a caller with a language of its own can
    /// write it without guessing at what a model does not wear on its face:
    /// whether four wheels are an array or a list, whether a part is owned or
    /// referred to, what `ISQ::MassValue` bottoms out in.
    fn generation_plan(&mut self, arguments: &Value) -> Result<Value, String> {
        self.refresh();
        let Opened {
            mut ws,
            open,
            focus,
        } = self.opened(arguments)?.about_something()?;
        ws.resolve_files(&open);
        let roots: Vec<ElementId> = match arguments.get("within").and_then(Value::as_str) {
            Some(within) => vec![ws
                .named_globally(within)
                .ok_or_else(|| format!("nothing is named `{within}`"))?],
            None => focus
                .map_or(open, |file| vec![file])
                .iter()
                .flat_map(|&file| ws.file_roots(file).to_vec())
                .collect(),
        };
        let planned = crate::plan::of(&mut ws, &roots);
        serde_json::to_value(&planned).map_err(|why| why.to_string())
    }

    /// What the standard library has under that name -- or, where
    /// nothing is named that, what it has that is *about* it. A model
    /// asked to use a quantity or a port will otherwise invent one.
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
        let asked = arguments
            .get("in")
            .and_then(Value::as_str)
            .unwrap_or("names");
        // The library is already resolved and the model is not, so the
        // two are searched in different workspaces rather than one: a
        // question about the library costs nothing it need not. The
        // model is opened once here rather than inside the search,
        // which may run twice.
        let mine = match scope == "library" {
            true => None,
            false => {
                self.refresh();
                let Opened { mut ws, open, .. } = self.opened(arguments)?.about_something()?;
                // Resolved, because what a match *is* is not written in
                // the text of it: an `alias Rueda for Wheel;` read
                // unresolved is a `Membership` with no documentation and
                // nothing to say what it names, which is the answer a
                // caller least wants and the one it used to get.
                ws.resolve_files(&open);
                Some((ws, open))
            }
        };
        let hunt = |how: &str| -> Vec<Value> {
            let mut found: Vec<Value> = Vec::new();
            if scope != "model" {
                found.extend(entries(&self.base, &matches(&self.base, how, query, limit)));
            }
            if let Some((ws, open)) = &mine {
                let ids: Vec<ElementId> = matches(ws, how, query, limit)
                    .into_iter()
                    // the library was searched already, or was not asked for
                    .filter(|&elem| ws.element_file(elem).is_some_and(|f| open.contains(&f)))
                    .collect();
                found.extend(entries(ws, &ids));
            }
            found
        };

        let mut how = asked;
        let mut found = hunt(how);
        // Nothing is named that. The words a specification used are not the words
        // the library used -- "how much fluid it holds" is `ISQ::VolumeValue` --
        // and an empty answer reads as "the library does not have one", which is
        // what sends a model off to invent its own.
        if found.is_empty() && how == "names" {
            how = "documentation";
            found = hunt(how);
        }
        Ok(json!({ "found": found, "searched": how }))
    }
}

/// What one workspace answers to a search of the kind asked for.
fn matches(ws: &Workspace, how: &str, query: &str, limit: usize) -> Vec<ElementId> {
    match how {
        "documentation" => ws.search_documentation(query, limit),
        "both" => {
            let mut found = ws.search_names(query, limit);
            let named = found.clone();
            found.extend(
                ws.search_documentation(query, limit)
                    .into_iter()
                    .filter(|id| !named.contains(id)),
            );
            found
        }
        // a client that asked for something else meant the default,
        // which is the search that answers most questions
        _ => ws.search_names(query, limit),
    }
}

/// How a construct is written, with an example that has been checked.
///
/// Asked for nothing in particular it answers the list, which is the
/// question a caller with prose in front of it has: not "what is the
/// grammar of a requirement" but "which of these is the sentence".
fn notation(arguments: &Value) -> Result<Value, String> {
    let Some(of) = arguments.get("of").and_then(Value::as_str) else {
        let every: Vec<Value> = crate::notation::NOTATION
            .iter()
            .map(|it| json!({ "of": it.of, "when": it.when }))
            .collect();
        return Ok(json!({ "constructs": every }));
    };
    let Some(found) = crate::notation::notation(of) else {
        let names: Vec<&str> = crate::notation::NOTATION.iter().map(|it| it.of).collect();
        return Err(format!(
            "nothing here is written as `{of}`; ask about one of: {}",
            names.join(", ")
        ));
    };
    Ok(json!({
        "of": found.of,
        "when": found.when,
        "sysml": found.sysml,
        "see_also": found.see_also,
    }))
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
            // An alias is a membership that carries a name, and the library offers
            // its friendliest names that way: `alias TemperatureValue for
            // ThermodynamicTemperatureValue;`. Read off the alias itself the
            // metaclass is `Membership` with no documentation, so what a caller most
            // wants was the thing labelled uselessly. What it names is what it is.
            let names = ws.alias_target(elem).unwrap_or(elem);
            let mut entry = json!({
                "name": ws.qualified_name_of(elem),
                "kind": ws.model().kind(names).name(),
            });
            if names != elem {
                entry["alias_for"] = json!(ws.qualified_name_of(names));
            }
            if let Some(doc) = ws.documentation_of(names) {
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
        entry = crate::at(&text, range, entry);
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
            "description": "Parse a SysML v2 / KerML model, resolve every name in it against the standard library, and run the well-formedness constraints the specification itself states. Answers which references resolve to nothing and where -- across every file that was opened, each finding under its own path, each with what it might have meant: `declared_as` names the elements that answer to it somewhere, so the name is right and wants an import, while `did_you_mean` names what is near it, so the name itself is wrong -- which root packages of your own take a name the standard library has already taken (`collisions`: both sides still resolve, but the name means one thing in your model and another in the library), and which constraints the model breaks. `library` and `project` say what was loaded, or are null; without the library every reference into it reads as unresolved. The constraints are put only to a model that parses, resolves and has the library to be asked against -- one asked with a name still dangling answers about the hole and not about the model -- so read `rules.asked` before reading an empty `rules.violations` as a clean bill. `rules.unevaluated` counts the constraints that could not be asked at all, which says what this toolchain does not yet build rather than anything about the model. Use this on anything you write before believing it.",
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
            "description": "State an existing Rust crate's public API as a SysML package, every definition carrying the `@code { ... }` metadata that names the language and the item it binds to -- so a model can type its ports and `perform` its actions against the real API, and `generate_rust` can later call it instead of inventing a parallel one. The input is what `cargo +nightly rustdoc -- -Zunstable-options --output-format json` writes; this server does not run cargo. `skipped` names every item that has no monomorphic SysML shape, so what is left to model by hand is stated rather than missing, and `checked` says whether every name in the package resolves.",
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
            "name": "notation",
            "description": "How a kind of thing is written in SysML v2, with a worked example that this toolchain has checked: it parses, every name in it resolves against the standard library, and the constraints the specification states hold of it -- so it can be copied and adapted rather than guessed at. Asked with no `of`, it lists every construct with a line saying when to reach for it, which is the question a caller transcribing prose or code actually has: which of these is the sentence in front of me. Ask this before writing a construct you have not written here before; the examples are held right by a test, and what a language model remembers of SysML v2 largely is not.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "of": { "type": "string", "description": "The construct to show -- `part`, `requirement`, `state`, and so on. Leave it out for the list of what there is, each with when it is wanted." },
                },
            },
        },
        {
            "name": "generation_plan",
            "description": "What the model implies for code, in no language in particular -- the answer `generate_rust` writes Rust from, handed over so that you can write the language you actually have. Every definition with the shape it takes in code (`record`, `value` for a primitive under another name, `enumeration`, `variation`, `abstract`, `port`, `requirement`, `state machine`, `function`, `behaviour`), what the model declares it specializes (the parent it wrote, not everything above it), and for each feature the things a model does not wear on its face and a reader guesses wrong: what the type bottoms out in among the standard library's primitives (`ISQ::MassValue` is a `Real`, and its name does not say so), the declared default, and what inherited feature it redefines -- including the ones written with no name of their own, which carry a value the model states and used to go missing. A state machine says which state it starts in, which is not the first one declared. A requirement carries what it demands, as the model wrote it, and what it is about. A behaviour carries the order of its steps and what flows between them. `checked` says whether the model behind the plan resolves: a feature whose type resolved to nothing is planned with no type at all, so read it before writing from the rest. `constants` carries what a package declares outright, so a default that names one can be followed. `performs` carries the behaviours a definition carries out -- what makes it something that *does* anything, written as a method in a language with methods -- and KerML's own words (`class`, `struct`, `datatype`, `feature`) are planned in the same shapes as SysML's. `annotations` carries what the model says about a definition that this toolchain has no opinion about -- most usefully `@code { writtenIn = \"rust\"; path = ... }` and its like, which say the thing already exists somewhere and what it is called there, so you bind to it rather than writing it again. Said only where it says something: a multiplicity that is absent is one of the thing, `ordered` and `unique` are given for collections, and `composite` for what a whole can be made of. Nothing here decides anything about a language: no identifier is spelled and no container is named, because those are yours. Ask this before writing code from a model, and write from the answer rather than from the SysML.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": source_properties["text"],
                    "path": source_properties["path"],
                    "name": source_properties["name"],
                    "alongside": source_properties["alongside"],
                    "within": { "type": "string", "description": "A qualified name to plan instead of the whole model" },
                },
            },
        },
        {
            "name": "library_search",
            "description": "Search the standard library and your own model. Answers the qualified name, the metaclass and the documentation of each match -- what a quantity, port or action definition is actually called. `scope` says where to look: the standard library, the model being worked on (so the same concept is not defined twice), or both. `in` says what to match: declared names, or the documentation, which is how to find a thing whose name you do not know -- the words a specification uses are rarely the words the library uses, and a sentence about how much a tank holds is asking for `ISQ::VolumeValue`. A name search that finds nothing falls back to the documentation by itself, and `searched` in the answer says which one answered.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Matched case-insensitively against the declared name" },
                    "limit": { "type": "integer", "description": "At most this many matches from each scope (default 20, at most 200)" },
                    "scope": { "type": "string", "enum": ["library", "model", "both"], "description": "Where to search (default `library`)" },
                    "in": { "type": "string", "enum": ["names", "documentation", "both"], "description": "What to match `query` against (default `names`, which falls back to `documentation` when nothing is named that)" },
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

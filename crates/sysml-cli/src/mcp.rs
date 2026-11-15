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
//! - `import_code` -- what is this code, in any language, as a model?
//! - `find` -- which elements are of this kind, here, or lack this link?
//! - `relations` -- what is this element linked to, both ways?
//! - `outline` -- what shape does it have once resolved, inherited and all?
//! - `generate_rust` / `import_rust` -- the Rust a model implies, and the
//!   model a crate's API already is
//! - `format` -- how does `sysml fmt` lay this out?
//! - `definition` / `references` -- what does this name resolve to, and
//!   where else is that named?
//! - `rename` -- what edits rename it everywhere, and does the model still
//!   resolve once they are made?
//! - `diagram` -- what does it look like, drawn as `sysml diagram` draws it?
//! - `impact` -- what does a change to this reach, link after link, or
//!   what does it rest on?
//! - `trace` -- which requirements are satisfied and verified, by what,
//!   and which are not?
//!
//! Every tool reads and none writes, and each says so in its annotations,
//! so a client may run them without asking each time. `rename` too: it
//! answers the edits and the client makes them, and `diagram` answers
//! the SVG for the client to save. An answer comes as
//! structured content and as the same JSON in text, for clients of either
//! revision.
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

/// What the client is told this server is for, at `initialize`.
///
/// The protocol hands this straight to the model on the other side, and
/// it is the only thing here that reaches one that has not called a tool
/// yet. SysML v2 was adopted in 2025 and there is little of it in any
/// model's training data, so what a model writes unaided looks right and
/// names things that do not exist -- which is the failure this server is
/// here to stop, and it stops nothing if it is never asked.
const INSTRUCTIONS: &str = "\
SysML v2 and KerML are newer than most of what you were trained on, and \
the names -- and which of them are visible where -- are what a model \
gets wrong. This server answers from the language's own standard \
library rather than from memory, so ask it instead of guessing.

Before you write a `.sysml` or `.kerml` file, and again after every edit \
to one, call `check` on what you are about to write: it takes the source \
as `text`, so nothing has to be on disk. Do not report a model as \
finished until `check` answers with nothing unresolved and no \
violations. A finding carries what the name might have meant -- \
`declared_as` where something answers to it elsewhere and an import is \
what is missing, `did_you_mean` where the name itself is wrong.

`visible_names` says what may legally be written at a point, `notation` \
shows how each construct is written with an example this toolchain has \
checked, and `library_search` finds what the standard library already \
declares. A model that spans several files resolves only when they are \
given together, so pass the others as `alongside`.

To read a model rather than write one, ask instead of opening files: \
`find` lists elements by kind, namespace, name or a relation they lack \
(an unverified requirement, an unallocated function), `relations` gives \
everything one element is linked to in both directions, and `outline` \
the resolved shape under a name. `definition` says what a name at a \
point resolves to and where that is declared, and `references` every \
place an element is named, inheritance included -- ask it before \
changing or deleting a declaration. `impact` follows the links on from \
there, transitively: everything a change to an element reaches, each \
with the chain of links that reached it. `trace` is the requirement \
traceability matrix -- every requirement, what satisfies and verifies \
it, and which nothing does -- and `diagram` draws the model as SVG for \
someone to look at.

Rename with `rename`, not by search and replace: it answers every edit \
the rename takes, across files and through redefinitions, with the new \
name quoted where it has to be, and says whether the model still \
resolves once they are made. It writes nothing; you apply the edits.

This holds for what you say as much as for what you write. Before \
telling anyone that SysML v2 has something, or has not got it, ask: \
`notation` with no argument lists every construct there is, and with \
one shows how that construct is written. The language was adopted in \
2025 and what you remember of it is older than that.";

/// What this server speaks when the client asks for something else.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// The revisions this server can speak. A client names one and that one is
/// echoed back; any other -- including one from after this server was
/// written -- is told what the server does speak and decides for itself.
/// Echoing back whatever was asked promised revisions that do not exist.
const SPOKEN: [&str; 3] = ["2024-11-05", "2025-03-26", "2025-06-18"];

/// Why `project` cannot be the project a server is started with, or
/// nothing where it can.
///
/// A project is a directory: its files are read from under it, and the
/// relative names a call gives are looked for there. A path that is not
/// there, or one that is a file, used to start a server with an empty
/// project, which answered every call by saying to start it with
/// `--project`. A single file is refused rather than taken as a project
/// of one, since the names a call gives would then be looked for under a
/// file; it is what a call's `path` is for, and its directory is the
/// project it belongs to. A directory with nothing in it yet is a project
/// still to be written, and is taken.
pub fn refuse_project(project: &Path) -> Option<String> {
    let shown = project.display();
    match std::fs::metadata(project) {
        Err(err) => Some(format!("cannot read the project at {shown}: {err}")),
        Ok(found) if !found.is_dir() => Some(format!(
            "the project at {shown} is a file, not a directory: start the server with \
             the directory it is in as `--project`, or pass the file to a call as `path`"
        )),
        Ok(_) => std::fs::read_dir(project)
            .err()
            .map(|err| format!("cannot read the project at {shown}: {err}")),
    }
}

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
    /// The workspaces the last few calls were answered against, newest
    /// first.
    ///
    /// Opening a project of a hundred files over the library and
    /// resolving it took a third of a second, and putting the
    /// specification's constraints to it two seconds more -- every call,
    /// though nothing on disk had changed since the last. What a call is
    /// answered against is decided by the name and text of every file it
    /// opens, so that is what one is kept under: a file edited, added or
    /// removed is a different list, and the walk that reads the project
    /// is redone on every call to find out.
    cache: Vec<Cached>,
}

/// How many workspaces [`Server::cache`] keeps. Two, so that a client
/// asking about one file by its `path` between questions about the whole
/// project does not throw the project's away each time.
const KEPT: usize = 2;

/// One workspace a call was answered against, kept for the next call that
/// opens the same files.
struct Cached {
    /// What was opened over the library, in order, and whether the last
    /// of it is the file the call named.
    files: Composition,
    /// It, resolved, before any tool has asked it anything. A call is
    /// answered against a copy, so whatever a tool works out on the way
    /// to its answer is not left behind for the next one to find.
    opened: Opened,
    /// What `check` answered of it, unfocused and focused: the
    /// constraints are most of what a check costs, and the same files
    /// draw the same answer.
    checked: [Option<Value>; 2],
}

/// The files one call opens over the library, each as it was named and
/// what it said, and whether the last of them is the one the call named.
#[derive(Clone, PartialEq, Eq)]
struct Composition {
    files: Vec<(String, String)>,
    focused: bool,
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
            cache: Vec::new(),
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

    /// One message in, one response out -- or nothing, for a
    /// notification, which by JSON-RPC has no reply.
    ///
    /// A message may be a batch: revision 2025-03-26, which this server
    /// says it speaks, has a server accept an array of requests and answer
    /// with an array of what they draw. It used to be refused whole. The
    /// array holds the answers to the requests that draw one, and a batch
    /// of notifications draws nothing at all.
    pub fn handle(&mut self, request: &Value) -> Option<Value> {
        match request.as_array() {
            Some(batch) if !batch.is_empty() => {
                let answers: Vec<Value> = batch.iter().filter_map(|one| self.one(one)).collect();
                (!answers.is_empty()).then_some(Value::Array(answers))
            }
            _ => self.one(request),
        }
    }

    /// One request, out of a batch or not.
    fn one(&mut self, request: &Value) -> Option<Value> {
        // Anything that is not a request object cannot be acted on, and JSON-RPC
        // has the server say so rather than fall silent: a client that sent an id
        // would wait for ever. An empty batch, or a batch inside one, is refused
        // the same way, under the null id the spec gives an unidentifiable request.
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
                "instructions": INSTRUCTIONS,
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
        // content beyond the answer's own text: only `diagram` adds any
        let mut pictured: Vec<Value> = Vec::new();
        let answered = match name {
            "check" => self.check(&arguments),
            "visible_names" => self.visible_names(&arguments),
            "library_search" => self.library_search(&arguments),
            "generate_rust" => self.generate_rust(&arguments),
            "outline" => self.outline(&arguments),
            "import_rust" => self.import_rust(&arguments),
            "import_code" => self.import_code(&arguments),
            "notation" => notation(&arguments),
            "generation_plan" => self.generation_plan(&arguments),
            "relations" => self.relations(&arguments),
            "find" => self.find(&arguments),
            "format" => self.format(&arguments),
            "definition" => self.definition(&arguments),
            "references" => self.references(&arguments),
            "rename" => self.rename(&arguments),
            "diagram" => self.diagram(&arguments).map(|(said, image)| {
                pictured.extend(image);
                said
            }),
            "impact" => self.impact(&arguments),
            "trace" => self.trace(&arguments),
            _ => return Err(format!("no tool `{name}`")),
        };
        // The answer twice: as structured content, which a client of the
        // 2025-06-18 revision hands its model as data, and as the same JSON
        // in text, which is all an older client reads.
        let said = match answered {
            Ok(value) => (value, false),
            Err(why) => (json!({ "error": why }), true),
        };
        // The text first, so a client that reads only the first content
        // reads the answer; a picture, where one was asked for, after it.
        let mut content = vec![text(&said.0)];
        content.extend(pictured);
        Ok(json!({
            "content": content,
            "structuredContent": said.0,
            "isError": said.1,
        }))
    }

    /// The workspace one call is answered against: the library, the project
    /// where the launcher named one, and whatever the call brought itself.
    ///
    /// A file the call names replaces the project's copy rather than joining
    /// it -- added, everything in it would be declared twice -- so an edit can
    /// be asked about before it is written.
    ///
    /// Which file a name is, is asked of the file system, so `lib.sysml`,
    /// `./lib.sysml` and the absolute path are one file -- and a relative
    /// name is read from the project's root before the server's own
    /// directory, since that is where the project's files are named from.
    /// Compared as written, `name: "lib.sysml"` replaced nothing, the
    /// project's copy on disk was loaded beside the edit, and an edit that
    /// deleted a definition another file uses was answered `ok`.
    fn opened(&mut self, arguments: &Value) -> Result<Opened, String> {
        let files = self.composition(arguments)?;
        Ok(self.open(&files))
    }

    /// Which files a call opens over the library, read as they are now.
    fn composition(&self, arguments: &Value) -> Result<Composition, String> {
        let named = self.source(arguments)?;
        let named_as = named.as_ref().map(|(name, _)| self.canonical(name));
        // each once, and none that is the file the call's text replaces
        let mut seen: HashSet<String> = named_as.iter().cloned().collect();
        let alongside: Vec<String> = alongside(arguments)?
            .into_iter()
            .filter(|path| seen.insert(self.canonical(path)))
            .collect();
        let replaced = seen;
        let mut files: Vec<(String, String)> = self
            .project
            .iter()
            .flat_map(|project| &project.files)
            .filter(|(path, _)| !replaced.contains(path))
            .cloned()
            .collect();
        for path in alongside {
            let text = self.read(&path)?;
            files.push((path, text));
        }
        let focused = named.is_some();
        files.extend(named);
        Ok(Composition { files, focused })
    }

    /// Those files opened over the library and resolved -- or a copy of
    /// them as they were when the same files were last opened.
    ///
    /// Every tool that opens a model resolves it before it answers, and
    /// the few that refuse on the way do so on what the text says alone,
    /// so resolving here first changes no answer.
    fn open(&mut self, files: &Composition) -> Opened {
        if let Some(at) = self.cache.iter().position(|it| it.files == *files) {
            let kept = self.cache.remove(at);
            let opened = kept.opened.clone();
            self.cache.insert(0, kept);
            return opened;
        }
        let mut ws = self.base.clone();
        let open: Vec<usize> = files
            .files
            .iter()
            .map(|(name, text)| ws.add_file(name.clone(), text))
            .collect();
        let focus = files.focused.then(|| *open.last().expect("the file named"));
        let stats = ws.resolve_files(&open);
        let opened = Opened {
            ws,
            open,
            focus,
            stats,
        };
        self.cache.insert(
            0,
            Cached {
                files: files.clone(),
                opened: opened.clone(),
                checked: [None, None],
            },
        );
        self.cache.truncate(KEPT);
        opened
    }

    /// The file a name a call gives stands for, spelled as the project's
    /// files are: from the project's root where it names a file there,
    /// otherwise from the server's directory.
    fn canonical(&self, path: &str) -> String {
        canonical(&self.located(path).to_string_lossy())
    }

    /// Where a path a call gives is on disk: under the project's root where
    /// it names something there, otherwise as written, from the server's
    /// own directory.
    ///
    /// Which file a `name` stood for was asked this way while `path` and
    /// `alongside` were read from the server's directory alone, so a
    /// launcher that started the server anywhere but the project's root
    /// had `path: "sub/car.sysml"` refused as unreadable -- the same
    /// spelling `name` was matched by.
    fn located(&self, path: &str) -> std::path::PathBuf {
        if let Some(project) = &self.project {
            let under = Path::new(&project.root).join(path);
            if Path::new(path).is_relative() && under.exists() {
                return under;
            }
        }
        std::path::PathBuf::from(path)
    }

    /// The text of a file a call names, found as [`Server::located`] finds it.
    fn read(&self, path: &str) -> Result<String, String> {
        std::fs::read_to_string(self.located(path))
            .map_err(|err| format!("cannot read `{path}`: {err}"))
    }

    /// The text a tool works on: written out in the call, or read from a
    /// path, or neither -- a call that names no source is about the project
    /// the server was started with. The name decides the dialect, so a
    /// `.kerml` file is read as KerML whichever way it arrives.
    fn source(&self, arguments: &Value) -> Result<Option<(String, String)>, String> {
        let name = optional_str(arguments, "name")?;
        if let Some(text) = optional_str(arguments, "text")? {
            let name = name.unwrap_or("model.sysml");
            return Ok(Some((name.to_string(), text.to_string())));
        }
        let Some(path) = optional_str(arguments, "path")? else {
            return Ok(None);
        };
        Ok(Some((path.to_string(), self.read(path)?)))
    }

    /// Does it parse, and does every name in it resolve?
    ///
    /// Each finding is placed the way `sysml check --format json` places
    /// one -- `path`, then offsets, lines and columns from [`crate::at`]
    /// -- but the answer around them is this server's own: the hints are
    /// `declared_as` and `did_you_mean` where the command line says
    /// `declaredAs` and `didYouMean`, and the violations are under
    /// `rules`. Agents already read these names, so they stay; what used
    /// to be said here -- that the two shapes were one -- was not so.
    fn check(&mut self, arguments: &Value) -> Result<Value, String> {
        self.refresh();
        let focused = match arguments.get("focus") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(it)) => *it,
            Some(_) => return Err("`focus` must be true or false".to_string()),
        };
        let files = self.composition(arguments)?;
        let kept = self.cache.iter().position(|it| it.files == files);
        if let Some(said) = kept.and_then(|at| self.cache[at].checked[usize::from(focused)].clone())
        {
            return Ok(said);
        }
        let said = self.checked(&files, focused)?;
        // kept with the workspace it was answered against, which opening
        // it has just put first
        if let Some(kept) = self.cache.iter_mut().find(|it| it.files == files) {
            kept.checked[usize::from(focused)] = Some(said.clone());
        }
        Ok(said)
    }

    /// What `check` answers of those files, worked out.
    fn checked(&mut self, files: &Composition, focused: bool) -> Result<Value, String> {
        let Opened {
            mut ws,
            open,
            focus,
            stats,
        } = self.open(files).about_something()?;
        // `focus` asks about the file the call named and nothing else:
        // in a project of a hundred files, an edit to one was answered
        // with what is wrong in all of them, and what the edit itself
        // broke was somewhere in the list.
        let reported = match (focused, focus) {
            (true, Some(file)) => vec![file],
            (true, None) => {
                return Err(
                    "`focus` reports on the file a call names: give `text` or `path` too"
                        .to_string(),
                )
            }
            (false, _) => open.clone(),
        };
        let in_report = |file: usize| reported.contains(&file);

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
        let place_in = |file: usize, range: sysml_syntax::TextRange, mut extra: Value| {
            let which = open
                .iter()
                .position(|&f| f == file)
                .expect("a finding in a file that was opened");
            let (name, text) = &texts[which];
            extra["path"] = json!(name);
            crate::at(text, range, extra)
        };
        let place = |finding: &sysml_semantics::Finding, extra: Value| {
            place_in(finding.file, finding.range, extra)
        };

        // syntax first: a file that does not parse has no names worth
        // resolving, so what would be said about them is about a tree
        // the parser guessed at
        let broken: Vec<_> = ws
            .findings(&open)
            .syntax
            .into_iter()
            .filter(|f| in_report(f.file))
            .collect();
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

        // The names, the root packages that collide with the library's, and --
        // if the model is in any state to be asked -- what the specification
        // requires of it. The order and the stop are `diagnose`'s: putting every
        // constraint to whatever it was handed turned one dangling reference into
        // a list of complaints about the hole.
        //
        // Focused, the constraints are put to the file the call named and
        // to nothing else. They were put to every file opened and all but
        // the named one's answers thrown away, and over a project of a
        // hundred files that was most of what an edit took to check. What
        // the constraints say of an element is decided by the model it is
        // in, which is resolved whole either way; which elements are
        // asked decides only which are answered for -- and those are the
        // ones in the named file, the only ones a focused answer reports.
        // Whether they are asked at all is still decided by every file:
        // a name dangling anywhere leaves the model in no state to ask.
        let diagnosed = match focused {
            true => {
                let found = ws.findings(&open);
                let asked =
                    found.syntax.is_empty() && found.names.is_empty() && ws.has_standard_library();
                let rules = match asked {
                    true => ws.check_rules(&reported),
                    false => Default::default(),
                };
                sysml_semantics::Diagnosis {
                    found,
                    rules,
                    asked,
                }
            }
            false => ws.diagnose(&open),
        };
        // A name that found nothing is reported with what it might have meant.
        // Resolution says only that both of these missed:
        //
        //     attribute capacity : VolumeValue;   -- `ISQ::VolumeValue`, un-imported
        //     attribute temp : Temperature;       -- `TemperatureValue`, misremembered
        //
        // and they are not the same mistake.
        let names: Vec<&sysml_semantics::Finding> = diagnosed
            .found
            .names
            .iter()
            .filter(|f| in_report(f.file))
            .collect();
        let missed: Vec<String> = names.iter().map(|f| f.what.clone()).collect();
        let might = ws.suggestions(&missed);
        let unresolved: Vec<Value> = names
            .iter()
            .copied()
            .zip(&might)
            .map(|(f, might)| {
                let mut said = json!({ "name": f.what });
                // a name that reaches the wrong kind of thing wants
                // neither an import nor another spelling
                if let Some(why) = ws.wrong_kind_at(f.file, f.range) {
                    said["wrong_kind"] = json!(why);
                    return place(f, said);
                }
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
            .filter(|f| in_report(f.file))
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
            .filter(|violation| {
                !focused
                    || ws
                        .element_place(violation.element)
                        .is_some_and(|(file, _)| in_report(file))
            })
            .map(|violation| {
                let said = json!({
                    "rule": violation.rule,
                    "says": violation.says,
                    "element": ws.qualified_name_written(violation.element),
                });
                // placed where the element is declared, as the command
                // line places it, wherever that is one of the files opened
                ws.element_place(violation.element)
                    .filter(|(file, _)| open.contains(file))
                    .map_or(said.clone(), |(file, range)| place_in(file, range, said))
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
                // held of the elements that could answer, and not asked
                // of the rest: a violation among those is not reported
                "partly": checked.partly.len(),
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
        let Opened { mut ws, focus, .. } = self.opened(arguments)?.about_something()?;
        // a position is in a file, and the project is a great many of
        // them: this is the one tool that has to be told which
        let file = focus
            .ok_or_else(|| "give either `text` or `path`: a position is in a file".to_string())?;
        let text = ws.file_parse(file).syntax().text().to_string();
        let offset = offset_of(&text, line, column)
            .ok_or_else(|| format!("line {line}, column {column} is past the end of the text"))?;
        // Inside a comment or a string no name is written, so none may
        // be: the names in scope around a block comment were offered for
        // a point in its prose.
        use sysml_syntax::SyntaxKind::{BLOCK_NOTE, COMMENT_BODY, LINE_NOTE, STRING};
        let at = sysml_syntax::TextSize::from(offset as u32);
        let in_prose = ws
            .file_parse(file)
            .syntax()
            .token_at_offset(at)
            .any(|token| {
                matches!(token.kind(), BLOCK_NOTE | COMMENT_BODY | LINE_NOTE | STRING)
                    && token.text_range().start() < at
                    && at < token.text_range().end()
            });
        if in_prose {
            return Ok(json!({ "visible": [] }));
        }
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
        // 0 is the element alone; anything but a count is refused rather
        // than read as the default, which a negative one used to be
        let depth = match arguments.get("depth") {
            None | Some(Value::Null) => 2,
            Some(given) => given
                .as_u64()
                .ok_or_else(|| "`depth` must be a whole number, 0 or more".to_string())?
                .min(16) as usize,
        };
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
            ..
        } = if arguments.get("within").is_some() {
            opened
        } else {
            opened.about_something()?
        };
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
            ws,
            open,
            focus,
            stats,
        } = self.opened(arguments)?.about_something()?;
        // What a file that does not parse declares is the parser's guess,
        // and the code would be generated from the guess.
        let broken: Vec<String> = open
            .iter()
            .filter(|&&file| !ws.file_parse(file).ok())
            .map(|&file| ws.file_name(file).to_string())
            .collect();
        if !broken.is_empty() {
            return Err(format!(
                "{} does not parse, so what is generated would be generated from a guess. \
                 Ask `check` for where.",
                broken.join(", ")
            ));
        }
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
                self.read(path)?
            }
        };
        let package = arguments.get("package").and_then(Value::as_str);
        let imported =
            sysml_rust::rustdoc_to_sysml(&json, package).map_err(|err| err.to_string())?;
        Ok(self.imported(imported))
    }

    /// Code in any language, stated as SysML from a surface: what
    /// `import_rust` reads off rustdoc, said as JSON by whoever read the
    /// code -- a tool for the language, or the caller itself, for a
    /// language no tool reads yet.
    fn import_code(&mut self, arguments: &Value) -> Result<Value, String> {
        let surface = match arguments.get("surface") {
            Some(Value::String(text)) => {
                sysml_rust::surface::read(text).map_err(|err| err.to_string())?
            }
            Some(given) => serde_json::from_value(given.clone())
                .map_err(|err| format!("the input is not a surface: {err}"))?,
            None => {
                let path = arguments
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        "give either `surface` -- the JSON itself, as an object -- or `path` \
                         to a file holding it. `sysml import-code --schema` prints its shape."
                            .to_string()
                    })?;
                let text = self.read(path)?;
                sysml_rust::surface::read(&text).map_err(|err| err.to_string())?
            }
        };
        let package = arguments.get("package").and_then(Value::as_str);
        Ok(self.imported(sysml_rust::surface::to_sysml(&surface, package)))
    }

    /// An imported package, checked. The writer promises that every name
    /// in what it writes resolves. Proving it costs one pass and makes
    /// the answer something a client can act on without checking it
    /// first.
    fn imported(&self, imported: sysml_rust::surface::Imported) -> Value {
        let mut ws = self.base.clone();
        let file = ws.add_file("imported.sysml", &imported.sysml);
        ws.resolve_files(&[file]);
        let unresolved: Vec<String> = ws
            .findings(&[file])
            .names
            .iter()
            .map(|finding| finding.what.clone())
            .collect();
        json!({
            "sysml": imported.sysml,
            "skipped": imported.skipped,
            "checked": { "ok": unresolved.is_empty(), "unresolved": unresolved },
        })
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
            ..
        } = self.opened(arguments)?.about_something()?;
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
        let planned = sysml_rust::plan::of(&mut ws, &roots);
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
                let Opened { ws, open, .. } = self.opened(arguments)?.about_something()?;
                // Resolved -- as everything opened is -- because what a match *is* is not written in
                // the text of it: an `alias Rueda for Wheel;` read
                // unresolved is a `Membership` with no documentation and
                // nothing to say what it names, which is the answer a
                // caller least wants and the one it used to get.
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

    /// The model source as `sysml fmt` writes it.
    ///
    /// Answered rather than written: every tool here reads and none
    /// writes, so a client is never surprised by a file it did not change
    /// itself. Text that does not parse is refused, as `sysml fmt` refuses
    /// it -- re-spacing what the parser could not follow can move where a
    /// quote or a comment ends.
    fn format(&mut self, arguments: &Value) -> Result<Value, String> {
        let Some((name, text)) = self.source(arguments)? else {
            return Err("give either `text` or `path`: formatting is of one file".to_string());
        };
        let width = match arguments.get("width") {
            None | Some(Value::Null) => sysml_syntax::fmt::Layout::default().width,
            Some(given) => given
                .as_u64()
                .ok_or_else(|| "`width` must be a whole number, 0 for no limit".to_string())?
                as usize,
        };
        let parse = sysml_syntax::parse_dialect(&text, sysml_syntax::Dialect::from_path(&name));
        if !parse.ok() {
            return Err(format!(
                "`{name}` does not parse, so it is not formatted: re-spacing what the parser \
                 could not follow can change what it says. Ask `check` for where."
            ));
        }
        let formatted =
            sysml_syntax::fmt::format_parsed_with(&parse, sysml_syntax::fmt::Layout { width });
        Ok(json!({
            "changed": formatted != text,
            "formatted": formatted,
        }))
    }

    /// What one element is linked to, both ways: what it satisfies,
    /// verifies, is allocated to, performs, is typed by -- and what does
    /// any of that to it. None of the incoming side is written where the
    /// element is declared, which is why reading the file does not answer it.
    fn relations(&mut self, arguments: &Value) -> Result<Value, String> {
        self.refresh();
        let wanted = arguments
            .get("element")
            .and_then(Value::as_str)
            .ok_or_else(|| "`element` names what to ask about".to_string())?;
        let Opened { mut ws, open, .. } = self.opened(arguments)?.about_something()?;
        let elem = element_named(&mut ws, &open, wanted)?;
        let mut files = Texts::default();
        let edges = edges(&ws, &open);
        let mut outgoing = Vec::new();
        let mut incoming = Vec::new();
        for edge in edges
            .iter()
            .filter(|e| e.source == elem || e.target == elem)
        {
            let (list, other) = match edge.source == elem {
                true => (&mut outgoing, edge.target),
                false => (&mut incoming, edge.source),
            };
            let mut entry = json!({ "relation": edge.relation });
            entry["element"] = summary(&ws, &mut files, other);
            entry["at"] = files.place(&ws, edge.file, edge.offset);
            list.push(entry);
        }
        Ok(json!({
            "element": summary(&ws, &mut files, elem),
            "outgoing": outgoing,
            "incoming": incoming,
        }))
    }

    /// The model's elements that pass a filter: of a kind, under a
    /// namespace, named like something, or missing a relation -- the
    /// last being how an unverified requirement or an unallocated function
    /// is found.
    fn find(&mut self, arguments: &Value) -> Result<Value, String> {
        self.refresh();
        let text = |key: &str| arguments.get(key).and_then(Value::as_str);
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(50)
            .min(500) as usize;
        let kind = text("kind").map(str::to_lowercase);
        let query = text("query").map(str::to_lowercase);
        let Opened { mut ws, open, .. } = self.opened(arguments)?.about_something()?;
        let candidates: Vec<ElementId> = match text("within") {
            Some(within) => {
                let root = element_named(&mut ws, &open, within)?;
                ws.model().descendants(root).into_iter().skip(1).collect()
            }
            None => open
                .iter()
                .flat_map(|&file| ws.file_elements(file).to_vec())
                .collect(),
        };
        // A relation no edge is ever called -- `verified` for `verifies` --
        // filtered nothing out, and every element came back as lacking it:
        // a list of "unverified" requirements that was every requirement.
        if let Some(relation) = text("without").filter(|r| !RELATIONS.contains(r)) {
            return Err(format!(
                "no relation `{relation}`: `without` takes one of {}",
                RELATIONS
                    .iter()
                    .map(|r| format!("`{r}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let related: Option<HashSet<ElementId>> = text("without").map(|relation| {
            edges(&ws, &open)
                .into_iter()
                .filter(|e| e.relation == relation)
                .flat_map(|e| [e.source, e.target])
                .collect()
        });
        let model = ws.model();
        let passing: Vec<ElementId> = candidates
            .into_iter()
            .filter(|&elem| {
                let names: Vec<String> = ["declaredName", "declaredShortName"]
                    .iter()
                    .filter_map(|p| model.get(elem, p).and_then(sysml_model::Value::as_str))
                    .map(str::to_lowercase)
                    .collect();
                !names.is_empty()
                    && kind.as_ref().is_none_or(|k| {
                        model
                            .kind(elem)
                            .name()
                            .to_lowercase()
                            .starts_with(k.as_str())
                    })
                    && query
                        .as_ref()
                        .is_none_or(|q| names.iter().any(|n| n.contains(q.as_str())))
                    && related.as_ref().is_none_or(|r| !r.contains(&elem))
            })
            .collect();
        let mut files = Texts::default();
        let found: Vec<Value> = passing
            .iter()
            .take(limit)
            .map(|&elem| {
                let mut entry = summary(&ws, &mut files, elem);
                if let Some(doc) = ws.documentation_of(elem) {
                    entry["documentation"] = json!(doc);
                }
                entry
            })
            .collect();
        Ok(json!({ "found": found, "total": passing.len() }))
    }
}

impl Server {
    /// Where the element at a point, or of a name, is declared, and what
    /// it is -- what an editor's go-to-definition and hover say together.
    fn definition(&mut self, arguments: &Value) -> Result<Value, String> {
        self.refresh();
        let Opened {
            mut ws,
            open,
            focus,
            ..
        } = self.opened(arguments)?;
        let elem = asked_about(&mut ws, &open, focus, arguments)?;
        let model = ws.model();
        let mut said = json!({
            "element": qualified(&ws, elem),
            "kind": model.kind(elem).name(),
        });
        if let Some(short) = model.effective_short_name(elem) {
            said["shortName"] = json!(short);
        }
        if let Some(ty) = model.type_of(elem) {
            said["type"] = json!(ws.qualified_name_written(ty));
        }
        // An alias is a name for something else, and what it names is
        // what a caller asking about it wants next.
        if let Some(names) = ws.alias_target(elem) {
            said["alias_for"] = json!(ws.qualified_name_written(names));
        }
        if let Some(doc) = ws.documentation_of(elem) {
            said["documentation"] = json!(doc);
        }
        said["declared"] = match ws.declared_at(elem) {
            Some((file, range)) => placed(&ws, file, range, json!({})),
            // made by the toolchain rather than written: nowhere to go
            None => Value::Null,
        };
        Ok(said)
    }

    /// Every place the element at a point, or of a name, is named.
    fn references(&mut self, arguments: &Value) -> Result<Value, String> {
        self.refresh();
        let Opened {
            mut ws,
            open,
            focus,
            ..
        } = self.opened(arguments)?;
        let found = asked_about(&mut ws, &open, focus, arguments)?;
        // A feature that declares no name answers to the one it borrows,
        // and what is named that is the declaration it came from -- the
        // same element a rename of it would rename.
        let elem = ws.model().naming_element(found).unwrap_or(found);
        let mut places: Vec<(usize, sysml_syntax::TextRange)> = ws.mentions(elem);
        // in the order they are written, and each once
        places.sort_by_key(|&(file, range)| (file, range.start(), range.end()));
        places.dedup();
        // The library names what it declares thousands of times, and
        // none of those is anything the model's author wrote or can
        // change; they are counted rather than listed.
        let in_library = places
            .iter()
            .filter(|(file, _)| !open.contains(file))
            .count();
        let mut listed = Vec::new();
        if let Some((file, range)) = ws.declared_at(elem) {
            let mut place = placed(&ws, file, range, json!({ "declaration": true }));
            if !open.contains(&file) {
                place["library"] = json!(true);
            }
            listed.push(place);
        }
        for (file, range) in places.into_iter().filter(|(file, _)| open.contains(file)) {
            listed.push(placed(&ws, file, range, json!({})));
        }
        Ok(json!({
            "element": qualified(&ws, elem),
            "kind": ws.model().kind(elem).name(),
            "references": listed,
            "in_library": in_library,
        }))
    }

    /// The edits that rename an element everywhere it is named, and what
    /// the model would say once they were made. Answered, not written.
    fn rename(&mut self, arguments: &Value) -> Result<Value, String> {
        self.refresh();
        let new_name = optional_str(arguments, "new_name")?
            .ok_or_else(|| "`new_name` is what to rename it to".to_string())?;
        let Opened {
            mut ws,
            open,
            focus,
            ..
        } = self.opened(arguments)?.about_something()?;
        // Where the rename is asked from: the point given, or the name the
        // element is declared with -- its short name where that is what
        // the call wrote, since that is then the name being given up.
        let (file, offset) = match optional_str(arguments, "element")? {
            Some(wanted) => {
                let elem = element_asked(&mut ws, &open, wanted)?;
                let short = wanted.trim_matches(|c| c == '<' || c == '>' || c == '\'');
                let model = ws.model();
                let by_short = model.effective_short_name(elem) == Some(short)
                    && model.effective_name(elem) != Some(short);
                let at = match by_short {
                    true => ws.element_file(elem).zip(ws.short_name_range(elem)),
                    false => ws.declared_at(elem),
                };
                let (file, range) = at.ok_or_else(|| {
                    format!("`{wanted}` is not written anywhere, so there is no name to rename")
                })?;
                (file, range.start())
            }
            None => position(&ws, focus, arguments)?,
        };
        let dialect = sysml_syntax::Dialect::from_path(ws.file_name(file));
        let renaming = ws.prepare_rename(file, offset, new_name, dialect)?;
        // The library's files are the ones opened before anything of the
        // model's, and none of them is anybody's to rewrite.
        if renaming.declared.0 < self.base.file_count() {
            return Err(format!(
                "`{}` is declared in the standard library, which is not yours to rename",
                ws.qualified_name_written(renaming.target)
            ));
        }
        let element = qualified(&ws, renaming.target);
        let edits = ws.rename_edits(&renaming, |_, name| sysml_syntax::Dialect::from_path(name))?;

        // The edits, made in memory to every file they touch, and the
        // model opened again from what that leaves: a rename that the
        // checks above let through and that still leaves a name dangling
        // is said to, rather than found out after the client has written
        // it.
        let mut once = HashSet::new();
        let open: Vec<usize> = open.into_iter().filter(|&f| once.insert(f)).collect();
        let mut texts: Vec<String> = open
            .iter()
            .map(|&f| ws.file_parse(f).syntax().text().to_string())
            .collect();
        let mut said = Vec::new();
        let mut count = 0;
        for (file, mut written) in edits {
            written.sort_by_key(|edit| edit.range.start());
            written.dedup();
            count += written.len();
            let at = open.iter().position(|&f| f == file);
            let text = ws.file_parse(file).syntax().text().to_string();
            let listed: Vec<Value> = written
                .iter()
                .map(|edit| crate::at(&text, edit.range, json!({ "newText": edit.new_text })))
                .collect();
            if let Some(at) = at {
                for edit in written.iter().rev() {
                    texts[at].replace_range(
                        usize::from(edit.range.start())..usize::from(edit.range.end()),
                        &edit.new_text,
                    );
                }
            }
            if !listed.is_empty() {
                said.push(json!({ "path": ws.file_name(file), "edits": listed }));
            }
        }
        let before: Vec<(String, String)> = ws
            .findings(&open)
            .names
            .iter()
            .map(|f| (ws.file_name(f.file).to_string(), f.what.clone()))
            .collect();
        let mut after = self.base.clone();
        let reopened: Vec<usize> = open
            .iter()
            .zip(&texts)
            .map(|(&f, text)| after.add_file(ws.file_name(f).to_string(), text))
            .collect();
        after.resolve_files(&reopened);
        let found = after.findings(&reopened);
        let place = |finding: &sysml_semantics::Finding, extra: Value| {
            let which = reopened
                .iter()
                .position(|&f| f == finding.file)
                .expect("a finding in a file that was opened");
            let mut extra = crate::at(&texts[which], finding.range, extra);
            extra["path"] = json!(after.file_name(finding.file));
            extra
        };
        let parse_errors: Vec<Value> = found
            .syntax
            .iter()
            .map(|f| place(f, json!({ "message": f.what })))
            .collect();
        // What dangles after that did not before. Compared by file and
        // name rather than by place, since the rename moves the text
        // after every edit it makes.
        let mut was = before.clone();
        let introduced: Vec<Value> = found
            .names
            .iter()
            .filter(|f| {
                let key = (after.file_name(f.file).to_string(), f.what.clone());
                match was.iter().position(|it| *it == key) {
                    Some(at) => {
                        was.remove(at);
                        false
                    }
                    None => true,
                }
            })
            .map(|f| place(f, json!({ "name": f.what })))
            .collect();
        Ok(json!({
            "element": element,
            "new_name": new_name,
            "files": said,
            "edits": count,
            "written": false,
            "checked": {
                "ok": parse_errors.is_empty() && introduced.is_empty(),
                "parseErrors": parse_errors,
                "introduced": introduced,
                "unresolved": found.names.len(),
                "unresolved_before": before.len(),
            },
        }))
    }
}

impl Server {
    /// The drawing `sysml diagram` makes of the model, answered as SVG.
    ///
    /// Answered rather than written, like everything here: the client
    /// has the SVG in hand and puts it where its user wants it. What the
    /// drawing holds -- which boxes, how many of each line -- comes with
    /// it, since a model reading the answer cannot look at the picture
    /// and should not have to read the SVG to know what is in it.
    fn diagram(&mut self, arguments: &Value) -> Result<(Value, Option<Value>), String> {
        self.refresh();
        let view = optional_str(arguments, "view")?.unwrap_or("definitions");
        let skin = match arguments.get("skin") {
            None | Some(Value::Null) => sysml_diagram::Skin::default(),
            Some(Value::String(asked)) => match sysml_diagram::Skin::named(asked) {
                Some(skin) => skin,
                // a name that is none of the skins that ship is a file
                // saying what to paint it in, read as `--skin` reads one
                None => {
                    let text = self.read(asked).map_err(|why| {
                        format!(
                            "no skin `{asked}`, and {why}; the skins that ship are {}",
                            sysml_diagram::Skin::NAMES.join(", ")
                        )
                    })?;
                    let said: Value = serde_json::from_str(&text)
                        .map_err(|why| format!("`{asked}` is not JSON: {why}"))?;
                    sysml_diagram::skin::read(&said).map_err(|why| format!("{asked}: {why}"))?
                }
            },
            Some(said @ Value::Object(_)) => {
                sysml_diagram::skin::read(said).map_err(|why| format!("`skin`: {why}"))?
            }
            Some(_) => {
                return Err(
                    "`skin` is the name of one that ships, a path to a JSON skin, or the \
                     skin itself as an object"
                        .to_string(),
                )
            }
        };
        let image = match arguments.get("image") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(it)) => *it,
            Some(_) => return Err("`image` must be true or false".to_string()),
        };
        let Opened {
            mut ws,
            open,
            focus,
            stats,
        } = self.opened(arguments)?.about_something()?;

        // Drawn all the same, as `sysml diagram` draws it, since the
        // person writing the model is entitled to see it so far -- but a
        // drawing of a model that does not parse or resolve leaves out
        // what the parser guessed past and what a dangling name typed,
        // and nothing on the page says so. The answer does.
        let mut warnings: Vec<String> = open
            .iter()
            .filter(|&&file| !ws.file_parse(file).ok())
            .map(|&file| {
                format!(
                    "{} does not parse, so what is drawn of it is a guess; `check` says where",
                    ws.file_name(file)
                )
            })
            .collect();
        if stats.unresolved > 0 {
            warnings.push(format!(
                "{} name(s) resolve to nothing, so what they type or relate is missing from \
                 the drawing; `check` says where they are",
                stats.unresolved
            ));
        }

        // What a view of the whole of something is of: one element, or
        // what the call named, or everything opened.
        let roots: Vec<ElementId> = match optional_str(arguments, "within")? {
            Some(within) => vec![element_asked(&mut ws, &open, within)?],
            None => focus
                .map_or(open.clone(), |file| vec![file])
                .iter()
                .flat_map(|&file| ws.file_roots(file).to_vec())
                .collect(),
        };
        // the one element a view of one element is of
        let target = |ws: &mut Workspace, of: &str| -> Result<ElementId, String> {
            let wanted = optional_str(arguments, "element")?.ok_or_else(|| {
                format!("the {of} view is of one definition: name it as `element`")
            })?;
            element_asked(ws, &open, wanted)
        };
        let style = sysml_diagram::Style {
            skin,
            ..Default::default()
        };
        let (svg, drawn) = match view {
            "definitions" | "internal" => {
                let diagram = match view {
                    "internal" => {
                        let of = target(&mut ws, "internal")?;
                        sysml_diagram::interconnection_diagram(ws.model(), of)
                    }
                    _ => sysml_diagram::definition_diagram(ws.model(), &roots),
                };
                if diagram.nodes.is_empty() {
                    return Err(match view {
                        "internal" => "it has no parts or connections to draw".to_string(),
                        _ => "nothing to draw: there are no definitions here".to_string(),
                    });
                }
                // by the name each line is drawn with, so that a count
                // can be read against the picture
                let mut lines: std::collections::BTreeMap<&str, usize> = Default::default();
                for edge in &diagram.edges {
                    *lines.entry(edge.relation.name()).or_default() += 1;
                }
                let boxes: Vec<Value> = diagram
                    .nodes
                    .iter()
                    .map(|node| json!(qualified(&ws, node.id)))
                    .collect();
                (
                    sysml_diagram::render(&diagram, &style),
                    json!({ "boxes": boxes, "lines": lines }),
                )
            }
            "browser" => {
                let browser = sysml_diagram::browser_view(ws.model(), &roots);
                if browser.rows.is_empty() {
                    return Err("nothing to draw".to_string());
                }
                (
                    sysml_diagram::render_browser(&browser, &style),
                    json!({ "rows": browser.rows.len() }),
                )
            }
            "sequence" => {
                let of = target(&mut ws, "sequence")?;
                let sequence = sysml_diagram::sequence_view(ws.model(), of);
                if sequence.lifelines.is_empty() {
                    return Err(format!(
                        "`{}` declares no interaction to draw: a sequence view is of the \
                         events and messages one definition declares",
                        qualified(&ws, of)
                    ));
                }
                let lifelines: Vec<Value> = sequence
                    .lifelines
                    .iter()
                    .map(|it| json!(qualified(&ws, it.id)))
                    .collect();
                (
                    sysml_diagram::render_sequence(&sequence, &style),
                    json!({ "lifelines": lifelines, "messages": sequence.moments.len() }),
                )
            }
            other => {
                return Err(format!(
                    "no view `{other}`: `view` is one of `definitions`, `internal`, `browser`, \
                     `sequence`"
                ))
            }
        };
        // The SVG as an image as well, only where it is asked for. A
        // client hands an image content on to its model, and a model that
        // takes pictures takes PNG, JPEG, GIF or WebP: an SVG among them
        // is refused there, and the turn with it. The text is what every
        // client can use -- written to a file, or shown -- so it is the
        // answer, and the image is for a client known to draw SVG.
        let picture = image.then(|| {
            json!({
                "type": "image",
                "data": base64(svg.as_bytes()),
                "mimeType": "image/svg+xml",
            })
        });
        Ok((
            json!({
                "view": view,
                "drawn": drawn,
                "warnings": warnings,
                "svg": svg,
            }),
            picture,
        ))
    }

    /// What an element's change reaches: everything linked to it, and
    /// everything linked to those, as far as `depth` -- or, the other
    /// way, everything it rests on.
    ///
    /// `relations` answers one step of this, and a review asks the whole
    /// of it: a part definition changed is every usage typed by it, every
    /// whole those belong to, every requirement one of them was said to
    /// satisfy and every test that verifies one of those.
    fn impact(&mut self, arguments: &Value) -> Result<Value, String> {
        self.refresh();
        let wanted = optional_str(arguments, "element")?
            .ok_or_else(|| "`element` names what would change".to_string())?;
        let direction = optional_str(arguments, "direction")?.unwrap_or("affected");
        if !["affected", "depends_on"].contains(&direction) {
            return Err(format!(
                "no direction `{direction}`: `direction` is `affected` (what a change to it \
                 reaches) or `depends_on` (what it rests on)"
            ));
        }
        let depth = match arguments.get("depth") {
            None | Some(Value::Null) => 6,
            Some(given) => given
                .as_u64()
                .filter(|&it| it > 0)
                .ok_or_else(|| "`depth` must be a whole number, 1 or more".to_string())?
                .min(32) as usize,
        };
        let only: Option<Vec<String>> = match arguments.get("relations") {
            None | Some(Value::Null) => None,
            Some(Value::Array(given)) => Some(
                given
                    .iter()
                    .map(|it| match it.as_str() {
                        Some(relation) if RELATIONS.contains(&relation) || relation == "owns" => {
                            Ok(relation.to_string())
                        }
                        _ => Err(format!(
                            "`relations` takes relations `relations` names, and `owns`: {it} is \
                             none of them"
                        )),
                    })
                    .collect::<Result<_, _>>()?,
            ),
            Some(_) => return Err("`relations` must be an array of relation names".to_string()),
        };
        let followed = |relation: &str| {
            only.as_ref()
                .is_none_or(|it| it.iter().any(|r| r == relation))
        };

        let Opened { mut ws, open, .. } = self.opened(arguments)?.about_something()?;
        let start = element_asked(&mut ws, &open, wanted)?;
        let edges = edges(&ws, &open);
        let model = ws.model();
        let ours = |elem: ElementId| ws.element_file(elem).is_some_and(|f| open.contains(&f));
        let mut by_target: std::collections::HashMap<ElementId, Vec<&Edge>> = Default::default();
        let mut by_source: std::collections::HashMap<ElementId, Vec<&Edge>> = Default::default();
        for edge in &edges {
            by_target.entry(edge.target).or_default().push(edge);
            by_source.entry(edge.source).or_default().push(edge);
        }

        // One step from `at`: the element reached, and the link that
        // reached it as the model says it -- `source relation target`,
        // and where that is written.
        let mut files = Texts::default();
        let step = |files: &mut Texts, edge: &Edge| -> Value {
            json!({
                "from": qualified(&ws, edge.source),
                "relation": edge.relation,
                "to": qualified(&ws, edge.target),
                "at": files.place(&ws, edge.file, edge.offset),
            })
        };
        // A change to an element is a change to everything inside it,
        // so the element asked about answers for its members too; what
        // is reached later answers for itself alone, or the whole of
        // every definition anything touches would be swept in.
        let inside = |elem: ElementId, first: bool| -> Vec<ElementId> {
            match first {
                true => model.descendants(elem),
                false => vec![elem],
            }
        };
        let mut reached: Vec<(ElementId, usize, Vec<Value>)> = Vec::new();
        let mut seen: HashSet<ElementId> = HashSet::from([start]);
        let mut frontier: Vec<(ElementId, Vec<Value>)> = vec![(start, Vec::new())];
        let mut truncated = false;
        for level in 1..=depth {
            let mut next = Vec::new();
            for (at, path) in &frontier {
                let mut onward: Vec<(ElementId, Value)> = Vec::new();
                for elem in inside(*at, level == 1) {
                    match direction {
                        // Whoever names it rests on it -- and what it
                        // claims of something else has to be claimed
                        // again: the requirement it satisfies, the one
                        // it verifies, the component it is allocated to,
                        // what it is connected to.
                        "affected" => {
                            for edge in by_target.get(&elem).into_iter().flatten() {
                                if followed(edge.relation) {
                                    onward.push((edge.source, step(&mut files, edge)));
                                }
                            }
                            for edge in by_source.get(&elem).into_iter().flatten() {
                                if CLAIMS.contains(&edge.relation) && followed(edge.relation) {
                                    onward.push((edge.target, step(&mut files, edge)));
                                }
                            }
                        }
                        _ => {
                            for edge in by_source.get(&elem).into_iter().flatten() {
                                if followed(edge.relation) {
                                    onward.push((edge.target, step(&mut files, edge)));
                                }
                            }
                        }
                    }
                }
                // A feature changed changes the whole it belongs to, and
                // so whoever uses that whole -- but not the package it is
                // in, which would be every element beside it.
                if direction == "affected" && followed("owns") {
                    let up = model.owner(*at).map(|it| named_owner(model, it));
                    if let Some(owner) =
                        up.filter(|&it| model.kind(it).is_a(sysml_model::ElementKind::Type))
                    {
                        let at_owner = ws
                            .element_place(*at)
                            .map(|(file, range)| files.place(&ws, file, range.start().into()));
                        onward.push((
                            owner,
                            json!({
                                "from": qualified(&ws, owner),
                                "relation": "owns",
                                "to": qualified(&ws, *at),
                                "at": at_owner,
                            }),
                        ));
                    }
                }
                for (elem, link) in onward {
                    if !seen.insert(elem) {
                        continue;
                    }
                    if reached.len() >= REACHED {
                        truncated = true;
                        break;
                    }
                    let mut path = path.clone();
                    path.push(link);
                    reached.push((elem, level, path.clone()));
                    // the standard library is where a model's links end:
                    // what it declares is not the model's to change, and
                    // what rests on it there is not the model's either
                    if ours(elem) {
                        next.push((elem, path));
                    }
                }
            }
            frontier = next;
            if frontier.is_empty() {
                break;
            }
        }
        // whether the walk stopped at `depth` with elements still to
        // follow from -- which may or may not have led anywhere
        let cut = !frontier.is_empty();
        let listed: Vec<Value> = reached
            .into_iter()
            .map(|(elem, steps, path)| {
                let mut entry = summary(&ws, &mut files, elem);
                entry["depth"] = json!(steps);
                if !ours(elem) {
                    entry["library"] = json!(true);
                }
                entry["via"] = json!(path);
                entry
            })
            .collect();
        Ok(json!({
            "element": summary(&ws, &mut files, start),
            "direction": direction,
            "total": listed.len(),
            "reached": listed,
            "stopped_at_depth": cut,
            "truncated": truncated,
        }))
    }

    /// The requirement traceability matrix: every requirement, what
    /// satisfies it and what verifies it, and which have neither.
    ///
    /// `find` with `without` answers one column of this and `relations`
    /// one row; a review asks for the table.
    fn trace(&mut self, arguments: &Value) -> Result<Value, String> {
        use sysml_model::ElementKind as K;
        self.refresh();
        let Opened { mut ws, open, .. } = self.opened(arguments)?.about_something()?;
        let candidates: Vec<ElementId> = match optional_str(arguments, "within")? {
            Some(within) => {
                let root = element_named(&mut ws, &open, within)?;
                ws.model().descendants(root)
            }
            None => open
                .iter()
                .flat_map(|&file| ws.file_elements(file).to_vec())
                .collect(),
        };
        let model = ws.model();
        let requirement = |elem: ElementId| {
            let kind = model.kind(elem);
            (kind.is_a(K::RequirementDefinition) || kind.is_a(K::RequirementUsage))
                // a satisfaction is a requirement usage by metaclass, and
                // a viewpoint one by specialization, and neither is a
                // requirement anybody has to meet
                && !kind.is_a(K::SatisfyRequirementUsage)
                && !kind.is_a(K::ViewpointDefinition)
                && !kind.is_a(K::ViewpointUsage)
                // a case's objective, and the `verify` inside it, are
                // what the case is for, not requirements of their own
                && !matches!(
                    sysml_model::membership_kind(model, elem),
                    K::ObjectiveMembership | K::RequirementVerificationMembership
                )
                && model.effective_name(elem).or(model.effective_short_name(elem)).is_some()
        };
        let mut seen = HashSet::new();
        let requirements: Vec<ElementId> = candidates
            .into_iter()
            .filter(|&elem| requirement(elem) && seen.insert(elem))
            .collect();
        let edges = edges(&ws, &open);
        let mut files = Texts::default();
        let by = |files: &mut Texts, elem: ElementId, relation: &str| -> Vec<Value> {
            let mut once = HashSet::new();
            edges
                .iter()
                .filter(|edge| edge.target == elem && edge.relation == relation)
                .filter(|edge| once.insert(edge.source))
                .map(|edge| {
                    let mut entry = json!({ "name": qualified(&ws, edge.source) });
                    if let Some(short) = model.effective_short_name(edge.source) {
                        entry["shortName"] = json!(short);
                    }
                    entry["at"] = files.place(&ws, edge.file, edge.offset);
                    entry
                })
                .collect()
        };
        let mut rows = Vec::new();
        let mut unsatisfied = Vec::new();
        let mut unverified = Vec::new();
        for &elem in &requirements {
            let satisfied_by = by(&mut files, elem, "satisfies");
            let not_satisfied_by = by(&mut files, elem, "does_not_satisfy");
            let verified_by = by(&mut files, elem, "verifies");
            let name = qualified(&ws, elem);
            if satisfied_by.is_empty() {
                unsatisfied.push(json!(name));
            }
            if verified_by.is_empty() {
                unverified.push(json!(name));
            }
            let mut row = json!({ "name": name, "kind": model.kind(elem).name() });
            if let Some(short) = model.effective_short_name(elem) {
                row["shortName"] = json!(short);
            }
            if let Some(doc) = ws.documentation_of(elem) {
                row["documentation"] = json!(doc);
            }
            // a usage's definition is where what it demands is said
            if let Some(ty) = model.type_of(elem) {
                row["definition"] = json!(ws.qualified_name_written(ty));
            }
            if let Some((file, range)) = ws.element_place(elem) {
                row["at"] = files.place(&ws, file, range.start().into());
            }
            row["satisfied_by"] = json!(satisfied_by);
            if !not_satisfied_by.is_empty() {
                row["not_satisfied_by"] = json!(not_satisfied_by);
            }
            row["verified_by"] = json!(verified_by);
            rows.push(row);
        }
        Ok(json!({
            "requirements": rows,
            "total": requirements.len(),
            "satisfied": requirements.len() - unsatisfied.len(),
            "verified": requirements.len() - unverified.len(),
            "unsatisfied": unsatisfied,
            "unverified": unverified,
        }))
    }
}

/// The relations whose source says something of its target that holds
/// only while the source is as it was: what a change to the source makes
/// someone check again of the target. A change to a part is a change to
/// whether the requirement it satisfies is satisfied.
const CLAIMS: [&str; 5] = [
    "satisfies",
    "does_not_satisfy",
    "verifies",
    "allocated_to",
    "connected_to",
];

/// How many elements `impact` lists at most. A link to something every
/// part uses reaches most of a large model, and an answer that size is
/// one nobody reads; `truncated` says it was cut.
const REACHED: usize = 1000;

/// Bytes as base64, the way an MCP image content carries them.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            out.push(match i <= chunk.len() {
                true => ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char,
                false => '=',
            });
        }
    }
    out
}

/// A range in a file, placed the way `check` places a finding, with the
/// text of the line it starts on -- which is what a reader of a list of
/// places looks at to tell them apart.
fn placed(ws: &Workspace, file: usize, range: sysml_syntax::TextRange, extra: Value) -> Value {
    let text = ws.file_parse(file).syntax().text().to_string();
    let mut placed = crate::at(&text, range, extra);
    placed["path"] = json!(ws.file_name(file));
    let start = usize::from(range.start()).min(text.len());
    let line_start = text[..start].rfind('\n').map_or(0, |at| at + 1);
    let line_end = text[start..].find('\n').map_or(text.len(), |at| start + at);
    placed["text"] = json!(text[line_start..line_end].trim());
    placed
}

/// The element a navigation call is about: the one it names as `element`,
/// or the one at `line` and `column` in the file it names.
fn asked_about(
    ws: &mut Workspace,
    open: &[usize],
    focus: Option<usize>,
    arguments: &Value,
) -> Result<ElementId, String> {
    if let Some(wanted) = optional_str(arguments, "element")? {
        return element_asked(ws, open, wanted);
    }
    let (file, offset) = position(ws, focus, arguments)?;
    ws.target_at(file, offset).ok_or_else(|| {
        "nothing is named there: point at a name, where it is declared or where it is used"
            .to_string()
    })
}

/// An element by the name a call gives it, as [`element_named`] finds one
/// -- or, failing that, as the name would resolve written at the top of a
/// file: `ISQ::MassValue` is how the library's own users write it, and
/// it is declared in `ISQBase` and only re-exported by `ISQ`.
fn element_asked(ws: &mut Workspace, open: &[usize], wanted: &str) -> Result<ElementId, String> {
    element_named(ws, open, wanted).or_else(|missed| {
        // A lookup starts in the scope around the element it is asked
        // from, so it is asked from one of the root packages: what is
        // around that is the global namespace a file's own top level sees.
        let Some(&top) = ws.model().owned(ws.root()).first() else {
            return Err(missed);
        };
        ws.resolve_from(top, &sysml_syntax::split_qualified(wanted))
            .ok_or(missed)
    })
}

/// The point `line` and `column` name in the file the call names.
fn position(
    ws: &Workspace,
    focus: Option<usize>,
    arguments: &Value,
) -> Result<(usize, sysml_syntax::TextSize), String> {
    if arguments.get("line").is_none() && arguments.get("column").is_none() {
        return Err(
            "give `element`, or `line` and `column` in the file `text` or `path` names".to_string(),
        );
    }
    let line = number(arguments, "line")?;
    let column = number(arguments, "column")?;
    let file =
        focus.ok_or_else(|| "give either `text` or `path`: a position is in a file".to_string())?;
    let text = ws.file_parse(file).syntax().text().to_string();
    let offset = offset_of(&text, line, column)
        .ok_or_else(|| format!("line {line}, column {column} is past the end of the text"))?;
    Ok((file, (offset as u32).into()))
}

/// One link between two elements, and where the text says so.
struct Edge {
    source: ElementId,
    relation: &'static str,
    target: ElementId,
    file: usize,
    offset: usize,
}

/// Every relation [`edges`] names a link by, which is what `find`'s
/// `without` may be given.
const RELATIONS: [&str; 16] = [
    "satisfies",
    "does_not_satisfy",
    "verifies",
    "allocated_to",
    "then",
    "connected_to",
    "imports",
    "annotated_by",
    "performs",
    "typed_by",
    "redefines",
    "references",
    "subsets",
    "specializes",
    "conjugates",
    "mentions",
];

/// Every link the opened files make, read off the resolved model.
///
/// Satisfy, verify, allocate, connect and succession are stored on the
/// relationship element itself; everything else -- perform, typing,
/// specialization, imports, metadata, a bare mention -- is a name that
/// resolved, classified by what the naming element is.
fn edges(ws: &Workspace, open: &[usize]) -> Vec<Edge> {
    use sysml_model::Value as V;
    let model = ws.model();
    let mut edges = Vec::new();
    // the relationship elements whose own references are already said by
    // the edge they make
    let mut said: HashSet<ElementId> = HashSet::new();
    let placed: Vec<_> = open
        .iter()
        .flat_map(|&file| ws.file_elements(file).iter().copied())
        .filter_map(|elem| Some((elem, ws.element_place(elem)?)))
        .collect();
    for &(elem, (file, range)) in &placed {
        let mut add = |source, relation, target| {
            edges.push(Edge {
                source,
                relation,
                target,
                file,
                offset: range.start().into(),
            })
        };
        let kind = model.kind(elem).name();
        if let Some(V::Ref(requirement)) = model.maybe(elem, "satisfiedRequirement") {
            let by = match model.maybe(elem, "satisfyingFeature") {
                Some(V::Ref(by)) => *by,
                _ => named_owner(model, elem),
            };
            let relation = match model.flag(elem, "isNegated") {
                true => "does_not_satisfy",
                false => "satisfies",
            };
            add(by, relation, *requirement);
            said.insert(elem);
        }
        if let Some(V::RefList(requirements)) = model.maybe(elem, "verifiedRequirement") {
            for &requirement in requirements {
                add(elem, "verifies", requirement);
            }
        }
        // a connection of more than two ends is not a link between two
        if let Some(&[from, to]) = model.maybe(elem, "relatedFeature").and_then(V::as_ids) {
            let relation = match kind {
                "AllocationUsage" | "AllocationDefinition" => "allocated_to",
                _ if kind.contains("Succession") => "then",
                _ => "connected_to",
            };
            add(from, relation, to);
            said.insert(elem);
        }
    }
    let typed: HashSet<(ElementId, ElementId)> =
        edges.iter().map(|e| (e.source, e.target)).collect();

    // A qualified name `a::b` resolves each segment, so `a` is recorded too;
    // only the reference that reaches furthest is what was written.
    let mut reach: std::collections::HashMap<(usize, ElementId, u32), u32> = Default::default();
    for r in ws.references().iter().filter(|r| open.contains(&r.file)) {
        let end = reach
            .entry((r.file, r.from, r.range.start().into()))
            .or_default();
        *end = (*end).max(r.range.end().into());
    }
    for r in ws.references().iter().filter(|r| open.contains(&r.file)) {
        if said.contains(&r.from)
            || reach[&(r.file, r.from, r.range.start().into())] != u32::from(r.range.end())
        {
            continue;
        }
        // `perform x;` borrows the name `x`, but what performs it is its owner
        let source = match model.kind(r.from) {
            sysml_model::ElementKind::PerformActionUsage => {
                named_owner(model, model.owner(r.from).unwrap_or(r.from))
            }
            _ => named_owner(model, r.from),
        };
        if typed.contains(&(source, r.target)) {
            continue;
        }
        let relation = match model.kind(r.from).name() {
            "NamespaceImport" | "MembershipImport" => "imports",
            "MetadataUsage" => "annotated_by",
            "PerformActionUsage" => "performs",
            _ => how_named(model, r.from, r.target),
        };
        edges.push(Edge {
            source,
            relation,
            target: r.target,
            file: r.file,
            offset: r.range.start().into(),
        });
    }
    // a name added here and not there is one `without` refuses
    debug_assert!(edges.iter().all(|e| RELATIONS.contains(&e.relation)));
    edges
}

/// What a reference from `from` to `target` is, by the relationship that
/// carries it: `: T` is a typing, `:>` a specialization or subsetting,
/// `:>>` a redefinition. A name used any other way is only mentioned.
fn how_named(model: &sysml_model::Model, from: ElementId, target: ElementId) -> &'static str {
    for &rel in model.owned(from) {
        let points = model.props(rel).any(|(_, v)| {
            v.as_id() == Some(target) || v.as_ids().is_some_and(|ids| ids.contains(&target))
        });
        if !points {
            continue;
        }
        let kind = model.kind(rel).name();
        if kind.ends_with("Typing") {
            return "typed_by";
        }
        if kind == "Redefinition" {
            return "redefines";
        }
        if kind == "ReferenceSubsetting" {
            return "references";
        }
        if kind == "Subsetting" {
            return "subsets";
        }
        if kind == "Subclassification" || kind == "Specialization" {
            return "specializes";
        }
        if kind == "Conjugation" || kind == "PortConjugation" {
            return "conjugates";
        }
    }
    "mentions"
}

/// The element a relationship or anonymous usage belongs to: itself where it
/// has a name -- its own, or the one `:>> engine` borrows -- else the nearest
/// owner that does.
fn named_owner(model: &sysml_model::Model, elem: ElementId) -> ElementId {
    let mut at = elem;
    loop {
        match (model.naming_element(at).is_some(), model.owner(at)) {
            (false, Some(up)) => at = up,
            _ => return at,
        }
    }
}

/// An element by qualified name, or by short name -- `SR-001` is how a
/// trace matrix and a review comment name it.
fn element_named(ws: &mut Workspace, open: &[usize], name: &str) -> Result<ElementId, String> {
    if let Some(found) = ws.named_globally(name) {
        return Ok(found);
    }
    let short = name.trim_matches(|c| c == '<' || c == '>' || c == '\'');
    let found: Vec<ElementId> = open
        .iter()
        .flat_map(|&file| ws.file_elements(file).iter().copied())
        .filter(|&elem| {
            ws.model().effective_short_name(elem) == Some(short)
                // a redefinition answers to the name it borrows, which
                // `named_globally` does not know
                || sysml_syntax::split_qualified(&qualified(ws, elem))
                    == sysml_syntax::split_qualified(name)
        })
        .collect();
    match found[..] {
        [one] => Ok(one),
        [] => Err(format!(
            "nothing is named `{name}`: give a qualified name (`find` lists them) or a short name"
        )),
        _ => Err(format!(
            "`{name}` names {} elements: {}",
            found.len(),
            found
                .iter()
                .map(|&e| qualified(ws, e))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// An element as a list names it: what it is called, what it is, and where.
fn summary(ws: &Workspace, files: &mut Texts, elem: ElementId) -> Value {
    let model = ws.model();
    let mut entry = json!({
        "name": qualified(ws, elem),
        "kind": model.kind(elem).name(),
    });
    if let Some(short) = model.effective_short_name(elem) {
        entry["shortName"] = json!(short);
    }
    if let Some((file, range)) = ws.element_place(elem) {
        entry["at"] = files.place(ws, file, range.start().into());
    }
    entry
}

/// The owner path by effective names, so `part :>> engine;` reads as the
/// `engine` it is rather than `?`.
fn qualified(ws: &Workspace, elem: ElementId) -> String {
    let model = ws.model();
    let mut segments = Vec::new();
    let mut at = Some(elem);
    while let Some(e) = at.filter(|&e| e != ws.root()) {
        segments.push(model.effective_name(e).unwrap_or("?"));
        at = model.owner(e);
    }
    segments.reverse();
    // written the way it would have to be, so that what a call hands
    // back can be handed back in: `P q::x` is no name at all
    sysml_syntax::quote_qualified(segments)
}

/// The text of each file a call places something in, read once per call.
#[derive(Default)]
struct Texts(std::collections::HashMap<usize, String>);

impl Texts {
    /// `path:line`, which is what an editor and a person both open.
    fn place(&mut self, ws: &Workspace, file: usize, offset: usize) -> Value {
        let text = self
            .0
            .entry(file)
            .or_insert_with(|| ws.file_parse(file).syntax().text().to_string());
        let (line, _) = sysml_syntax::line_col(text, offset);
        json!(format!("{}:{line}", ws.file_name(file)))
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
///
/// Given as anything but a list of paths it is refused: a single path
/// written as a string used to be dropped without a word, and the model
/// checked without the files it was meant to be checked with.
fn alongside(arguments: &Value) -> Result<Vec<String>, String> {
    let refused = || "`alongside` must be an array of paths".to_string();
    match arguments.get("alongside") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(paths)) => paths
            .iter()
            .map(|path| path.as_str().map(str::to_string).ok_or_else(refused))
            .collect(),
        Some(_) => Err(refused()),
    }
}

/// An argument that is a string where it is given at all. One of another
/// type is refused rather than read as absent: `text: 42` answered about
/// the project instead, as if no text had been sent.
fn optional_str<'a>(arguments: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(it)) => Ok(Some(it)),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
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
                "name": ws.qualified_name_written(elem),
                "kind": ws.model().kind(names).name(),
            });
            if names != elem {
                entry["alias_for"] = json!(ws.qualified_name_written(names));
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
        "name": ws.qualified_name_written(elem),
        "kind": model.kind(elem).name(),
    });
    if let Some(ty) = model.type_of(elem) {
        entry["type"] = json!(ws.qualified_name_written(ty));
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
            .map(|&up| ws.qualified_name_written(up))
            .collect::<Vec<_>>());
    }
    // depth 0 asked for the element and nothing inside it, and used to
    // be answered as 1
    if depth == 0 {
        return entry;
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
                "name": ws.qualified_name_written(child),
                "kind": ws.model().kind(child).name(),
            })
        });
    }
    if !inside.is_empty() {
        entry["members"] = json!(inside);
    }
    entry
}

/// The workspace one call is answered against, resolved.
#[derive(Clone)]
struct Opened {
    ws: Workspace,
    /// Every file opened over the library: the project's and the call's.
    open: Vec<usize>,
    /// The file the call named, where it named one. What a position is
    /// measured in.
    focus: Option<usize>,
    /// What resolving `open` came to.
    stats: sysml_semantics::ResolveStats,
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
///
/// The column is in characters, as the columns of a finding are, so a
/// place `check` reports can be handed straight back.
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
    let at = rest
        .char_indices()
        .map(|(at, _)| at)
        .chain([rest.len()])
        .nth(column)?;
    Some(start + at)
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
    // The shape of a surface is the shape of the `import_code` call, so
    // the schema a front end's author reads is the one a client builds
    // the call from. Its definitions move to the root of the call's
    // schema, since that is where a `$ref` in it points.
    let mut surface_schema = sysml_rust::surface::schema();
    let surface_definitions = surface_schema
        .as_object_mut()
        .and_then(|schema| {
            schema.remove("$schema");
            schema.remove("title");
            schema.remove("$defs")
        })
        .unwrap_or(json!({}));
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
    let mut every = json!([
        {
            "name": "check",
            "description": "Parse a SysML v2 / KerML model, resolve every name in it against the standard library, and run the well-formedness constraints the specification itself states. Answers which references resolve to nothing and where -- across every file that was opened, each finding under its own path, each with what it might have meant: `declared_as` names the elements that answer to it somewhere, so the name is right and wants an import, while `did_you_mean` names what is near it, so the name itself is wrong -- which root packages of your own take a name the standard library has already taken (`collisions`: both sides still resolve, but the name means one thing in your model and another in the library), and which constraints the model breaks. `library` and `project` say what was loaded, or are null; without the library every reference into it reads as unresolved. The constraints are put only to a model that parses, resolves and has the library to be asked against -- one asked with a name still dangling answers about the hole and not about the model -- so read `rules.asked` before reading an empty `rules.violations` as a clean bill. `rules.unevaluated` counts the constraints that could not be asked at all, and `rules.partly` those some elements could not answer -- a violation among those is not reported -- both of which say what this toolchain does not yet build rather than anything about the model. Use this on anything you write before believing it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": source_properties["text"],
                    "path": source_properties["path"],
                    "name": source_properties["name"],
                    "alongside": source_properties["alongside"],
                    "focus": { "type": "boolean", "description": "Report only what is found in the file `text` or `path` names, while still resolving it against everything else (default false: every file opened). The constraints are then put to that file's elements alone, so `rules.held`, `rules.partly` and `rules.unevaluated` count what was asked of it -- and a large project answers in a fraction of the time. Use it to ask about one edit in a large project." },
                },
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
                    "column": { "type": "integer", "description": "Column in characters, counting from one" },
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
                    "depth": { "type": "integer", "description": "How many levels of nesting to descend: 0 is the element alone, 1 names what is directly inside it (default 2, at most 16)" },
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
            "name": "import_code",
            "description": "State existing code in any language as a SysML package, the way `import_rust` states a crate: every definition carrying the `@code { ... }` metadata that names the language and the item it binds to, so a model can type its ports and `perform` its actions against what already exists. The input is a surface -- what a reader of the code says about it, in no language in particular: its records, enumerations, interfaces and functions, each with the name the code gives it and the path to it, typed with the five scalars, a scalar the surface declares as a kind of one, or a type the surface declares. `sysml import-rust --surface` shows the one written for a real crate. Write one yourself for a language no tool reads yet, from what you can see of the code; everything SysML decides -- which names are quoted, told apart, or refused for naming what nothing declares -- is decided here, not by you. `skipped` names what had no shape, and `checked` says whether every name in the package resolves.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "surface": surface_schema,
                    "path": { "type": "string", "description": "A file holding the surface JSON, instead of `surface`" },
                    "package": { "type": "string", "description": "What to call the package written (default: what the surface says, or derived from its module)" },
                },
                "$defs": surface_definitions,
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
        {
            "name": "relations",
            "description": "Everything one element of the model is linked to, in both directions: `satisfies` / `does_not_satisfy`, `verifies`, `allocated_to`, `performs`, `connected_to`, `then` (succession), `typed_by`, `specializes`, `subsets`, `redefines`, `references`, `imports`, `annotated_by` (metadata), and `mentions` for a name used any other way. `incoming` is what a file cannot tell you -- which parts satisfy this requirement, which test verifies it, which component a function is allocated to, who uses this definition. Each link says where it is written (`path:line`). Only links written in the model's own files are listed, not the standard library's.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "element": { "type": "string", "description": "A qualified name (`VehicleRequirements::range`) or a short name (`SR-001`)" },
                    "text": source_properties["text"],
                    "path": source_properties["path"],
                    "name": source_properties["name"],
                    "alongside": source_properties["alongside"],
                },
                "required": ["element"],
            },
        },
        {
            "name": "find",
            "description": "List the model's named elements that pass every filter given, each with its qualified name, metaclass, short name, documentation and `path:line`. `kind` matches the start of the metaclass, case-insensitively: `Requirement` finds requirement definitions and usages, `PartUsage` only part usages. `within` keeps what is nested under one element. `query` matches part of the name or short name. `without` keeps the elements that are on neither end of a relation `relations` names -- `kind: RequirementUsage, without: verifies` is every requirement no test verifies, `kind: ActionUsage, without: allocated_to` every function not allocated. `total` counts all matches, even past `limit`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "description": "The start of a metaclass name, case-insensitive" },
                    "within": { "type": "string", "description": "A qualified or short name; only what is nested under it" },
                    "query": { "type": "string", "description": "Part of the name or short name, case-insensitive" },
                    "without": { "type": "string", "description": "A relation (as `relations` names them) the element must not take part in" },
                    "limit": { "type": "integer", "description": "At most this many (default 50, at most 500)" },
                    "text": source_properties["text"],
                    "path": source_properties["path"],
                    "name": source_properties["name"],
                    "alongside": source_properties["alongside"],
                },
            },
        },
        {
            "name": "format",
            "description": "The file as `sysml fmt` lays it out: indentation, spacing and line breaks, with nothing it says changed. The formatted text is answered, not written -- write it yourself if you want it. Text that does not parse is refused rather than re-spaced, since moving tokens the parser could not follow can change what they say. `changed` says whether formatting made any difference.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": source_properties["text"],
                    "path": { "type": "string", "description": "A file to read instead of `text`" },
                    "name": source_properties["name"],
                    "width": { "type": "integer", "description": "The line width to fit, 0 for none (default that of `sysml fmt`)" },
                },
            },
        },
        {
            "name": "definition",
            "description": "Where an element is declared and what it is: its qualified name (which every other tool takes back as `element`), metaclass, short name, type, what an alias stands for, documentation, and `declared` -- the path and the line/column range of the name it is declared with, or null for an element the toolchain made rather than anyone wrote. Ask about a name as it is used, by `line` and `column` in the file `text` or `path` gives, or about an element by `element`. Use this to find out what a name in front of you actually resolves to -- which may be something in the standard library, or an inherited feature a definition never spells out -- instead of guessing from the spelling.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "element": { "type": "string", "description": "A qualified name (`Vehicle::engine`) or a short name, instead of a position" },
                    "line": { "type": "integer", "description": "Line of the name, counting from one, in the file `text` or `path` gives" },
                    "column": { "type": "integer", "description": "Column in characters, counting from one -- where `check` places a finding is a place this takes" },
                    "text": source_properties["text"],
                    "path": source_properties["path"],
                    "name": source_properties["name"],
                    "alongside": source_properties["alongside"],
                },
            },
        },
        {
            "name": "references",
            "description": "Every place the model names an element, each with its path, line/column range and the text of that line -- the declaration first, marked `declaration: true` (and `library: true` where it is in the standard library), then every use in the model's own files in the order written: as a type, a specialization, a redefinition, an import, a step of a qualified name or a feature chain, and through a feature that borrows the name by redefining it without one of its own (`l.component` where `l` holds `:>> component`). These are exactly the places `rename` rewrites. Uses inside the standard library are counted in `in_library` rather than listed. Ask about an element by `element`, or by `line` and `column` in the file `text` or `path` gives. Ask this before changing or deleting a declaration, to know what depends on it; searching the text for the name misses what reaches it through inheritance and finds what merely shares its spelling.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "element": { "type": "string", "description": "A qualified name or a short name, instead of a position" },
                    "line": { "type": "integer", "description": "Line of the name, counting from one, in the file `text` or `path` gives" },
                    "column": { "type": "integer", "description": "Column in characters, counting from one" },
                    "text": source_properties["text"],
                    "path": source_properties["path"],
                    "name": source_properties["name"],
                    "alongside": source_properties["alongside"],
                },
            },
        },
        {
            "name": "rename",
            "description": "The complete set of text edits that renames an element of the model everywhere it is named -- its declaration, every reference, every step of a qualified name or feature chain through it, and every redefinition that borrows its name -- the same edits the language server makes, with the new name quoted in each file where that file's notation needs it (`'two words'`, or a word that is a keyword in SysML but not in KerML). Nothing is written: this answers the edits, under `files` (each a `path` and its `edits`, each a range in that file as it is now and the `newText` to put there, in the order they appear -- apply them last first, or all at once, so the earlier ranges hold), and you write them yourself. `checked` is the rename already tried: the edits made in memory and the model resolved again, `ok` when nothing parses wrong and no name dangles that did not before (`introduced` lists any that do). Asked at a short name (`<W>`), it renames the short name. A new name that is not one legal name, one already visible where the declaration or a use of it stands (which the rename would capture), an alias, an element declared in the standard library, and one reached some way no rename can follow are each refused with why. Rename with this rather than by search and replace, which renames whatever shares the spelling and misses what reaches the element through inheritance.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "new_name": { "type": "string", "description": "The name to give it, written as one name is written in the file the rename is asked in: `Wheel`, or `'Wheel Unit'` quoted, since unquoted that is two names. Another file that would not read it as one name gets it quoted." },
                    "element": { "type": "string", "description": "A qualified name or a short name, instead of a position" },
                    "line": { "type": "integer", "description": "Line of a name, counting from one, in the file `text` or `path` gives -- at the declaration or at any use" },
                    "column": { "type": "integer", "description": "Column in characters, counting from one" },
                    "text": source_properties["text"],
                    "path": source_properties["path"],
                    "name": source_properties["name"],
                    "alongside": source_properties["alongside"],
                },
                "required": ["new_name"],
            },
        },
        {
            "name": "diagram",
            "description": "Draw the model as `sysml diagram` draws it, and answer the drawing as SVG text under `svg` -- written nowhere: save it or show it yourself. `view` picks the drawing: `definitions` (the default) boxes every definition with its features and draws the lines between them -- specialization, composition, reference, connection, flow, allocation, satisfaction and the rest; `internal` is the inside of one definition named by `element`, its parts and the connections between them; `browser` the membership tree, indented; `sequence` the lifelines and messages of the interaction one definition (`element`) declares. `within` narrows `definitions` and `browser` to what is under one element. `drawn` says what the picture holds -- each box by qualified name and how many of each kind of line, or the lifelines and the number of messages -- so you can describe it without reading the SVG. `warnings` says where the model does not parse or a name resolves to nothing, since such a drawing quietly leaves those parts out; it is drawn all the same. `skin` paints it: `default`, `mono`, `contrast`, a path to a JSON skin, or the skin itself as an object. Use this when someone wants to see a model, or to check that what was written has the shape that was meant.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "view": { "type": "string", "enum": ["definitions", "internal", "browser", "sequence"], "description": "Which drawing (default `definitions`)" },
                    "element": { "type": "string", "description": "The definition an `internal` or `sequence` view is of: a qualified name or a short name" },
                    "within": { "type": "string", "description": "Draw only what is under this element, for `definitions` and `browser`" },
                    "skin": { "description": "`default`, `mono` or `contrast`, a path to a JSON skin, or the skin as an object" },
                    "image": { "type": "boolean", "description": "Also attach the SVG as an image content (image/svg+xml), for a client that draws SVG. Off by default: a client that passes images on to a model that takes only PNG, JPEG, GIF or WebP refuses it." },
                    "text": source_properties["text"],
                    "path": source_properties["path"],
                    "name": source_properties["name"],
                    "alongside": source_properties["alongside"],
                },
            },
        },
        {
            "name": "impact",
            "description": "What a change to one element reaches, transitively: with `direction: affected` (the default), everything that rests on it -- whoever specializes it, is typed by it, subsets, redefines or references it, performs, connects to or mentions it -- and everything that rests on those, as far as `depth` links (default 6); and what it claims of others has to be claimed again, so the requirement a changed part satisfies, the requirement a changed test verifies and the component a changed function is allocated to are reached too, as is the whole a changed feature belongs to (`owns`). A change to the element asked about is a change to everything inside it, so what rests on its members is reached as well. With `direction: depends_on`, the other way: everything it, or anything inside it, rests on. Each element reached comes with its `path:line`, how many links away it is, and `via` -- the chain of links that reached it first, each as `from relation to` and where that link is written -- so the answer can be checked rather than trusted. An element of the standard library is listed where it is reached but not followed further. `stopped_at_depth` says the walk stopped at `depth` with elements still to follow from, and `truncated` that it stopped at 1000 elements. `relations` limits which links are followed (as `relations` names them, plus `owns`). Ask this before changing or deleting an element, or when reviewing a change, to know what else has to be looked at again; `references` lists only the direct uses.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "element": { "type": "string", "description": "A qualified name or a short name" },
                    "direction": { "type": "string", "enum": ["affected", "depends_on"], "description": "`affected` (default): what a change to it reaches. `depends_on`: what it rests on." },
                    "depth": { "type": "integer", "description": "How many links to follow at most (default 6, at most 32)" },
                    "relations": { "type": "array", "items": { "type": "string" }, "description": "Follow only these relations, as `relations` names them, plus `owns`" },
                    "text": source_properties["text"],
                    "path": source_properties["path"],
                    "name": source_properties["name"],
                    "alongside": source_properties["alongside"],
                },
                "required": ["element"],
            },
        },
        {
            "name": "trace",
            "description": "The requirement traceability matrix in one answer: every requirement of the model -- definitions and usages, concerns among them, but not a case's objective, a satisfaction or a viewpoint -- with its qualified name, short name, documentation (the requirement's text), its definition where it is a usage, its `path:line`, `satisfied_by` (what the model says satisfies it, each with where that is written), `not_satisfied_by` where an `assert not satisfy` says otherwise, and `verified_by` (the verification cases whose objective verifies it). Then `total`, how many are `satisfied` and `verified`, and `unsatisfied` and `unverified` -- the requirements nothing satisfies and nothing verifies, which is what a review looks for. Only what the model states directly counts: satisfying a requirement definition is not read as satisfying each usage of it, nor a group as satisfying the requirements inside it. `within` keeps the requirements under one package or element. Use this for a coverage review or before reporting that requirements are met; `find` with `without` answers one column of it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "within": { "type": "string", "description": "A qualified or short name; only the requirements under it" },
                    "text": source_properties["text"],
                    "path": source_properties["path"],
                    "name": source_properties["name"],
                    "alongside": source_properties["alongside"],
                },
            },
        },
    ]);
    // Every tool here only reads: none writes a file, none reaches past
    // this machine, and asking twice answers the same. Said, a client
    // may run them without asking its user each time.
    for tool in every.as_array_mut().into_iter().flatten() {
        let title = tool["name"].as_str().unwrap_or_default().replace('_', " ");
        tool["annotations"] = json!({
            "title": title,
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false,
        });
    }
    every
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

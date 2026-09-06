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
//! None of them generate anything. What they do is tell the truth about
//! a model, which is the part a language model cannot supply for itself.
//!
//! The transport is the MCP stdio one: JSON-RPC 2.0, one message per
//! line. [`serve`] runs the loop over any reader and writer, so a test
//! drives it the same way a client does.

use std::io::{BufRead, Write};
use std::path::Path;

use serde_json::{json, Value};

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
}

impl Server {
    /// A server whose answers are grounded in the standard library at
    /// `library`. Without one it still parses and resolves, but every
    /// reference into the library reads as unresolved, so a client that
    /// has the library should say where it is.
    pub fn new(library: Option<&Path>) -> Server {
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
        Server {
            base,
            library: loaded,
        }
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
            "library_search" => Ok(self.library_search(&arguments)),
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

    /// Does it parse, and does every name in it resolve? The findings
    /// carry the shape `sysml check --format json` reports, so a client
    /// that knows one knows the other.
    fn check(&mut self, arguments: &Value) -> Result<Value, String> {
        let (name, text) = source(arguments)?;
        let mut ws = self.base.clone();
        let mut open = Vec::new();
        for path in alongside(arguments) {
            let text = std::fs::read_to_string(&path)
                .map_err(|err| format!("cannot read `{path}`: {err}"))?;
            open.push(ws.add_file(path, &text));
        }
        let file = ws.add_file(name.clone(), &text);
        open.push(file);

        // The answer is about the whole model that was opened -- the
        // file asked about and the `alongside` ones it is spread over --
        // because that is what the reference counts are over. Each
        // finding says which file it is in, so a sibling that does not
        // resolve is reported under its own path instead of being
        // counted in `references` and then left out of `unresolved`.
        //
        // The name and the text of each open file are held here so that
        // a finding can be placed after `ws` has been borrowed to
        // resolve.
        let opened: Vec<(String, String)> = open
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
            let (name, text) = &opened[which];
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
        let (name, text) = source(arguments)?;
        let line = number(arguments, "line")?;
        let column = number(arguments, "column")?;
        let offset = offset_of(&text, line, column)
            .ok_or_else(|| format!("line {line}, column {column} is past the end of the text"))?;

        let mut ws = self.base.clone();
        let mut open = Vec::new();
        for path in alongside(arguments) {
            let text = std::fs::read_to_string(&path)
                .map_err(|err| format!("cannot read `{path}`: {err}"))?;
            open.push(ws.add_file(path, &text));
        }
        let file = ws.add_file(name, &text);
        open.push(file);
        ws.resolve_files(&open);
        let names: Vec<Value> = ws
            .visible_names(file, (offset as u32).into())
            .into_iter()
            .map(|(name, kind)| json!({ "name": name, "kind": kind.name() }))
            .collect();
        Ok(json!({ "visible": names }))
    }

    /// What the standard library has under that name. A model asked to
    /// use a quantity or a port will otherwise invent one.
    fn library_search(&self, arguments: &Value) -> Value {
        let query = arguments
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .min(200) as usize;
        let mut found: Vec<Value> = Vec::new();
        for elem in self.base.search_names(query, limit) {
            let mut entry = json!({
                "name": self.base.qualified_name_of(elem),
                "kind": self.base.model().kind(elem).name(),
            });
            if let Some(doc) = self.base.documentation_of(elem) {
                entry["documentation"] = json!(doc);
            }
            found.push(entry);
        }
        json!({ "found": found })
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
/// path. The name decides the dialect, so a `.kerml` file is read as
/// KerML whichever way it arrives.
fn source(arguments: &Value) -> Result<(String, String), String> {
    if let Some(text) = arguments.get("text").and_then(Value::as_str) {
        let name = arguments
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("model.sysml");
        return Ok((name.to_string(), text.to_string()));
    }
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "give either `text` or `path`".to_string())?;
    let text =
        std::fs::read_to_string(path).map_err(|err| format!("cannot read `{path}`: {err}"))?;
    Ok((path.to_string(), text))
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
        "text": { "type": "string", "description": "The model source itself, where it is not on disk yet" },
        "path": { "type": "string", "description": "A file to read instead of `text`" },
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
            "description": "Parse a SysML v2 / KerML model and resolve every name in it against the standard library. Answers which references resolve to nothing, and where -- across the file asked about and every `alongside` file, each finding under its own path. `library` says where the standard library was loaded from, or is null when there is none and references into it will read as unresolved. Use this on anything you write before believing it.",
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
            "name": "library_search",
            "description": "Search the SysML v2 standard library by name. Answers the qualified name, the metaclass and the documentation of each match -- what a quantity, port or action definition is actually called.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Matched case-insensitively against the declared name" },
                    "limit": { "type": "integer", "description": "At most this many matches (default 20, at most 200)" },
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

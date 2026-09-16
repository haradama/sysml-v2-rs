//! The `sysml` tool: every subcommand a person types.
//!
//! What something other than a person drives -- the MCP server, the
//! reporting a program reads -- is in the library beside this, so that
//! its tests drive it rather than a subprocess.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use sysml_cli::report::{self, Said, Severity};
use sysml_syntax::fmt::Layout;
use sysml_syntax::{Diagnostic, Dialect};

mod api;

fn parse_file(path: &Path, text: &str) -> sysml_syntax::Parse {
    let dialect = path
        .extension()
        .and_then(|e| e.to_str())
        .map(Dialect::from_extension)
        .unwrap_or_default();
    sysml_syntax::parse_dialect(text, dialect)
}

/// What `--version` prints.
///
/// The standard library moves with the specification, so which release a
/// model was resolved against is part of what produced an answer. Built
/// once rather than `concat!`ed: the release is a `const` in another crate
/// and `concat!` takes literals.
fn version() -> &'static str {
    static IT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    IT.get_or_init(|| {
        format!(
            "{} (standard library {})",
            env!("CARGO_PKG_VERSION"),
            sysml_stdlib::RELEASE
        )
    })
}

#[derive(Parser)]
#[command(name = "sysml", version = version(), about = "SysML v2 command-line tools")]
struct Cli {
    /// How findings are reported: for a person, or as JSON for a program
    #[arg(long, value_enum, default_value_t = Format::Text, global = true)]
    format: Format,
    /// Resolve against no standard library at all, not even the copy
    /// built in -- which is what a model looks like to a tool that
    /// cannot find one
    #[arg(long, global = true)]
    no_library: bool,
    /// Whether findings are coloured: by default when a terminal is
    /// reading them and `NO_COLOR` is unset
    #[arg(long, value_enum, default_value_t = Colour::Auto, global = true)]
    color: Colour,
    #[command(subcommand)]
    command: Command,
}

/// What `parse`, `check`, `stats` and `fmt --check` report in. The
/// artifacts of `export`, `diagram` and `rustgen` are machine-readable
/// already and are unaffected.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Format {
    Text,
    Json,
}

#[derive(Subcommand)]
enum Command {
    /// Parse .sysml/.kerml files and report syntax errors
    Parse {
        /// Files to parse
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Dump the syntax tree of each file
        #[arg(long)]
        tree: bool,
    },
    /// Parse files, build the element model and print element counts by kind
    Stats {
        /// Files to analyze
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Parse files, build the element model and write standard interchange
    /// JSON to stdout (or a file)
    Export {
        /// Files to export
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Resolve names against these files or directories too
        #[arg(long)]
        library: Vec<PathBuf>,
        /// Write the library into the document as well, marked
        /// `isLibraryElement` -- a document that stands on its own,
        /// rather than one whose references into the library are for the
        /// reader to resolve
        #[arg(long)]
        include_library: bool,
        /// Write to this file instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Format .sysml/.kerml files (prints to stdout by default)
    Fmt {
        /// Files to format
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Rewrite the files in place
        #[arg(short, long)]
        write: bool,
        /// Exit non-zero if any file is not already formatted
        #[arg(long)]
        check: bool,
        /// Columns a line may reach before it is broken at the readiest
        /// joint of what is on it; 0 leaves every line as long as it comes
        #[arg(long, default_value_t = 100, value_name = "COLUMNS")]
        width: usize,
    },
    /// Load files (or directories) into one workspace, resolve all names and
    /// report unresolved references, then check what the specification
    /// requires of a model whose names all resolve (against the standard
    /// library built in, or one named as a path)
    Check {
        /// Files or directories to load
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Show each unresolved reference (up to N; 0 = all)
        #[arg(long, default_value_t = 20)]
        show: usize,
    },
    /// What the model implies for code, in no language in particular:
    /// every definition's shape, its features with their multiplicities
    /// and what they bottom out in, and what happens in it
    Plan {
        /// Files or directories to plan
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },
    /// Load files (or directories), resolve names and render the definitions
    /// and their specializations as an SVG diagram
    Diagram {
        /// Files or directories to draw
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Also load these files or directories so names resolve against
        /// them, without drawing their definitions (e.g. sysml.library)
        #[arg(long)]
        library: Vec<PathBuf>,
        /// Draw the internal structure of this definition -- the parts it is
        /// assembled from and the connections between them -- instead of the
        /// definitions themselves
        #[arg(long, value_name = "NAME")]
        internal: Option<String>,
        /// Draw the membership hierarchy as an indented tree instead of a
        /// diagram of relationships
        #[arg(long, conflicts_with = "internal")]
        browser: bool,
        /// Draw the interaction this definition declares as a sequence
        /// view: a lifeline per participant and the messages between them
        #[arg(long, value_name = "NAME", conflicts_with_all = ["internal", "browser"])]
        sequence: Option<String>,
        /// How the drawing is painted: a skin that ships (`default`,
        /// `mono`, `contrast`) or a JSON file saying what to paint it in
        #[arg(long, value_name = "NAME|FILE")]
        skin: Option<String>,
        /// Write to this file instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Read standard interchange JSON back into a model: what `export`
    /// writes, and what a SysML v2 API & Services model server hands out
    Import {
        /// The interchange document
        json: PathBuf,
        /// Draw it instead of counting it: the definitions it holds and
        /// the specializations between them, as SVG
        #[arg(long, value_name = "FILE")]
        diagram: Option<PathBuf>,
        /// How the drawing is painted: a skin that ships (`default`,
        /// `mono`, `contrast`) or a JSON file saying what to paint it in
        #[arg(long, value_name = "NAME|FILE", requires = "diagram")]
        skin: Option<String>,
    },
    /// Import a Rust crate's public API as a SysML package with `@code`
    /// binding metadata, from the JSON `cargo +nightly rustdoc --
    /// -Zunstable-options --output-format json` writes
    ImportRust {
        /// The crate's rustdoc JSON file
        json: PathBuf,
        /// Name of the generated package (default: derived from the crate)
        #[arg(long)]
        package: Option<String>,
        /// Write to this file instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Generate Rust from a resolved model: parts become structs and the
    /// `perform`ed actions of imported `@code`-bound APIs become methods
    Rustgen {
        /// Files or directories holding the model to generate for
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Also load these files or directories so names resolve, without
        /// generating for them (imported API packages, libraries)
        #[arg(long)]
        library: Vec<PathBuf>,
        /// Write to this file instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Speak the Model Context Protocol over stdin and stdout, so an
    /// agent can ask whether a model parses and resolves, what names are
    /// legal at a point, what shape a definition has, what the standard
    /// library declares, and can have an existing Rust API stated as
    /// SysML or the Rust a model implies written for it
    Mcp {
        /// The standard library, so that references into it resolve;
        /// `SYSML_LIBRARY_PATH` says the same thing
        #[arg(long)]
        library: Option<PathBuf>,
        /// The model being worked on, so a call that names no source of
        /// its own is about it and one that names a file is read against
        /// the rest; `SYSML_PROJECT_PATH` says the same thing
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Talk to a SysML v2 API & Services model server
    Api {
        #[command(subcommand)]
        what: ApiCommand,
        /// Base URL of the model server
        #[arg(long, default_value = "http://localhost:9000", global = true)]
        server: String,
        /// Give up on a request that has taken this long, in seconds
        #[arg(long, default_value_t = 30, global = true, value_name = "SECONDS")]
        timeout: u64,
    },
    /// Parse every .sysml/.kerml file under a directory and report the
    /// success rate (used to track grammar coverage against the official
    /// SysML-v2-Release corpus)
    Corpus {
        /// Directory to scan recursively
        dir: PathBuf,
        /// Show the N files with the most errors
        #[arg(long, default_value_t = 10)]
        worst: usize,
        /// List every failing file
        #[arg(long)]
        failures: bool,
    },
}

#[derive(Subcommand)]
enum ApiCommand {
    /// List the projects the server holds
    Projects,
    /// Show one project
    Project {
        /// Project id
        project: String,
    },
    /// Create a project
    NewProject {
        /// What to call it
        name: String,
    },
    /// List the commits of one project
    Commits {
        /// Project id
        project: String,
    },
    /// List the elements of one commit
    Elements {
        /// Project id
        project: String,
        /// Commit id
        commit: String,
    },
    /// Show one element of one commit
    Element {
        /// Project id
        project: String,
        /// Commit id
        commit: String,
        /// Element id
        element: String,
    },
    /// Export a model and send it to the server as a new commit
    Push {
        /// Files or directories holding the model
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Project to commit to
        #[arg(long)]
        project: String,
        /// What the commit is for
        #[arg(long, default_value = "sysml push")]
        message: String,
        /// Resolve names against these files or directories too
        #[arg(long)]
        library: Vec<PathBuf>,
        /// Send the library along as well, marked `isLibraryElement`.
        /// Without this the commit is the model, and its references into
        /// the library are for the server to resolve
        #[arg(long)]
        include_library: bool,
    },
}

/// Whether findings are coloured, asked once.
///
/// `check`, `parse`, `export` and `api push` can all print a finding and
/// none of them is about display. Threading a bool down four call chains
/// to reach the one line that draws it puts a display decision in the
/// signature of everything, so it is settled before any command runs.
static COLOUR: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// When to colour what is said to a person.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Colour {
    /// When a terminal is reading, and `NO_COLOR` is unset.
    Auto,
    Always,
    Never,
}

impl Colour {
    /// Whether to colour, now, on this stream.
    ///
    /// Findings go to stderr, so that is the stream to ask: colouring by what
    /// stdout happens to be gives a person escape codes in a log. `NO_COLOR`
    /// is honoured -- said once, it should not have to be said per tool.
    fn wanted(self) -> bool {
        match self {
            Colour::Always => true,
            Colour::Never => false,
            Colour::Auto => {
                std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal()
            }
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let format = cli.format;
    let bare = cli.no_library;
    COLOUR
        .set(cli.color.wanted())
        .expect("set once, before any command");
    match cli.command {
        Command::Parse { files, tree } => parse_files(&files, tree, format),
        Command::Stats { files } => stats(&files, format),
        Command::Export {
            files,
            library,
            include_library,
            output,
        } => export(&files, &library, output.as_deref(), bare, include_library),
        Command::Fmt {
            files,
            write,
            check,
            width,
        } => fmt(&files, write, check, format, Layout { width }),
        Command::Check { paths, show } => check(&paths, show, format, bare),
        Command::Import {
            json,
            diagram,
            skin,
        } => {
            let painted = match skin.as_deref().map(read_skin) {
                Some(Ok(skin)) => skin,
                Some(Err(why)) => {
                    eprintln!("error: {why}");
                    return ExitCode::FAILURE;
                }
                None => sysml_diagram::Skin::default(),
            };
            import(&json, diagram.as_deref(), painted, format)
        }
        Command::Plan { paths } => plan(&paths, format, bare),
        Command::Diagram {
            paths,
            library,
            internal,
            browser,
            sequence,
            skin,
            output,
        } => {
            // the flags name one drawing between them; clap allows more
            // than one to be given and the innermost wins, as it did
            // when these were three arguments read in this order
            let view = match (internal.as_deref(), browser, sequence.as_deref()) {
                (_, true, _) => View::Browser,
                (_, _, Some(name)) => View::Sequence(name),
                (Some(name), _, _) => View::Internal(name),
                _ => View::Definitions,
            };
            let skin = match skin.as_deref().map(read_skin) {
                Some(Ok(skin)) => skin,
                Some(Err(why)) => {
                    eprintln!("error: {why}");
                    return ExitCode::FAILURE;
                }
                None => sysml_diagram::Skin::default(),
            };
            diagram(&paths, &library, view, output.as_deref(), bare, skin)
        }
        Command::ImportRust {
            json,
            package,
            output,
        } => import_rust(&json, package.as_deref(), output.as_deref()),
        Command::Rustgen {
            paths,
            library,
            output,
        } => rustgen(&paths, &library, output.as_deref(), bare),
        Command::Mcp { library, project } => {
            // an agent's launcher often has nowhere to put a flag, so the
            // environment says it too -- the language server reads the
            // same variable
            let library =
                library.or_else(|| std::env::var_os("SYSML_LIBRARY_PATH").map(Into::into));
            let project =
                project.or_else(|| std::env::var_os("SYSML_PROJECT_PATH").map(Into::into));
            // there is nowhere to report a failure to write: the only
            // way this ends badly is the client going away mid-answer,
            // and the exit code is what its launcher reads
            let mut server = match bare {
                true => sysml_cli::mcp::Server::without_library(project.as_deref()),
                false => {
                    sysml_cli::mcp::Server::with_project(library.as_deref(), project.as_deref())
                }
            };
            sysml_cli::mcp::serve(
                &mut server,
                std::io::BufReader::new(std::io::stdin()),
                std::io::stdout(),
            )
            .map_or(ExitCode::FAILURE, |()| ExitCode::SUCCESS)
        }
        Command::Api {
            what,
            server,
            timeout,
        } => api_command(
            &server,
            std::time::Duration::from_secs(timeout),
            &what,
            format,
            bare,
        ),
        Command::Corpus {
            dir,
            worst,
            failures,
        } => corpus(&dir, worst, failures),
    }
}

fn stats(files: &[PathBuf], format: Format) -> ExitCode {
    let mut counts: std::collections::BTreeMap<&'static str, usize> = Default::default();
    let mut total = 0usize;
    let mut errors = 0usize;
    for path in files {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => return Unreadable::refuse(path, err, "stats", format),
        };
        let parse = parse_file(path, &text);
        errors += parse.errors().len();
        let (model, _roots) = sysml_model::build_model(&parse);
        total += model.len();
        for id in model.ids() {
            *counts.entry(model.kind(id).name()).or_default() += 1;
        }
    }
    if format == Format::Json {
        report(serde_json::json!({
            "command": "stats",
            "ok": errors == 0,
            "unreadable": [],
            "elements": total,
            "parseErrors": errors,
            "counts": counts,
        }));
    } else {
        let mut rows: Vec<_> = counts.into_iter().collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        for (kind, n) in rows {
            println!("{n:6}  {kind}");
        }
        println!("{total:6}  total elements ({errors} parse error(s))");
    }
    // counting the elements of a file that did not parse is counting
    // what the parser guessed at, and `parse` and `check` already exit
    // non-zero on one; a script that runs all three reads them alike
    if errors == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn export(
    files: &[PathBuf],
    library: &[PathBuf],
    output: Option<&Path>,
    bare: bool,
    whole: bool,
) -> ExitCode {
    let Some((ws, extras, borrowed)) = resolved(files, library, bare, whole) else {
        return ExitCode::FAILURE;
    };
    // Written straight out, an element at a time. A model of the standard
    // library is 47 MB and the document of it is 754 MB of text; built
    // whole in between, as a `serde_json::Value`, it is about six
    // gigabytes. There is nothing to be done about how long the text is,
    // and nothing that wants the six gigabytes.
    match output {
        Some(path) => {
            // A serde error over a writer carries the writer's own, so
            // the one message covers a disk that filled halfway through
            // as well as one that was never writable.
            let wrote: serde_json::Result<usize> = std::fs::File::create(path)
                .map_err(serde_json::Error::io)
                .and_then(|file| {
                    let mut out = std::io::BufWriter::new(file);
                    let written = sysml_interchange::write_json(ws.model(), &extras, &mut out)?;
                    std::io::Write::flush(&mut out).map_err(serde_json::Error::io)?;
                    Ok(written)
                });
            let Ok(written) = wrote.inspect_err(|err| {
                eprintln!("error: cannot write {}: {err}", path.display());
            }) else {
                return ExitCode::FAILURE;
            };
            eprintln!("wrote {written} element(s) to {}", path.display());
            if let Some(note) = library_share(borrowed) {
                eprintln!("{note}");
            }
        }
        None => {
            let mut out = std::io::BufWriter::new(std::io::stdout().lock());
            sysml_interchange::write_json(ws.model(), &extras, &mut out)
                .expect("a model serializes");
            // the line `println!` used to put on the end of it
            let _ = std::io::Write::write_all(&mut out, b"\n");
        }
    }
    ExitCode::SUCCESS
}

/// The model of `files`, resolved against `library`, and what only the
/// resolver knows about it.
///
/// `sysml export` writes this out and `sysml api push` sends it, so both
/// mean the same thing by a model. The count is the part of it that is a
/// library's rather than the model's.
fn resolved(
    files: &[PathBuf],
    library: &[PathBuf],
    bare: bool,
    whole: bool,
) -> Option<(sysml_semantics::Workspace, sysml_interchange::Extras, usize)> {
    // resolve before serializing: the reified typings and specializations
    // are what the interchange derives inheritance and types from
    let mut ws = sysml_semantics::Workspace::new();
    load_paths(&mut ws, files).map_err(|e| e.say()).ok()?;
    let own = ws.file_count();
    load_paths(&mut ws, library).map_err(|e| e.say()).ok()?;
    ensure_library(&mut ws, bare).map_err(|e| e.say()).ok()?;
    // parse diagnostics still get printed while exporting
    let mut broken = false;
    for file in 0..own {
        let parse = ws.file_parse(file);
        let text = parse.syntax().text().to_string();
        for diagnostic in parse.errors() {
            print_diagnostic(Path::new(ws.file_name(file)), &text, diagnostic);
            broken = true;
        }
    }
    // A file the parser could not follow is missing whole declarations,
    // so what would be exported is not the model that was written. In a
    // file of its own that is merely wrong; `api push` sends the same
    // export to a server, where a declaration that failed to parse and
    // one that was deleted look exactly alike.
    if broken {
        return None;
    }
    ws.resolve_all();
    // the implied specializations resolution reasons with become part of
    // the model, the way the standard interchanges them
    ws.materialize_implied();

    // what only the resolver knows: imported memberships, import targets,
    // and which elements came in as library models
    let mut extras = sysml_interchange::Extras::default();
    let ids: Vec<_> = ws.model().ids().collect();
    for &id in &ids {
        let imported = ws.imported_members(id);
        if !imported.is_empty() {
            extras.imported.insert(id, imported);
        }
        if let Some(target) = ws.import_of(id) {
            extras.import_targets.insert(id, target);
        }
    }
    for file in own..ws.file_count() {
        for &root in ws.file_roots(file) {
            extras.library.insert(root);
            extras.library.extend(ws.model().descendants(root));
        }
    }
    // A model is resolved against a library and is not made of one. Six
    // elements exported with the standard library beside them wrote
    // ninety-six thousand, and `sysml api push` sent every one.
    //
    // What the model refers to across that line stays the `@id` it always
    // was. Those are UUIDv5 over the ownership path, so anybody holding the
    // same library computes the same ones -- how the standard refers to an
    // element another project holds. `--include-library` writes a document
    // that stands on its own.
    if !whole {
        extras.omitted = extras.library.clone();
    }

    let borrowed = match whole {
        true => extras.library.len(),
        false => 0,
    };
    Some((ws, extras, borrowed))
}

/// The model of `files` as interchange JSON, whole.
///
/// What `sysml api push` sends, which is a body rather than a file. The
/// counts are the document's and the part of it that is a library's.
fn exported(
    files: &[PathBuf],
    library: &[PathBuf],
    bare: bool,
    whole: bool,
) -> Option<(serde_json::Value, usize, usize)> {
    let (ws, extras, borrowed) = resolved(files, library, bare, whole)?;
    // what was written, rather than what was loaded: the library the
    // model resolved against is in the second and not the first
    let json = sysml_interchange::to_json_with(ws.model(), &extras);
    let written = json.as_array().map_or(0, Vec::len);
    Some((json, written, borrowed))
}

fn fmt(
    files: &[PathBuf],
    write: bool,
    check_only: bool,
    format: Format,
    layout: Layout,
) -> ExitCode {
    let mut dirty = 0usize;
    let mut unformatted = Vec::new();
    let mut broken = Vec::new();
    for path in files {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => return Unreadable::refuse(path, err, "fmt", format),
        };
        // Re-spacing a file the parser could not follow can move where a quote or
        // comment ends -- `package Name' {` runs the quote on to the next one --
        // and putting the tokens back differently puts the name somewhere else.
        // Printing that is harmless; writing it over the modeller's file is not,
        // so `--write` reports such a file instead.
        let parse = parse_file(path, &text);
        if write && !parse.ok() {
            eprintln!(
                "error: {} does not parse; `sysml parse` says where",
                path.display()
            );
            broken.push(path.display().to_string());
            continue;
        }
        // the tree is what the formatter reads, and this one is already in
        // hand: formatting from the text would parse the file again
        let formatted = sysml_syntax::fmt::format_parsed_with(&parse, layout);
        if check_only {
            if formatted != text {
                if format == Format::Text {
                    eprintln!("{}: not formatted", path.display());
                }
                unformatted.push(path.display().to_string());
                dirty += 1;
            }
        } else if write {
            if formatted != text {
                if let Err(err) = std::fs::write(path, &formatted) {
                    eprintln!("error: cannot write {}: {err}", path.display());
                    return ExitCode::FAILURE;
                }
                eprintln!("formatted {}", path.display());
            }
        } else {
            print!("{formatted}");
        }
    }
    // `--write` refuses a file it could not parse and exits non-zero
    // for it, so the JSON has to name it too -- otherwise a program is
    // told nothing is wrong and handed a failure
    if format == Format::Json && (check_only || write) {
        report(serde_json::json!({
            "command": "fmt",
            "ok": dirty == 0 && broken.is_empty(),
            "unreadable": [],
            "unformatted": unformatted,
            "broken": broken,
        }));
    }
    if dirty > 0 || !broken.is_empty() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// A path a command was given and could not take in. The command says
/// so in the form its `--format` calls for, which is why this is handed
/// back rather than printed here.
struct Unreadable {
    path: String,
    /// `read` for a file, `load` for a directory -- a directory is read
    /// as a whole, and the error names the file that stopped it
    verb: &'static str,
    error: String,
}

impl Unreadable {
    /// Refuse to go on because `path` could not be read, said the way
    /// `command` says things. Every command's JSON carries an
    /// `unreadable` list, so a program that watches for one watches for
    /// all of them.
    fn refuse(path: &Path, error: std::io::Error, command: &str, format: Format) -> ExitCode {
        let unreadable = Unreadable {
            path: path.display().to_string(),
            verb: "read",
            error: error.to_string(),
        };
        unreadable.say();
        if format == Format::Json {
            report(serde_json::json!({
                "command": command,
                "ok": false,
                "unreadable": [unreadable.json()],
            }));
        }
        ExitCode::FAILURE
    }

    fn say(&self) {
        let Unreadable { path, verb, error } = self;
        eprintln!("error: cannot {verb} {path}: {error}");
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({ "path": self.path, "error": self.error })
    }
}

/// Load every path -- file or directory -- into `ws`, stopping at the
/// first one that cannot be read.
fn load_paths(ws: &mut sysml_semantics::Workspace, paths: &[PathBuf]) -> Result<(), Unreadable> {
    for path in paths {
        let failed = |verb, error: std::io::Error| Unreadable {
            path: path.display().to_string(),
            verb,
            error: error.to_string(),
        };
        if path.is_dir() {
            ws.load_dir(path).map_err(|err| failed("load", err))?;
        } else {
            let text = std::fs::read_to_string(path).map_err(|err| failed("read", err))?;
            ws.add_file(path.to_string_lossy(), &text);
        }
    }
    Ok(())
}

/// Give `ws` a standard library, unless it already has one.
///
/// Almost nothing resolves without it -- `part def Vehicle;` specializes
/// `Parts::Part`, every feature subsets `Base::things` -- so a tool that
/// cannot find one reports every name in every model as unresolved.
///
/// What is already loaded wins: `sysml check model/ path/to/sysml.library`
/// has named it as a plain path since before there was anything to fall
/// back on, and a second copy over it would make every name answer twice.
/// That is why this runs after the paths are loaded. Then
/// `SYSML_LIBRARY_PATH`, which a launcher with nowhere to put a flag sets
/// and the other two front ends read. Then the copy built in.
fn ensure_library(
    ws: &mut sysml_semantics::Workspace,
    without: bool,
) -> Result<Answered, Unreadable> {
    if without {
        return Ok(Answered::None);
    }
    if ws.has_standard_library() {
        return Ok(Answered::Given);
    }
    if let Some(path) = std::env::var_os("SYSML_LIBRARY_PATH") {
        let named = PathBuf::from(path);
        load_paths(ws, std::slice::from_ref(&named))?;
        return Ok(Answered::At(named.display().to_string()));
    }
    for (name, text) in sysml_stdlib::FILES {
        ws.add_file(*name, text);
    }
    Ok(Answered::BuiltIn)
}

/// Which standard library answered.
///
/// A program reading the JSON cannot tell otherwise, and what it decides
/// depends on it: without one every reference into the library reads as
/// unresolved and the specification's constraints are not put at all.
enum Answered {
    /// One of the paths the caller named was it.
    Given,
    /// `SYSML_LIBRARY_PATH` said where.
    At(String),
    /// The copy built into this binary.
    BuiltIn,
    /// None: `--no-library`.
    None,
}

impl Answered {
    fn json(&self) -> serde_json::Value {
        match self {
            Answered::Given => serde_json::json!("given"),
            Answered::At(path) => serde_json::json!(path),
            Answered::BuiltIn => serde_json::json!(format!("built in ({})", sysml_stdlib::RELEASE)),
            Answered::None => serde_json::Value::Null,
        }
    }
}

/// What the model implies for code, in no language in particular.
///
/// The library is loaded and resolved behind the model -- what a feature
/// bottoms out in is a question about it -- but only the model's own
/// definitions are planned.
fn plan(paths: &[PathBuf], format: Format, bare: bool) -> ExitCode {
    let mut ws = sysml_semantics::Workspace::new();
    if let Err(unreadable) = load_paths(&mut ws, paths) {
        unreadable.say();
        return ExitCode::FAILURE;
    }
    let own: Vec<usize> = (0..ws.file_count()).collect();
    if let Err(unreadable) = ensure_library(&mut ws, bare) {
        unreadable.say();
        return ExitCode::FAILURE;
    }
    ws.resolve_all();
    let roots: Vec<sysml_model::ElementId> = own
        .iter()
        .flat_map(|&file| ws.file_roots(file).to_vec())
        .collect();
    let planned = sysml_cli::plan::of(&mut ws, &roots);
    match format {
        Format::Json => report(serde_json::to_value(&planned).expect("a built plan serializes")),
        Format::Text => {
            for definition in &planned.definitions {
                println!(
                    "{} -- {} ({} feature(s))",
                    definition.of,
                    definition.shape,
                    definition.features.len()
                );
            }
            // Silence reads as "there is nothing here to write", which
            // is a wrong answer where the truth is that the files hold
            // no definition at all -- a package of imports, a model
            // whose declarations are all in the file next to it.
            if planned.definitions.is_empty() {
                println!("no definitions: nothing here implies any code");
            }
        }
    }
    // A plan is only as good as the model behind it: a feature whose
    // type resolved to nothing is planned with no type at all. The plan
    // is still handed over -- somebody writing a model is entitled to
    // see what it implies so far -- and the exit code says not to build
    // from it yet.
    if !planned.checked.ok {
        if format == Format::Text {
            for broken in &planned.checked.syntax {
                eprintln!("does not parse: {broken}");
            }
            if !planned.checked.unresolved.is_empty() {
                eprintln!(
                    "warning: {} name(s) in this model resolve to nothing, so what they \
                     type is missing from the plan; `sysml check` says where they are",
                    planned.checked.unresolved.len()
                );
            }
        }
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// How much of an export is a library's rather than the model's.
///
/// Only `--include-library` puts a library in the document, so this is
/// what somebody who asked for one gets told they got: the size of the
/// request is the other way to find out.
fn library_share(borrowed: usize) -> Option<String> {
    (borrowed > 0).then(|| format!("note: {borrowed} of them are the library's"))
}

/// The element a `--internal`/`--sequence` argument names, or a message
/// saying there is none. `own` is what the modeller asked to draw, as
/// against a library loaded behind it.
fn named(
    ws: &sysml_semantics::Workspace,
    own: &std::collections::HashSet<sysml_model::ElementId>,
    name: &str,
) -> Option<sysml_model::ElementId> {
    let mut found: Vec<_> = ws
        .named_elements()
        .filter(|(_, declared)| *declared == name)
        .map(|(id, _)| id)
        .collect();
    // a qualified name where a declared one finds nothing: `Vehicles::Car`
    // is how a modeller says which `Car` when the plain name will not
    if found.is_empty() {
        found = ws
            .named_elements()
            .map(|(id, _)| id)
            .filter(|id| ws.qualified_name_of(*id) == name)
            .collect();
    }
    // the modeller's own files come first: a name they declared is the
    // one they meant, even where the library declares it too
    if found.iter().any(|id| own.contains(id)) {
        found.retain(|id| own.contains(id));
    }
    match found.as_slice() {
        [] => {
            eprintln!("error: no element named `{name}`");
            None
        }
        [one] => Some(*one),
        // drawing one of several without a word would answer about a
        // model the modeller did not mean to ask about
        [first, rest @ ..] => {
            eprintln!(
                "warning: `{name}` names {} elements; drawing {}; a qualified name says which",
                rest.len() + 1,
                ws.qualified_name_of(*first)
            );
            Some(*first)
        }
    }
}

/// Which drawing was asked for.
///
/// `--internal`, `--browser` and `--sequence` name one drawing between
/// them, and travelled as three arguments that could all be set at once
/// and meant nothing together.
enum View<'a> {
    /// The definitions and the relationships that run between them,
    /// which is what `diagram` draws when it is asked for nothing else.
    Definitions,
    /// The internal structure of one definition: the parts it is
    /// assembled from and the connections between them.
    Internal(&'a str),
    /// The membership hierarchy as an indented tree.
    Browser,
    /// The interaction one definition declares, as lifelines and the
    /// messages between them.
    Sequence(&'a str),
}

/// The skin `asked` names: one that ships, or a JSON file saying what
/// to paint the drawing in.
///
/// A name that is not one of the skins that ship is more likely a path
/// that is not there than a skin nobody wrote, so the two are told
/// apart by whether anything is at that path.
fn read_skin(asked: &str) -> Result<sysml_diagram::Skin, String> {
    if let Some(skin) = sysml_diagram::Skin::named(asked) {
        return Ok(skin);
    }
    let path = Path::new(asked);
    if !path.exists() {
        return Err(format!(
            "no skin `{asked}`, and no file at that path; the skins that ship are {}",
            sysml_diagram::Skin::NAMES.join(", ")
        ));
    }
    let text =
        std::fs::read_to_string(path).map_err(|why| format!("cannot read {asked}: {why}"))?;
    let said: serde_json::Value =
        serde_json::from_str(&text).map_err(|why| format!("{asked} is not JSON: {why}"))?;
    sysml_diagram::skin::read(&said).map_err(|why| format!("{asked}: {why}"))
}

fn diagram(
    paths: &[PathBuf],
    library: &[PathBuf],
    view: View<'_>,
    output: Option<&Path>,
    bare: bool,
    skin: sysml_diagram::Skin,
) -> ExitCode {
    let mut ws = sysml_semantics::Workspace::new();
    if let Err(unreadable) = load_paths(&mut ws, paths) {
        unreadable.say();
        return ExitCode::FAILURE;
    }
    // everything loaded so far is drawn; the library that follows only has
    // to be resolvable, so its definitions never become boxes
    let drawn = ws.file_count();
    if let Err(unreadable) =
        load_paths(&mut ws, library).and_then(|()| ensure_library(&mut ws, bare))
    {
        unreadable.say();
        return ExitCode::FAILURE;
    }
    // only what is drawn, and what it reaches: a library is loaded so
    // that names resolve, not so that all of it is worked through
    let files: Vec<usize> = (0..drawn).collect();
    ws.resolve_reached(&files);
    let roots: Vec<_> = (0..drawn)
        .flat_map(|file| ws.file_roots(file).to_vec())
        .collect();
    // what was asked for, as against what was loaded to resolve against
    let own: std::collections::HashSet<_> = roots
        .iter()
        .flat_map(|&root| std::iter::once(root).chain(ws.model().descendants(root)))
        .collect();
    if let View::Browser = view {
        let view = sysml_diagram::browser_view(ws.model(), &roots);
        if view.rows.is_empty() {
            eprintln!("error: nothing to draw");
            return ExitCode::FAILURE;
        }
        let svg = sysml_diagram::render_browser(
            &view,
            &sysml_diagram::Style {
                skin: skin.clone(),
                ..Default::default()
            },
        );
        return emit(&svg, output, &format!("{} row(s)", view.rows.len()));
    }
    if let View::Sequence(name) = view {
        let Some(target) = named(&ws, &own, name) else {
            return ExitCode::FAILURE;
        };
        let view = sysml_diagram::sequence_view(ws.model(), target);
        if view.lifelines.is_empty() {
            eprintln!("error: `{name}` declares no interaction to draw");
            return ExitCode::FAILURE;
        }
        let svg = sysml_diagram::render_sequence(
            &view,
            &sysml_diagram::Style {
                skin: skin.clone(),
                ..Default::default()
            },
        );
        return emit(
            &svg,
            output,
            &format!(
                "{} lifeline(s) and {} message(s)",
                view.lifelines.len(),
                view.moments.len()
            ),
        );
    }
    let diagram = match view {
        View::Internal(name) => {
            let Some(target) = named(&ws, &own, name) else {
                return ExitCode::FAILURE;
            };
            sysml_diagram::interconnection_diagram(ws.model(), target)
        }
        _ => sysml_diagram::definition_diagram(ws.model(), &roots),
    };
    if diagram.nodes.is_empty() {
        eprintln!("error: nothing to draw");
        return ExitCode::FAILURE;
    }
    let style = sysml_diagram::Style {
        skin,
        ..Default::default()
    };
    let svg = sysml_diagram::render(&diagram, &style);
    let count = |relation| {
        diagram
            .edges
            .iter()
            .filter(|edge| edge.relation == relation)
            .count()
    };
    use sysml_diagram::Relation;
    let summary = format!(
        "{} box(es), {} specialization(s), {} composition(s), {} reference(s), \
         {} subsetting(s), {} connection(s), {} flow(s), {} allocation(s), \
         {} transition(s), {} dependency(ies) and {} satisfaction(s)",
        diagram.nodes.len(),
        count(Relation::Specialization),
        // a portion is a composite membership the standard draws with a
        // marker of its own
        count(Relation::Composition) + count(Relation::Portion),
        count(Relation::Reference),
        count(Relation::Subsetting) + count(Relation::Redefinition),
        // an interface and a binding are connections too -- what each is,
        // the drawing says on the line
        count(Relation::Connection) + count(Relation::Interface) + count(Relation::Binding),
        count(Relation::Flow) + count(Relation::SuccessionFlow) + count(Relation::Message),
        count(Relation::Allocation),
        count(Relation::Transition) + count(Relation::Succession),
        // the keyworded lines: what a definition answers for, and what
        // one element depends on another for
        count(Relation::Dependency)
            + count(Relation::Client)
            + count(Relation::Assert)
            + count(Relation::Assume)
            + count(Relation::Require)
            + count(Relation::Perform)
            + count(Relation::Exhibit)
            + count(Relation::Event)
            + count(Relation::Annotation),
        count(Relation::Satisfy),
    );
    emit(&svg, output, &summary)
}

/// Write a rendered view to `output`, or to stdout when there is none.
fn rustgen(paths: &[PathBuf], library: &[PathBuf], output: Option<&Path>, bare: bool) -> ExitCode {
    let mut ws = sysml_semantics::Workspace::new();
    if let Err(unreadable) = load_paths(&mut ws, paths) {
        unreadable.say();
        return ExitCode::FAILURE;
    }
    // generation covers what was named; the library only resolves
    let own = ws.file_count();
    if let Err(unreadable) =
        load_paths(&mut ws, library).and_then(|()| ensure_library(&mut ws, bare))
    {
        unreadable.say();
        return ExitCode::FAILURE;
    }
    // what is generated is what was named, and what that reaches; the
    // rest of a library is loaded so names resolve, not to be worked
    // through
    let files: Vec<usize> = (0..own).collect();
    let stats = ws.resolve_reached(&files);
    if stats.unresolved > 0 {
        // generated code would silently miss whatever did not resolve
        for missing in ws.unresolved() {
            eprintln!(
                "error: unresolved `{}` in {}",
                missing.name,
                ws.file_name(missing.file)
            );
        }
        return ExitCode::FAILURE;
    }
    let roots: Vec<_> = (0..own)
        .flat_map(|file| ws.file_roots(file).to_vec())
        .collect();
    match sysml_rust::generate(ws.model(), &roots) {
        Ok(generated) => {
            let rust = generated.rust;
            // counted by the lines that declare them: a `doc` in the
            // model becomes a `///` line, and counting substrings read
            // whatever it happened to say as another declaration
            let structs = rust
                .lines()
                .filter(|l| l.starts_with("pub struct "))
                .count();
            let methods = rust
                .lines()
                .filter(|l| l.starts_with("    pub fn ") || l.starts_with("    pub async fn "))
                .count();
            // What the model left open is the part a person still owes,
            // and a count of it belongs with the count of what was
            // written rather than only in comments inside the file.
            let open = generated.open.len();
            emit(
                &rust,
                output,
                &format!("{structs} struct(s), {methods} method(s) and {open} thing(s) left open"),
            )
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Read a standard interchange document back into a model.
///
/// `export` writes one and `sysml api` fetches one, and until now
/// nothing here could read either back: `sysml_interchange::from_json`
/// was written, round-trip tested over the whole standard library, and
/// reachable from no front end at all. A document that came off a model
/// server had nowhere to go.
///
/// What it can then do with the model is bounded by what a model on its
/// own is enough for. Counting it is; drawing it is, since a diagram
/// asks for elements and the relationships between them and nothing
/// else. Checking the specification's constraints is not, and neither is
/// writing the notation back out -- both want a workspace, which is
/// files, and this document is not files. Rather than half-answer those,
/// this does the two it can and says so.
fn import(
    path: &Path,
    diagram: Option<&Path>,
    skin: sysml_diagram::Skin,
    format: Format,
) -> ExitCode {
    // The bytes, and never the whole document as `serde_json::Value`: the
    // library's is 754 MB of text and about six gigabytes built, for a
    // model that is 47 MB. `read_json` parses one element at a time.
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => return Unreadable::refuse(path, err, "import", format),
    };
    let (model, roots, objects) = match sysml_interchange::read_json(&bytes) {
        Ok(read) => read,
        // A document `export` wrote without `--include-library` names the
        // library elements its model specializes and does not carry them:
        // every definition implicitly specializes one, so this is what a
        // reader meets first and what it would otherwise meet as an
        // opaque UUID. The same shape is what a model server returns for
        // a page of elements, whose neighbours are simply not in the page.
        Err(sysml_interchange::ImportError::UnknownReference(id)) => {
            return refuse_import(
                path,
                &format!(
                    "it names element {id} and does not carry it, so this is \
                     part of a model rather than one whole. \
                     `sysml export --include-library` writes a document that \
                     stands on its own"
                ),
                format,
            )
        }
        Err(why) => return refuse_import(path, &why.to_string(), format),
    };
    if let Some(output) = diagram {
        let drawing = sysml_diagram::definition_diagram(&model, &roots);
        if drawing.nodes.is_empty() {
            eprintln!("error: nothing to draw");
            return ExitCode::FAILURE;
        }
        let style = sysml_diagram::Style {
            skin,
            ..Default::default()
        };
        let svg = sysml_diagram::render(&drawing, &style);
        return emit(
            &svg,
            Some(output),
            &format!(
                "{} box(es) and {} line(s)",
                drawing.nodes.len(),
                drawing.edges.len()
            ),
        );
    }
    let mut counts: std::collections::BTreeMap<&'static str, usize> = Default::default();
    for id in model.ids() {
        *counts.entry(model.kind(id).name()).or_default() += 1;
    }
    if format == Format::Json {
        report(serde_json::json!({
            "command": "import",
            "ok": true,
            "read": path.display().to_string(),
            "objects": objects,
            "elements": model.len(),
            "roots": roots.len(),
            "counts": counts,
        }));
    } else {
        let mut rows: Vec<_> = counts.into_iter().collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        for (kind, n) in rows {
            println!("{n:6}  {kind}");
        }
        // The document holds more objects than the model holds elements:
        // a membership that only carries ownership is an object there and
        // an edge here, which is what `export` counted on the way out.
        println!(
            "{:6}  total elements, {} owned by nothing, from {objects} object(s)",
            model.len(),
            roots.len()
        );
    }
    ExitCode::SUCCESS
}

/// A document that will not read, said the way every other refusal here
/// is said: the path, one sentence, and the same shape under `--format
/// json` that `Unreadable` writes for a file that will not open.
fn refuse_import(path: &Path, why: &str, format: Format) -> ExitCode {
    match format {
        Format::Json => report(serde_json::json!({
            "command": "import",
            "ok": false,
            "unreadable": [{ "path": path.display().to_string(), "error": why }],
        })),
        Format::Text => eprintln!("error: {}: {why}", path.display()),
    }
    ExitCode::FAILURE
}

fn import_rust(json: &Path, package: Option<&str>, output: Option<&Path>) -> ExitCode {
    let text = match std::fs::read_to_string(json) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("error: cannot read {}: {err}", json.display());
            return ExitCode::FAILURE;
        }
    };
    match sysml_rust::rustdoc_to_sysml(&text, package) {
        Ok(imported) => {
            let sysml = imported.sysml;
            // counted off the tree, not the text: a `doc` that happens to
            // say "def" between two spaces is not a definition, and this
            // number is what a person checks the import by
            let written = sysml_syntax::parse(&sysml);
            let definitions = written
                .syntax()
                .descendants()
                .filter(|node| node.kind() == sysml_syntax::SyntaxKind::DEFINITION)
                .count();
            // and what had no shape is what is left to model by hand
            let skipped = imported.skipped.len();
            emit(
                &sysml,
                output,
                &format!("{definitions} definition(s), {skipped} item(s) not imported"),
            )
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn emit(svg: &str, output: Option<&Path>, summary: &str) -> ExitCode {
    let Some(path) = output else {
        print!("{svg}");
        return ExitCode::SUCCESS;
    };
    if let Err(err) = std::fs::write(path, svg) {
        eprintln!("error: cannot write {}: {err}", path.display());
        return ExitCode::FAILURE;
    }
    eprintln!("wrote {summary} to {}", path.display());
    ExitCode::SUCCESS
}

fn check(paths: &[PathBuf], show: usize, format: Format, bare: bool) -> ExitCode {
    let mut ws = sysml_semantics::Workspace::new();
    // `check` has always taken the library as one of its paths --
    // `sysml check model/ sysml.library` -- so what it was handed is
    // loaded first and `ensure_library` only fills a gap.
    let loaded = load_paths(&mut ws, paths);
    // What was asked about is what was handed over; the library that
    // follows is what it is resolved against. The constraints are put to
    // the former alone: the library satisfies them, a test says so every
    // run, and putting a hundred and thirty of them to its sixty-six
    // thousand elements again was most of what `check` cost.
    let own: Vec<usize> = (0..ws.file_count()).collect();
    let library = match loaded.and_then(|()| ensure_library(&mut ws, bare)) {
        Ok(answered) => answered,
        Err(unreadable) => {
            unreadable.say();
            if format == Format::Json {
                report(serde_json::json!({
                    "command": "check",
                    "ok": false,
                    "unreadable": [unreadable.json()],
                }));
            }
            return ExitCode::FAILURE;
        }
    };
    // A file that does not parse has no names to resolve, so resolution
    // finds nothing wrong with it and this used to answer `ok`. Anyone
    // running only `check` -- which is most of the reason it exists --
    // would be told a broken model was fine. Syntax comes first, as it
    // does in the MCP server's tool of the same name.
    let syntax = ws.findings(&own).syntax;
    let texts = held_texts(&ws, &syntax);
    let mut broken = Vec::new();
    for finding in &syntax {
        let name = ws.file_name(finding.file);
        let text = &texts[&finding.file];
        if format == Format::Text {
            say(&Said {
                severity: Severity::Error,
                title: finding.what.clone(),
                id: None,
                path: name,
                text,
                span: span(finding.range),
                label: None,
                helps: Vec::new(),
            });
        }
        broken.push(sysml_cli::at(
            text,
            finding.range,
            serde_json::json!({ "path": name, "message": finding.what }),
        ));
    }
    if !broken.is_empty() {
        if format == Format::Json {
            report(serde_json::json!({
                "command": "check",
                "ok": false,
                "unreadable": [],
                "elements": ws.model().len(),
                "parseErrors": broken,
            }));
        }
        return ExitCode::FAILURE;
    }

    let stats = ws.resolve_all();
    let total = stats.resolved + stats.unresolved;
    let rate = if total == 0 {
        100.0
    } else {
        100.0 * stats.resolved as f64 / total as f64
    };
    // `--show` trims a list a person reads; a program is given all of
    // them, since it is the list it works from
    let limit = if show == 0 || format == Format::Json {
        usize::MAX
    } else {
        show
    };
    // the names are asked for after resolving, the syntax before it:
    // there is nothing to resolve in a file that did not parse
    let diagnosed = ws.diagnose(&own);
    let found = &diagnosed.found;
    let names = &found.names;
    let shown = &names[..limit.min(names.len())];
    let texts = held_texts(&ws, shown);
    // What each of them might have meant: a right name nothing brought into
    // scope, or a wrong one with a right name beside it -- two mistakes that
    // resolution reports identically. Asked for all at once, because the walk
    // over every declared name is what costs.
    let missed: Vec<String> = shown.iter().map(|u| u.what.clone()).collect();
    let might = ws.suggestions(&missed);
    let mut unresolved = Vec::new();
    for (u, meant) in shown.iter().zip(&might) {
        let file = ws.file_name(u.file);
        let text = &texts[&u.file];
        match format {
            Format::Text => say(&Said {
                severity: Severity::Error,
                title: format!("`{}` resolves to nothing", u.what),
                id: None,
                path: file,
                text,
                span: span(u.range),
                label: None,
                helps: helps(meant),
            }),
            Format::Json => {
                let mut entry = sysml_cli::at(
                    text,
                    u.range,
                    serde_json::json!({ "path": file, "name": u.what.clone() }),
                );
                if !meant.elsewhere.is_empty() {
                    entry["declaredAs"] = serde_json::json!(meant.elsewhere);
                }
                if !meant.near.is_empty() {
                    entry["didYouMean"] = serde_json::json!(meant.near);
                }
                unresolved.push(entry);
            }
        }
    }
    // What the specification requires, over and above every name resolving: a
    // model that resolves can still be one the standard rejects. Whether it
    // was worth asking -- constraints come after names, as names come after
    // syntax, and they are written against the library besides -- is
    // `diagnose`'s to decide, so the three front ends cannot drift apart.
    //
    // `unevaluated` counts the constraints this toolchain refuses to run,
    // which says something about the toolchain and nothing about the model,
    // so the text report leaves it out and the JSON carries it.
    let checked = &diagnosed.rules;
    // A zero read out of a model nothing was asked of is a clean bill
    // the check never gave, so the text report leaves the sentence out
    // rather than printing one.
    let askable = diagnosed.asked;
    // Every element a constraint is asked of is under one of the files
    // that were loaded, so it has somewhere to be shown. The one element
    // of a workspace that is under no file is its root, which no
    // constraint is about.
    let placed: Vec<(
        (usize, sysml_syntax::TextRange),
        &sysml_semantics::rules::Violation,
    )> = checked
        .violations
        .iter()
        .map(|violation| {
            (
                ws.element_place(violation.element).unwrap_or_default(),
                violation,
            )
        })
        .collect();
    let violated = texts_of(&ws, placed.iter().map(|((file, _), _)| *file));
    let mut violations = Vec::new();
    for ((file, range), violation) in &placed {
        let named = ws.qualified_name_of(violation.element);
        let path = ws.file_name(*file);
        let text = &violated[file];
        match format {
            Format::Text => say(&Said {
                severity: Severity::Error,
                title: violation.says.to_string(),
                // the specification's own name for the constraint,
                // in the slot rustc puts an error code: it is what
                // somebody looking the rule up will search for
                id: Some(violation.rule),
                path,
                text,
                span: span(*range),
                label: Some(format!("`{named}`")),
                helps: Vec::new(),
            }),
            Format::Json => violations.push(sysml_cli::at(
                text,
                *range,
                serde_json::json!({
                    "path": path,
                    "rule": violation.rule,
                    "says": violation.says,
                    "element": named,
                }),
            )),
        }
    }

    // A root package of one's own named after one of the standard
    // library's resolves -- each side reads its own -- so it is said
    // alongside the names rather than counted among them.
    let mut collisions = Vec::new();
    let clashing = held_texts(&ws, &found.collisions);
    for c in &found.collisions {
        let file = ws.file_name(c.file);
        let text = &clashing[&c.file];
        match format {
            Format::Text => say(&Said {
                severity: Severity::Warning,
                title: c.what.clone(),
                id: None,
                path: file,
                text,
                span: span(c.range),
                label: None,
                helps: Vec::new(),
            }),
            Format::Json => collisions.push(sysml_cli::at(
                text,
                c.range,
                serde_json::json!({ "path": file, "message": c.what.clone() }),
            )),
        }
    }
    let sound = stats.unresolved == 0 && checked.violations.is_empty();
    if format == Format::Json {
        report(serde_json::json!({
            "command": "check",
            "ok": sound,
            "library": library.json(),
            "unreadable": [],
            "elements": ws.model().len(),
            "parseErrors": [],
            "resolved": stats.resolved,
            "references": total,
            "unresolved": unresolved,
            "collisions": collisions,
            "violations": violations,
            "rules": {
                "held": checked.held.len(),
                "unevaluated": checked.unevaluated.len(),
            },
        }));
        return if sound {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    if names.len() > limit {
        eprintln!("... and {} more", names.len() - limit);
    }
    let resolution = format!(
        "{} element(s), {}/{total} reference(s) resolved ({rate:.1}%)",
        ws.model().len(),
        stats.resolved
    );
    match askable {
        true => println!(
            "{resolution}, {} constraint(s) checked, {} violation(s)",
            checked.held.len(),
            checked.violations.len()
        ),
        false => println!("{resolution}"),
    }
    if sound {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// The text of each file these findings are in, so they can be placed
/// after `ws` has been borrowed again to resolve. From the workspace
/// rather than a second read: `file_name` is a lossy rendering of the
/// path, so a path that is not UTF-8 would find nothing and place every
/// finding at 1:1.
fn held_texts(
    ws: &sysml_semantics::Workspace,
    findings: &[sysml_semantics::Finding],
) -> std::collections::HashMap<usize, String> {
    texts_of(ws, findings.iter().map(|f| f.file))
}

/// The same, of files named directly rather than through findings.
fn texts_of(
    ws: &sysml_semantics::Workspace,
    files: impl Iterator<Item = usize>,
) -> std::collections::HashMap<usize, String> {
    files
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|file| (file, ws.file_parse(file).syntax().text().to_string()))
        .collect()
}

fn corpus(dir: &Path, worst: usize, list_failures: bool) -> ExitCode {
    let files = sysml_semantics::model_files(dir);
    if files.is_empty() {
        // the walk takes what it can reach, so a directory that is not
        // there and one with nothing under it look the same from here
        match std::fs::read_dir(dir) {
            Ok(_) => eprintln!("no .sysml/.kerml files found under {}", dir.display()),
            Err(err) => eprintln!("error: cannot read {}: {err}", dir.display()),
        }
        return ExitCode::FAILURE;
    }

    struct Stat {
        total: usize,
        ok: usize,
        errors: usize,
    }
    let mut by_ext: std::collections::BTreeMap<String, Stat> = Default::default();
    let mut failing: Vec<(usize, PathBuf)> = Vec::new();

    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            eprintln!("warning: cannot read {}", path.display());
            continue;
        };
        let parse = parse_file(path, &text);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("?")
            .to_string();
        let stat = by_ext.entry(ext).or_insert(Stat {
            total: 0,
            ok: 0,
            errors: 0,
        });
        stat.total += 1;
        stat.errors += parse.errors().len();
        if parse.ok() {
            stat.ok += 1;
        } else {
            failing.push((parse.errors().len(), path.clone()));
        }
    }

    for (ext, stat) in &by_ext {
        println!(
            ".{ext}: {}/{} files ok ({:.1}%), {} total error(s)",
            stat.ok,
            stat.total,
            100.0 * stat.ok as f64 / stat.total as f64,
            stat.errors
        );
    }

    failing.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    if list_failures {
        for (errors, path) in &failing {
            println!("FAIL {errors:5} {}", path.display());
        }
    } else if worst > 0 && !failing.is_empty() {
        println!("\nworst {} file(s):", worst.min(failing.len()));
        for (errors, path) in failing.iter().take(worst) {
            println!("  {errors:5} error(s)  {}", path.display());
        }
    }
    ExitCode::SUCCESS
}

fn parse_files(files: &[PathBuf], dump_tree: bool, format: Format) -> ExitCode {
    let mut total_errors = 0usize;
    let mut reported = Vec::new();
    let mut unreadable = Vec::new();
    for path in files {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => {
                total_errors += 1;
                let failed = Unreadable {
                    path: path.display().to_string(),
                    verb: "read",
                    error: err.to_string(),
                };
                failed.say();
                unreadable.push(failed.json());
                continue;
            }
        };
        let parse = parse_file(path, &text);
        if dump_tree {
            // stdout is one JSON document when a program is reading, and
            // a tree in front of it would spoil that; a person reading
            // the tree can have it either way round
            match format {
                Format::Text => println!("{:#?}", parse.syntax()),
                Format::Json => eprintln!("{:#?}", parse.syntax()),
            }
        }
        total_errors += parse.errors().len();
        match format {
            Format::Text => {
                for diagnostic in parse.errors() {
                    print_diagnostic(path, &text, diagnostic);
                }
                let status = if parse.ok() { "ok" } else { "FAILED" };
                eprintln!(
                    "{}: {status} ({} error(s))",
                    path.display(),
                    parse.errors().len()
                );
            }
            Format::Json => reported.push(serde_json::json!({
                "path": path.display().to_string(),
                "ok": parse.ok(),
                "errors": parse
                    .errors()
                    .iter()
                    .map(|d| sysml_cli::at(&text, d.range, serde_json::json!({
                        "message": d.message.clone(),
                    })))
                    .collect::<Vec<_>>(),
            })),
        }
    }
    if format == Format::Json {
        report(serde_json::json!({
            "command": "parse",
            "ok": total_errors == 0,
            "unreadable": unreadable,
            "errors": total_errors,
            "files": reported,
        }));
    }
    if total_errors == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// One JSON document per run, on stdout.
fn report(value: serde_json::Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(&value).expect("a built value serializes")
    );
}

fn print_diagnostic(path: &Path, text: &str, diagnostic: &Diagnostic) {
    say(&Said {
        severity: Severity::Error,
        title: diagnostic.message.clone(),
        id: None,
        path: &path.display().to_string(),
        text,
        span: span(diagnostic.range),
        label: None,
        helps: Vec::new(),
    });
}

/// A finding, to whoever is watching the terminal.
///
/// Findings go to stderr so that `--format json` on stdout stays one
/// document, and so that a person can watch them go by while a program
/// reads the answer.
fn say(said: &Said<'_>) {
    eprintln!("{}", report::draw(said, *COLOUR.get().unwrap_or(&false)));
}

/// The bytes a finding is about.
fn span(range: sysml_syntax::TextRange) -> std::ops::Range<usize> {
    usize::from(range.start())..usize::from(range.end())
}

/// What is known about a name that resolved to nothing.
///
/// Two different mistakes, and only one is this name's. A name the
/// workspace declares somewhere wants an import -- so that is all that is
/// said, even though names are near it too: `MassValue` is answered by
/// `ISQBase::MassValue`, and every other name with `value` in it
/// underneath makes the answer worse. A name nothing declares is a wrong
/// one, and then the near ones are the whole answer.
///
/// A person acts on two or three and reads past the rest, so the sentence
/// stops there; the JSON carries what was found.
fn helps(meant: &sysml_semantics::Suggestion) -> Vec<String> {
    const ENOUGH: usize = 3;
    if !meant.elsewhere.is_empty() {
        return vec![format!(
            "it is declared as {}, which nothing here brings into scope -- an import would",
            listed(&meant.elsewhere, ENOUGH)
        )];
    }
    match meant.near.is_empty() {
        true => Vec::new(),
        false => vec![format!("did you mean {}?", listed(&meant.near, ENOUGH))],
    }
}

/// Names, quoted, as a sentence says them, and no more of them than a
/// reader will read.
fn listed(names: &[String], most: usize) -> String {
    let extra = names.len().saturating_sub(most);
    let quoted: Vec<String> = names
        .iter()
        .take(most)
        .map(|name| format!("`{name}`"))
        .chain((extra > 0).then(|| format!("{extra} other(s)")))
        .collect();
    let mut sentence = quoted.join(", ");
    // the last comma is an `or`, which is how a sentence says a list
    if let Some(last) = sentence.rfind(", ") {
        sentence.replace_range(last..last + 2, " or ");
    }
    sentence
}

/// Talk to a model server. Every answer is the server's own JSON, so
/// what comes back is what the standard says came back; the text form
/// lists the one line a person reads it for.
fn api_command(
    server: &str,
    timeout: std::time::Duration,
    what: &ApiCommand,
    format: Format,
    bare: bool,
) -> ExitCode {
    let client = api::Client::new(server, timeout);
    let answered = match what {
        ApiCommand::Projects => client
            .projects()
            .map(|ps| (ps.iter().map(project_line).collect(), json_of(&ps))),
        ApiCommand::Project { project } => client
            .project(project)
            .map(|p| (vec![project_line(&p)], json_of(&p))),
        ApiCommand::NewProject { name } => client
            .create_project(name)
            .map(|p| (vec![project_line(&p)], json_of(&p))),
        ApiCommand::Commits { project } => client
            .commits(project)
            .map(|cs| (cs.iter().map(commit_line).collect(), json_of(&cs))),
        ApiCommand::Elements { project, commit } => client.elements(project, commit).map(|es| {
            let lines = es.iter().map(element_line).collect();
            (lines, serde_json::Value::Array(es))
        }),
        ApiCommand::Element {
            project,
            commit,
            element,
        } => client
            .element(project, commit, element)
            .map(|e| (vec![element_line(&e)], e)),
        ApiCommand::Push {
            paths,
            project,
            message,
            library,
            include_library,
        } => {
            let Some((json, elements, borrowed)) = exported(paths, library, bare, *include_library)
            else {
                return ExitCode::FAILURE;
            };
            let changes: Vec<serde_json::Value> = json.as_array().cloned().unwrap_or_default();
            client.create_commit(project, message, &changes).map(|c| {
                (
                    // the count of what was sent on its own line, so
                    // the sentence a script reads is the one it always
                    // read and the warning is beside it rather than
                    // inside it
                    [format!("{} element(s) as commit {}", elements, c.id)]
                        .into_iter()
                        .chain(library_share(borrowed))
                        .collect(),
                    serde_json::to_value(&c).expect("serializable"),
                )
            })
        }
    };
    match answered {
        Ok((lines, json)) => {
            match format {
                Format::Text => {
                    for line in lines {
                        println!("{line}");
                    }
                }
                Format::Json => report(json),
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: {}: {err}", api::redacted(server));
            ExitCode::FAILURE
        }
    }
}

fn json_of<T: serde::Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).expect("serializable")
}

fn project_line(project: &api::Project) -> String {
    format!("{} {}", project.id, project.name.as_deref().unwrap_or(""))
}

fn commit_line(commit: &api::Commit) -> String {
    format!(
        "{} {}",
        commit.id,
        commit.description.as_deref().unwrap_or("")
    )
}

fn element_line(element: &serde_json::Value) -> String {
    let at = |key: &str| element[key].as_str().unwrap_or("").to_string();
    format!("{} {} {}", at("@id"), at("@type"), at("declaredName"))
}

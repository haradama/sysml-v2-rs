use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
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

#[derive(Parser)]
#[command(name = "sysml", version, about = "SysML v2 command-line tools")]
struct Cli {
    /// How findings are reported: for a person, or as JSON for a program
    #[arg(long, value_enum, default_value_t = Format::Text, global = true)]
    format: Format,
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
        /// Resolve names against these files or directories too and
        /// export them along, marked `isLibraryElement`
        #[arg(long)]
        library: Vec<PathBuf>,
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
    },
    /// Load files (or directories) into one workspace, resolve all names and
    /// report unresolved references
    Check {
        /// Files or directories to load
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Show each unresolved reference (up to N; 0 = all)
        #[arg(long, default_value_t = 20)]
        show: usize,
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
        /// Let the Eclipse Layout Kernel arrange and route it (the
        /// drawing itself stays the same); needs `cargo install elkrs`
        #[arg(long)]
        elk: bool,
        /// The ELK command to run with --elk
        #[arg(long, default_value = "elkrs", value_name = "COMMAND")]
        elk_command: String,
        /// Write to this file instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Import a Rust crate's public API as a SysML package with `@rust`
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
    /// `perform`ed actions of imported `@rust`-bound APIs become methods
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
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let format = cli.format;
    match cli.command {
        Command::Parse { files, tree } => parse_files(&files, tree, format),
        Command::Stats { files } => stats(&files, format),
        Command::Export {
            files,
            library,
            output,
        } => export(&files, &library, output.as_deref()),
        Command::Fmt {
            files,
            write,
            check,
        } => fmt(&files, write, check, format),
        Command::Check { paths, show } => check(&paths, show, format),
        Command::Diagram {
            paths,
            library,
            internal,
            browser,
            sequence,
            elk,
            elk_command,
            output,
        } => diagram(
            &paths,
            &library,
            internal.as_deref(),
            browser,
            sequence.as_deref(),
            elk.then_some(elk_command.as_str()),
            output.as_deref(),
        ),
        Command::ImportRust {
            json,
            package,
            output,
        } => import_rust(&json, package.as_deref(), output.as_deref()),
        Command::Rustgen {
            paths,
            library,
            output,
        } => rustgen(&paths, &library, output.as_deref()),
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
            sysml_cli::mcp::serve(
                &mut sysml_cli::mcp::Server::with_project(library.as_deref(), project.as_deref()),
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

fn export(files: &[PathBuf], library: &[PathBuf], output: Option<&Path>) -> ExitCode {
    let Some((json, elements)) = exported(files, library) else {
        return ExitCode::FAILURE;
    };
    let rendered = serde_json::to_string_pretty(&json).expect("serializable");
    match output {
        Some(path) => {
            if let Err(err) = std::fs::write(path, rendered) {
                eprintln!("error: cannot write {}: {err}", path.display());
                return ExitCode::FAILURE;
            }
            eprintln!("wrote {elements} element(s) to {}", path.display());
        }
        None => println!("{rendered}"),
    }
    ExitCode::SUCCESS
}

/// The model of `files`, resolved against `library`, as interchange JSON
/// and the number of elements it came from. `sysml export` writes this
/// out and `sysml api push` sends it, so both mean the same thing by a
/// model.
fn exported(files: &[PathBuf], library: &[PathBuf]) -> Option<(serde_json::Value, usize)> {
    // resolve before serializing: the reified typings and specializations
    // are what the interchange derives inheritance and types from
    let mut ws = sysml_semantics::Workspace::new();
    load_paths(&mut ws, files).map_err(|e| e.say()).ok()?;
    let own = ws.file_count();
    load_paths(&mut ws, library).map_err(|e| e.say()).ok()?;
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

    let model = ws.model();
    Some((sysml_interchange::to_json_with(model, &extras), model.len()))
}

fn fmt(files: &[PathBuf], write: bool, check_only: bool, format: Format) -> ExitCode {
    let mut dirty = 0usize;
    let mut unformatted = Vec::new();
    let mut broken = Vec::new();
    for path in files {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => return Unreadable::refuse(path, err, "fmt", format),
        };
        // Re-spacing a file the parser could not follow can move where a
        // quote or a comment ends -- `package Name' {` runs the quote on
        // to wherever the next one is, and putting the tokens back with
        // different spacing puts the name somewhere else. Printing that
        // is harmless, since every character is still there to read.
        // Writing it over the modeller's file is not, so `--write`
        // reports such a file instead of rewriting it.
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
        let formatted = sysml_syntax::fmt::format_parsed(&parse);
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

fn diagram(
    paths: &[PathBuf],
    library: &[PathBuf],
    internal: Option<&str>,
    browser: bool,
    sequence: Option<&str>,
    elk: Option<&str>,
    output: Option<&Path>,
) -> ExitCode {
    let mut ws = sysml_semantics::Workspace::new();
    if let Err(unreadable) = load_paths(&mut ws, paths) {
        unreadable.say();
        return ExitCode::FAILURE;
    }
    // everything loaded so far is drawn; the library that follows only has
    // to be resolvable, so its definitions never become boxes
    let drawn = ws.file_count();
    if let Err(unreadable) = load_paths(&mut ws, library) {
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
    if browser {
        let view = sysml_diagram::browser_view(ws.model(), &roots);
        if view.rows.is_empty() {
            eprintln!("error: nothing to draw");
            return ExitCode::FAILURE;
        }
        let svg = sysml_diagram::render_browser(&view, &sysml_diagram::Style::default());
        return emit(&svg, output, &format!("{} row(s)", view.rows.len()));
    }
    if let Some(name) = sequence {
        let Some(target) = named(&ws, &own, name) else {
            return ExitCode::FAILURE;
        };
        let view = sysml_diagram::sequence_view(ws.model(), target);
        if view.lifelines.is_empty() {
            eprintln!("error: `{name}` declares no interaction to draw");
            return ExitCode::FAILURE;
        }
        let svg = sysml_diagram::render_sequence(&view, &sysml_diagram::Style::default());
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
    let diagram = match internal {
        Some(name) => {
            let Some(target) = named(&ws, &own, name) else {
                return ExitCode::FAILURE;
            };
            sysml_diagram::interconnection_diagram(ws.model(), target)
        }
        None => sysml_diagram::definition_diagram(ws.model(), &roots),
    };
    if diagram.nodes.is_empty() {
        eprintln!("error: nothing to draw");
        return ExitCode::FAILURE;
    }
    let style = sysml_diagram::Style::default();
    let svg = match elk {
        Some(command) => match sysml_diagram::render_with_elk(&diagram, &style, command) {
            Ok(svg) => svg,
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
        },
        None => sysml_diagram::render(&diagram, &style),
    };
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
fn rustgen(paths: &[PathBuf], library: &[PathBuf], output: Option<&Path>) -> ExitCode {
    let mut ws = sysml_semantics::Workspace::new();
    if let Err(unreadable) = load_paths(&mut ws, paths) {
        unreadable.say();
        return ExitCode::FAILURE;
    }
    // generation covers what was named; the library only resolves
    let own = ws.file_count();
    if let Err(unreadable) = load_paths(&mut ws, library) {
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

fn check(paths: &[PathBuf], show: usize, format: Format) -> ExitCode {
    let mut ws = sysml_semantics::Workspace::new();
    if let Err(unreadable) = load_paths(&mut ws, paths) {
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
    // A file that does not parse has no names to resolve, so resolution
    // finds nothing wrong with it and this used to answer `ok`. Anyone
    // running only `check` -- which is most of the reason it exists --
    // would be told a broken model was fine. Syntax comes first, as it
    // does in the MCP server's tool of the same name.
    let syntax = ws.findings(&[]).syntax;
    let texts = held_texts(&ws, &syntax);
    let mut broken = Vec::new();
    for finding in &syntax {
        let name = ws.file_name(finding.file);
        let text = &texts[&finding.file];
        let offset = usize::from(finding.range.start());
        if format == Format::Text {
            let (line, col) = sysml_syntax::line_col(text, offset.min(text.len()));
            eprintln!("{name}:{line}:{col}: {}", finding.what);
        }
        broken.push(sysml_cli::at(
            text,
            offset,
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
    let found = ws.findings(&[]);
    let names = found.names;
    let shown = &names[..limit.min(names.len())];
    let texts = held_texts(&ws, shown);
    let mut unresolved = Vec::new();
    for u in shown {
        let file = ws.file_name(u.file);
        let text = &texts[&u.file];
        let offset = usize::from(u.range.start());
        match format {
            Format::Text => {
                let (line, col) = sysml_syntax::line_col(text, offset.min(text.len()));
                eprintln!("{file}:{line}:{col}: unresolved `{}`", u.what);
            }
            Format::Json => unresolved.push(sysml_cli::at(
                text,
                offset,
                serde_json::json!({ "path": file, "name": u.what.clone() }),
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
        let offset = usize::from(c.range.start()).min(text.len());
        match format {
            Format::Text => {
                let (line, col) = sysml_syntax::line_col(text, offset);
                eprintln!("{file}:{line}:{col}: {}", c.what);
            }
            Format::Json => collisions.push(sysml_cli::at(
                text,
                offset,
                serde_json::json!({ "path": file, "message": c.what.clone() }),
            )),
        }
    }
    if format == Format::Json {
        report(serde_json::json!({
            "command": "check",
            "ok": stats.unresolved == 0,
            "unreadable": [],
            "elements": ws.model().len(),
            "parseErrors": [],
            "resolved": stats.resolved,
            "references": total,
            "unresolved": unresolved,
            "collisions": collisions,
        }));
        return if stats.unresolved == 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    if names.len() > limit {
        eprintln!("... and {} more", names.len() - limit);
    }
    println!(
        "{} element(s), {}/{total} reference(s) resolved ({rate:.1}%)",
        ws.model().len(),
        stats.resolved
    );
    if stats.unresolved == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// The text of each file these findings are in, so that they can be
/// placed after `ws` has been borrowed again to resolve. It comes from
/// the workspace rather than from a second read of the file: `file_name`
/// is a lossy rendering of the path, so a path that is not UTF-8 names
/// no file, and the second read would find nothing and place every
/// finding at 1:1.
fn held_texts(
    ws: &sysml_semantics::Workspace,
    findings: &[sysml_semantics::Finding],
) -> std::collections::HashMap<usize, String> {
    findings
        .iter()
        .map(|f| f.file)
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
                    .map(|d| sysml_cli::at(&text, usize::from(d.range.start()), serde_json::json!({
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
    let offset = usize::from(diagnostic.range.start());
    let (line, col) = sysml_syntax::line_col(text, offset);
    eprintln!(
        "{}:{line}:{col}: error: {}",
        path.display(),
        diagnostic.message
    );
    if let Some(written) = text.lines().nth(line - 1) {
        // `col` counts bytes, as an editor's offsets do, and the caret
        // counts characters: a name written in Japanese is three bytes a
        // letter, and the caret landed well past what it points at
        let before = written
            .char_indices()
            .take_while(|(byte, _)| *byte < col - 1)
            .count();
        eprintln!("    | {written}");
        eprintln!("    | {}^", " ".repeat(before));
    }
}

/// Talk to a model server. Every answer is the server's own JSON, so
/// what comes back is what the standard says came back; the text form
/// lists the one line a person reads it for.
fn api_command(
    server: &str,
    timeout: std::time::Duration,
    what: &ApiCommand,
    format: Format,
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
        } => {
            let Some((json, elements)) = exported(paths, library) else {
                return ExitCode::FAILURE;
            };
            let changes: Vec<serde_json::Value> = json.as_array().cloned().unwrap_or_default();
            client.create_commit(project, message, &changes).map(|c| {
                (
                    vec![format!("{} element(s) as commit {}", elements, c.id)],
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

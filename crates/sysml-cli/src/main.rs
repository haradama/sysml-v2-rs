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
    /// legal at a point, and what the standard library declares
    Mcp {
        /// The standard library, so that references into it resolve;
        /// `SYSML_LIBRARY_PATH` says the same thing
        #[arg(long)]
        library: Option<PathBuf>,
    },
    /// Talk to a SysML v2 API & Services model server
    Api {
        #[command(subcommand)]
        what: ApiCommand,
        /// Base URL of the model server
        #[arg(long, default_value = "http://localhost:9000", global = true)]
        server: String,
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
            elk,
            elk_command,
            output,
        } => diagram(
            &paths,
            &library,
            internal.as_deref(),
            browser,
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
        Command::Mcp { library } => {
            // an agent's launcher often has nowhere to put a flag, so the
            // environment says it too -- the language server reads the
            // same variable
            let library =
                library.or_else(|| std::env::var_os("SYSML_LIBRARY_PATH").map(Into::into));
            // there is nowhere to report a failure to write: the only
            // way this ends badly is the client going away mid-answer,
            // and the exit code is what its launcher reads
            sysml_cli::mcp::serve(
                &mut sysml_cli::mcp::Server::new(library.as_deref()),
                std::io::BufReader::new(std::io::stdin()),
                std::io::stdout(),
            )
            .map_or(ExitCode::FAILURE, |()| ExitCode::SUCCESS)
        }
        Command::Api { what, server } => api_command(&server, &what, format),
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
            Err(err) => {
                eprintln!("error: cannot read {}: {err}", path.display());
                return ExitCode::FAILURE;
            }
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
            "elements": total,
            "parseErrors": errors,
            "counts": counts,
        }));
        return ExitCode::SUCCESS;
    }
    let mut rows: Vec<_> = counts.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    for (kind, n) in rows {
        println!("{n:6}  {kind}");
    }
    println!("{total:6}  total elements ({errors} parse error(s))");
    ExitCode::SUCCESS
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
    if !load_paths(&mut ws, files) {
        return None;
    }
    let own = ws.file_count();
    if !load_paths(&mut ws, library) {
        return None;
    }
    // parse diagnostics still get printed while exporting
    for file in 0..own {
        let parse = ws.file_parse(file);
        let text = parse.syntax().text().to_string();
        for diagnostic in parse.errors() {
            print_diagnostic(Path::new(ws.file_name(file)), &text, diagnostic);
        }
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
            Err(err) => {
                eprintln!("error: cannot read {}: {err}", path.display());
                return ExitCode::FAILURE;
            }
        };
        // Re-spacing a file the parser could not follow can move where a
        // quote or a comment ends -- `package Name' {` runs the quote on
        // to wherever the next one is, and putting the tokens back with
        // different spacing puts the name somewhere else. Printing that
        // is harmless, since every character is still there to read.
        // Writing it over the modeller's file is not, so `--write`
        // reports such a file instead of rewriting it.
        if write && !parse_file(path, &text).ok() {
            eprintln!(
                "error: {} does not parse; `sysml parse` says where",
                path.display()
            );
            broken.push(path.display().to_string());
            continue;
        }
        let formatted = sysml_syntax::fmt::format_file(&path.to_string_lossy(), &text);
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
    if check_only && format == Format::Json {
        report(serde_json::json!({
            "command": "fmt",
            "ok": dirty == 0,
            "unformatted": unformatted,
        }));
    }
    if dirty > 0 || !broken.is_empty() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Load every path -- file or directory -- into `ws`. Reports the first path
/// that cannot be read and returns `false`.
fn load_paths(ws: &mut sysml_semantics::Workspace, paths: &[PathBuf]) -> bool {
    for path in paths {
        if path.is_dir() {
            if let Err(err) = ws.load_dir(path) {
                eprintln!("error: cannot load {}: {err}", path.display());
                return false;
            }
        } else {
            match std::fs::read_to_string(path) {
                Ok(text) => {
                    ws.add_file(path.to_string_lossy(), &text);
                }
                Err(err) => {
                    eprintln!("error: cannot read {}: {err}", path.display());
                    return false;
                }
            }
        }
    }
    true
}

fn diagram(
    paths: &[PathBuf],
    library: &[PathBuf],
    internal: Option<&str>,
    browser: bool,
    elk: Option<&str>,
    output: Option<&Path>,
) -> ExitCode {
    let mut ws = sysml_semantics::Workspace::new();
    if !load_paths(&mut ws, paths) {
        return ExitCode::FAILURE;
    }
    // everything loaded so far is drawn; the library that follows only has
    // to be resolvable, so its definitions never become boxes
    let drawn = ws.file_count();
    if !load_paths(&mut ws, library) {
        return ExitCode::FAILURE;
    }
    // only what is drawn, and what it reaches: a library is loaded so
    // that names resolve, not so that all of it is worked through
    let files: Vec<usize> = (0..drawn).collect();
    ws.resolve_reached(&files);
    let roots: Vec<_> = (0..drawn)
        .flat_map(|file| ws.file_roots(file).to_vec())
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
    let diagram = match internal {
        Some(name) => {
            let Some(target) = ws
                .named_elements()
                .find(|(_, declared)| *declared == name)
                .map(|(id, _)| id)
            else {
                eprintln!("error: no element named `{name}`");
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
         {} transition(s) and {} satisfaction(s)",
        diagram.nodes.len(),
        count(Relation::Specialization),
        count(Relation::Composition),
        count(Relation::Reference),
        count(Relation::Subsetting) + count(Relation::Redefinition),
        // an interface and a binding are connections too -- what each is,
        // the drawing says on the line
        count(Relation::Connection) + count(Relation::Interface) + count(Relation::Binding),
        count(Relation::Flow) + count(Relation::SuccessionFlow) + count(Relation::Message),
        count(Relation::Allocation),
        count(Relation::Transition),
        count(Relation::Satisfy),
    );
    emit(&svg, output, &summary)
}

/// Write a rendered view to `output`, or to stdout when there is none.
fn rustgen(paths: &[PathBuf], library: &[PathBuf], output: Option<&Path>) -> ExitCode {
    let mut ws = sysml_semantics::Workspace::new();
    if !load_paths(&mut ws, paths) {
        return ExitCode::FAILURE;
    }
    // generation covers what was named; the library only resolves
    let own = ws.file_count();
    if !load_paths(&mut ws, library) {
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
        Ok(rust) => {
            let structs = rust.matches("pub struct ").count();
            let methods =
                rust.matches("    pub fn ").count() + rust.matches("    pub async fn ").count();
            emit(
                &rust,
                output,
                &format!("{structs} struct(s) and {methods} method(s)"),
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
        Ok(sysml) => {
            let definitions = sysml.matches(" def ").count();
            emit(&sysml, output, &format!("{definitions} definition(s)"))
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
    if !load_paths(&mut ws, paths) {
        return ExitCode::FAILURE;
    }
    // A file that does not parse has no names to resolve, so resolution
    // finds nothing wrong with it and this used to answer `ok`. Anyone
    // running only `check` -- which is most of the reason it exists --
    // would be told a broken model was fine. Syntax comes first, as it
    // does in the MCP server's tool of the same name.
    // the file each finding is in, read once and kept: `ws` is borrowed
    // mutably in between to resolve
    let mut texts: std::collections::HashMap<usize, String> = Default::default();
    let mut read = |file: usize, name: &str| -> String {
        texts
            .entry(file)
            .or_insert_with(|| std::fs::read_to_string(name).unwrap_or_default())
            .clone()
    };
    let mut broken = Vec::new();
    for finding in &ws.findings(&[]).syntax {
        let name = ws.file_name(finding.file).to_string();
        let text = read(finding.file, &name);
        let offset = usize::from(finding.range.start()).min(text.len());
        if format == Format::Text {
            let (line, col) = sysml_syntax::line_col(&text, offset);
            eprintln!("{name}:{line}:{col}: {}", finding.what);
        }
        broken.push(at(
            &text,
            offset,
            serde_json::json!({ "path": name, "message": finding.what }),
        ));
    }
    if !broken.is_empty() {
        if format == Format::Json {
            report(serde_json::json!({
                "command": "check",
                "ok": false,
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
    let names = ws.findings(&[]).names;
    let mut unresolved = Vec::new();
    for u in names.iter().take(limit) {
        let file = ws.file_name(u.file).to_string();
        let text = &read(u.file, &file);
        let offset = usize::from(u.range.start()).min(text.len());
        match format {
            Format::Text => {
                let (line, col) = sysml_syntax::line_col(text, offset);
                eprintln!("{file}:{line}:{col}: unresolved `{}`", u.what);
            }
            Format::Json => unresolved.push(at(
                text,
                offset,
                serde_json::json!({ "path": file, "name": u.what.clone() }),
            )),
        }
    }
    if format == Format::Json {
        report(serde_json::json!({
            "command": "check",
            "ok": stats.unresolved == 0,
            "elements": ws.model().len(),
            "parseErrors": [],
            "resolved": stats.resolved,
            "references": total,
            "unresolved": unresolved,
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

fn corpus(dir: &Path, worst: usize, list_failures: bool) -> ExitCode {
    let mut files = Vec::new();
    collect_files(dir, &mut files);
    files.sort();
    if files.is_empty() {
        eprintln!("no .sysml/.kerml files found under {}", dir.display());
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

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("sysml" | "kerml")
        ) {
            out.push(path);
        }
    }
}

fn parse_files(files: &[PathBuf], dump_tree: bool, format: Format) -> ExitCode {
    let mut total_errors = 0usize;
    let mut reported = Vec::new();
    for path in files {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => {
                total_errors += 1;
                match format {
                    Format::Text => eprintln!("error: cannot read {}: {err}", path.display()),
                    Format::Json => reported.push(serde_json::json!({
                        "path": path.display().to_string(),
                        "unreadable": err.to_string(),
                    })),
                }
                continue;
            }
        };
        let parse = parse_file(path, &text);
        if dump_tree {
            println!("{:#?}", parse.syntax());
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
                    .map(|d| at(&text, usize::from(d.range.start()), serde_json::json!({
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

/// A finding with its place in the file: byte offset as the model sees
/// it, line and column as an editor counts them (from one).
fn at(text: &str, offset: usize, mut value: serde_json::Value) -> serde_json::Value {
    let (line, column) = sysml_syntax::line_col(text, offset);
    let map = value.as_object_mut().expect("built as an object");
    map.insert("offset".into(), offset.into());
    map.insert("line".into(), line.into());
    map.insert("column".into(), column.into());
    value
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
        eprintln!("    | {written}");
        eprintln!("    | {}^", " ".repeat(col - 1));
    }
}

/// Talk to a model server. Every answer is the server's own JSON, so
/// what comes back is what the standard says came back; the text form
/// lists the one line a person reads it for.
fn api_command(server: &str, what: &ApiCommand, format: Format) -> ExitCode {
    let client = api::Client::new(server);
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
            eprintln!("error: {server}: {err}");
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

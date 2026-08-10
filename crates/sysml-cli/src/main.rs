use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use sysml_syntax::{Diagnostic, Dialect};

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
        /// Let Graphviz `dot` decide the positions (PlantUML-style: the
        /// drawing itself stays the same); needs Graphviz installed
        #[arg(long)]
        graphviz: bool,
        /// The Graphviz command to run with --graphviz
        #[arg(long, default_value = "dot", value_name = "COMMAND")]
        dot: String,
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
            graphviz,
            dot,
            output,
        } => diagram(
            &paths,
            &library,
            internal.as_deref(),
            browser,
            graphviz.then_some(dot.as_str()),
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
    // resolve before serializing: the reified typings and specializations
    // are what the interchange derives inheritance and types from
    let mut ws = sysml_semantics::Workspace::new();
    if !load_paths(&mut ws, files) {
        return ExitCode::FAILURE;
    }
    let own = ws.file_count();
    if !load_paths(&mut ws, library) {
        return ExitCode::FAILURE;
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
    let json = sysml_interchange::to_json_with(model, &extras);
    let rendered = serde_json::to_string_pretty(&json).expect("serializable");
    match output {
        Some(path) => {
            if let Err(err) = std::fs::write(path, rendered) {
                eprintln!("error: cannot write {}: {err}", path.display());
                return ExitCode::FAILURE;
            }
            eprintln!("wrote {} element(s) to {}", model.len(), path.display());
        }
        None => println!("{rendered}"),
    }
    ExitCode::SUCCESS
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
    graphviz: Option<&str>,
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
    ws.resolve_all();
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
    let svg = match graphviz {
        Some(command) => match sysml_diagram::render_with_graphviz(&diagram, &style, command) {
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
    let summary = format!(
        "{} box(es), {} specialization(s), {} composition(s), {} connection(s), \
         {} transition(s) and {} satisfaction(s)",
        diagram.nodes.len(),
        count(sysml_diagram::Relation::Specialization),
        count(sysml_diagram::Relation::Composition),
        count(sysml_diagram::Relation::Connection),
        count(sysml_diagram::Relation::Transition),
        count(sysml_diagram::Relation::Satisfy),
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
    let stats = ws.resolve_all();
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
    match sysml_rustgen::generate(ws.model(), &roots) {
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
    match sysml_import_api::rustdoc_to_sysml(&text, package) {
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
    let mut broken = Vec::new();
    for file in 0..ws.file_count() {
        let parse = ws.file_parse(file);
        if parse.ok() {
            continue;
        }
        let name = ws.file_name(file).to_string();
        let text = std::fs::read_to_string(&name).unwrap_or_default();
        for error in parse.errors() {
            let offset = usize::from(error.range.start()).min(text.len());
            if format == Format::Text {
                let (line, col) = line_col(&text, offset);
                eprintln!("{name}:{}:{}: {}", line + 1, col + 1, error.message);
            }
            broken.push(at(
                &text,
                offset,
                serde_json::json!({ "path": name.clone(), "message": error.message }),
            ));
        }
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
    let mut texts: std::collections::HashMap<usize, String> = Default::default();
    let mut unresolved = Vec::new();
    for u in ws.unresolved().iter().take(limit) {
        let file = ws.file_name(u.file).to_string();
        let text = texts
            .entry(u.file)
            .or_insert_with(|| std::fs::read_to_string(&file).unwrap_or_default());
        let offset = usize::from(u.range.start()).min(text.len());
        match format {
            Format::Text => {
                let (line, col) = line_col(text, offset);
                eprintln!("{file}:{}:{}: unresolved `{}`", line + 1, col + 1, u.name);
            }
            Format::Json => unresolved.push(at(
                text,
                offset,
                serde_json::json!({ "path": file, "name": u.name.clone() }),
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
    if ws.unresolved().len() > limit {
        eprintln!("... and {} more", ws.unresolved().len() - limit);
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
    let (line, column) = line_col(text, offset);
    let map = value.as_object_mut().expect("built as an object");
    map.insert("offset".into(), offset.into());
    map.insert("line".into(), (line + 1).into());
    map.insert("column".into(), (column + 1).into());
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
    let (line_idx, col) = line_col(text, offset);
    eprintln!(
        "{}:{}:{}: error: {}",
        path.display(),
        line_idx + 1,
        col + 1,
        diagnostic.message
    );
    if let Some(line) = text.lines().nth(line_idx) {
        eprintln!("    | {line}");
        eprintln!("    | {}^", " ".repeat(col));
    }
}

fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let prefix = &text[..offset.min(text.len())];
    let line = prefix.matches('\n').count();
    let col = prefix.rfind('\n').map_or(offset, |i| offset - i - 1);
    (line, col)
}

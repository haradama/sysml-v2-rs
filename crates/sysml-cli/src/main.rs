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
    #[command(subcommand)]
    command: Command,
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
    match cli.command {
        Command::Parse { files, tree } => parse_files(&files, tree),
        Command::Stats { files } => stats(&files),
        Command::Export { files, output } => export(&files, output.as_deref()),
        Command::Fmt {
            files,
            write,
            check,
        } => fmt(&files, write, check),
        Command::Check { paths, show } => check(&paths, show),
        Command::Diagram {
            paths,
            library,
            internal,
            output,
        } => diagram(&paths, &library, internal.as_deref(), output.as_deref()),
        Command::Corpus {
            dir,
            worst,
            failures,
        } => corpus(&dir, worst, failures),
    }
}

fn stats(files: &[PathBuf]) -> ExitCode {
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
    let mut rows: Vec<_> = counts.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    for (kind, n) in rows {
        println!("{n:6}  {kind}");
    }
    println!("{total:6}  total elements ({errors} parse error(s))");
    ExitCode::SUCCESS
}

fn export(files: &[PathBuf], output: Option<&Path>) -> ExitCode {
    let mut model = sysml_model::Model::new();
    for path in files {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => {
                eprintln!("error: cannot read {}: {err}", path.display());
                return ExitCode::FAILURE;
            }
        };
        let parse = parse_file(path, &text);
        for diagnostic in parse.errors() {
            print_diagnostic(path, &text, diagnostic);
        }
        sysml_model::build_into(&mut model, &parse);
    }
    let json = sysml_interchange::to_json(&model);
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

fn fmt(files: &[PathBuf], write: bool, check_only: bool) -> ExitCode {
    let mut dirty = 0usize;
    for path in files {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => {
                eprintln!("error: cannot read {}: {err}", path.display());
                return ExitCode::FAILURE;
            }
        };
        let formatted = sysml_syntax::fmt::format_file(&path.to_string_lossy(), &text);
        if check_only {
            if formatted != text {
                eprintln!("{}: not formatted", path.display());
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
    if dirty > 0 {
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
        None => {
            let roots: Vec<_> = (0..drawn)
                .flat_map(|file| ws.file_roots(file).to_vec())
                .collect();
            sysml_diagram::definition_diagram(ws.model(), &roots)
        }
    };
    if diagram.nodes.is_empty() {
        eprintln!("error: nothing to draw");
        return ExitCode::FAILURE;
    }
    let svg = sysml_diagram::render(&diagram, &sysml_diagram::Style::default());
    match output {
        Some(path) => {
            if let Err(err) = std::fs::write(path, svg) {
                eprintln!("error: cannot write {}: {err}", path.display());
                return ExitCode::FAILURE;
            }
            let count = |relation| {
                diagram
                    .edges
                    .iter()
                    .filter(|edge| edge.relation == relation)
                    .count()
            };
            eprintln!(
                "wrote {} box(es), {} specialization(s), {} composition(s) and {} \
                 connection(s) to {}",
                diagram.nodes.len(),
                count(sysml_diagram::Relation::Specialization),
                count(sysml_diagram::Relation::Composition),
                count(sysml_diagram::Relation::Connection),
                path.display()
            );
        }
        None => print!("{svg}"),
    }
    ExitCode::SUCCESS
}

fn check(paths: &[PathBuf], show: usize) -> ExitCode {
    let mut ws = sysml_semantics::Workspace::new();
    if !load_paths(&mut ws, paths) {
        return ExitCode::FAILURE;
    }
    let stats = ws.resolve_all();
    let total = stats.resolved + stats.unresolved;
    let rate = if total == 0 {
        100.0
    } else {
        100.0 * stats.resolved as f64 / total as f64
    };
    let limit = if show == 0 { usize::MAX } else { show };
    let mut texts: std::collections::HashMap<usize, String> = Default::default();
    for u in ws.unresolved().iter().take(limit) {
        let file = ws.file_name(u.file).to_string();
        let text = texts
            .entry(u.file)
            .or_insert_with(|| std::fs::read_to_string(&file).unwrap_or_default());
        let offset = usize::from(u.range.start()).min(text.len());
        let (line, col) = line_col(text, offset);
        eprintln!("{file}:{}:{}: unresolved `{}`", line + 1, col + 1, u.name);
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

fn parse_files(files: &[PathBuf], dump_tree: bool) -> ExitCode {
    let mut total_errors = 0usize;
    for path in files {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => {
                eprintln!("error: cannot read {}: {err}", path.display());
                total_errors += 1;
                continue;
            }
        };
        let parse = parse_file(path, &text);
        if dump_tree {
            println!("{:#?}", parse.syntax());
        }
        for diagnostic in parse.errors() {
            print_diagnostic(path, &text, diagnostic);
        }
        total_errors += parse.errors().len();
        let status = if parse.ok() { "ok" } else { "FAILED" };
        eprintln!(
            "{}: {status} ({} error(s))",
            path.display(),
            parse.errors().len()
        );
    }
    if total_errors == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
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

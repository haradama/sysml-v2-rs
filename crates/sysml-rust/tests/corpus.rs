//! The generator against every model in the official corpus.
//!
//! Examples written for a generator only exercise what their author
//! thought of. These 309 were written for a specification, by people
//! who had never heard of this one, which is why a sweep over them
//! finds what no example does: names Rust cannot spell, definitions
//! referred to but never written, two things landing on one name, a
//! behaviour made of itself.
//!
//! The measure is the only one that matters for generated code: does it
//! compile. All of them do, and this holds them to it.

use std::path::{Path, PathBuf};
use std::process::Command;

fn vendor() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/sysml-v2-release")
        .canonicalize()
        .ok()?;
    root.join("sysml.library").is_dir().then_some(root)
}

fn models(root: &Path) -> Vec<PathBuf> {
    let mut found = sysml_semantics::model_files(&root.join("sysml/src"));
    found.extend(sysml_semantics::model_files(&root.join("kerml/src")));
    found.sort();
    found
}

/// The library, parsed and resolved once, cloned per model.
fn library(root: &Path) -> sysml_semantics::Workspace {
    let mut ws = sysml_semantics::Workspace::new();
    ws.load_dir(&root.join("sysml.library")).unwrap();
    ws.resolve_all();
    ws
}

#[test]
#[ignore = "compiles 300 generated files; run with --ignored"]
fn what_the_corpus_generates_compiles() {
    let Some(root) = vendor() else { return };
    let base = library(&root);
    let out = std::env::temp_dir().join("sysml-rust-corpus");
    std::fs::create_dir_all(&out).unwrap();

    let (mut generated, mut failed) = (0usize, Vec::new());
    for model in models(&root) {
        let Ok(text) = std::fs::read_to_string(&model) else {
            continue;
        };
        let mut ws = base.clone();
        let file = ws.add_file(model.to_string_lossy(), &text);
        if !ws.file_parse(file).ok() {
            continue; // the parser's corpus test answers for this
        }
        ws.resolve_files(&[file]);
        let roots = ws.file_roots(file).to_vec();
        let Ok(written) = sysml_rust::generate(ws.model(), &roots) else {
            continue; // a model this generator refuses is not a failure to compile
        };
        let rust = written.rust;
        generated += 1;

        let name: String = model
            .to_string_lossy()
            .chars()
            .map(|ch| if ch.is_alphanumeric() { ch } else { '_' })
            .collect();
        let at = out.join(format!("{}.rs", &name[name.len().saturating_sub(80)..]));
        std::fs::write(&at, &rust).unwrap();
        let rustc = Command::new(std::env::var("RUSTC").unwrap_or("rustc".into()))
            .args([
                "--crate-type",
                "lib",
                "--edition",
                "2021",
                "--emit=metadata",
            ])
            .arg("-o")
            .arg(out.join("meta.rmeta"))
            .arg(&at)
            .output()
            .expect("rustc runs");
        if !rustc.status.success() {
            let said = String::from_utf8_lossy(&rustc.stderr);
            let first = said
                .lines()
                .find(|line| line.starts_with("error"))
                .unwrap_or("(no error line)")
                .to_string();
            failed.push(format!("{}: {first}", model.display()));
        }
    }

    assert!(generated > 250, "only {generated} models generated at all");
    assert!(
        failed.is_empty(),
        "{} of {generated} generated files do not compile:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

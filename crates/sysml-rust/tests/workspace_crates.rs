//! Every crate in this workspace, through rustdoc and the importer.
//!
//! The checked-in fixture is a crate written for this test; these are
//! eleven real ones, with re-exports, error enums carrying payloads and
//! types that have no SysML shape. What comes out must parse, must
//! format, and -- the point of the exercise -- must resolve completely:
//! a package that names a type it never declared is worse than one that
//! leaves the type out and says so.
//!
//! Every crate in this workspace, through rustdoc, through the importer,
//! and back through the parser and the resolver.

use std::path::PathBuf;
use std::process::Command;

fn docs() -> Vec<(String, PathBuf)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = std::env::temp_dir().join("sysml-import-sweep");
    let made = Command::new("cargo")
        .current_dir(&root)
        .env("RUSTC_BOOTSTRAP", "1")
        .env("RUSTDOCFLAGS", "-Zunstable-options --output-format=json")
        .args([
            "doc",
            "--workspace",
            "--lib",
            "--no-deps",
            "--target-dir",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("cargo runs");
    if !made.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&made.stderr));
        return Vec::new();
    }
    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(out.join("doc")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                let name = path.file_stem().unwrap().to_string_lossy().to_string();
                found.push((name, path));
            }
        }
    }
    found.sort();
    found
}

#[test]
#[ignore = "builds rustdoc JSON for the workspace; run with --ignored"]
fn every_crate_imports_into_a_model_that_resolves() {
    let all = docs();
    assert!(!all.is_empty(), "no rustdoc json produced");
    eprintln!("{} crates", all.len());
    let mut found: Vec<String> = Vec::new();

    for (name, path) in &all {
        let Ok(json) = std::fs::read_to_string(path) else {
            continue;
        };
        let made = std::panic::catch_unwind(|| sysml_rust::rustdoc_to_sysml(&json, None));
        let sysml = match made {
            Err(_) => {
                found.push(format!("PANIC\t{name}"));
                continue;
            }
            Ok(Err(e)) => {
                found.push(format!("refused\t{name}\t{e}"));
                continue;
            }
            Ok(Ok(imported)) => imported.sysml,
        };

        let parse = sysml_syntax::parse(&sysml);
        if !parse.errors().is_empty() {
            found.push(format!(
                "does-not-parse\t{name}\t{:?}",
                parse.errors().first()
            ));
            std::fs::write(std::env::temp_dir().join(format!("{name}.sysml")), &sysml).ok();
            continue;
        }
        if parse.syntax().text().to_string() != sysml {
            found.push(format!("lossy\t{name}"));
        }

        // formatting it must not change what it says
        let formatted = sysml_syntax::fmt::format(&sysml, sysml_syntax::Dialect::SysML);
        if !sysml_syntax::parse(&formatted).errors().is_empty() {
            found.push(format!("fmt-broke-parse\t{name}"));
        }

        // and every name in it must resolve against the standard library
        let mut ws = sysml_semantics::Workspace::new();
        let lib = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sysml-stdlib/library");
        if lib.is_dir() {
            ws.load_dir(&lib).unwrap();
        }
        let file = ws.add_file(format!("{name}.sysml"), &sysml);
        ws.resolve_all();
        let mine: Vec<_> = ws
            .unresolved()
            .iter()
            .filter(|u| u.file == file)
            .map(|u| u.name.clone())
            .collect();
        if !mine.is_empty() {
            found.push(format!(
                "unresolved\t{name}\t{} {:?}",
                mine.len(),
                &mine[..mine.len().min(6)]
            ));
            std::fs::write(std::env::temp_dir().join(format!("{name}.sysml")), &sysml).ok();
        }
    }

    eprintln!("--- {} findings", found.len());
    for line in &found {
        eprintln!("{line}");
    }
    assert!(found.is_empty());
}

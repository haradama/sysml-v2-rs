//! JSON round-trip over the whole standard library. Skipped when the
//! corpus submodule is not checked out.

use std::path::Path;

use sysml_model::{build_into, Model};
use sysml_syntax::{parse_dialect, Dialect};

fn collect_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
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

#[test]
fn library_round_trips_through_json() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/sysml-v2-release/sysml.library");
    if !root.exists() {
        eprintln!("skipping: {} not checked out", root.display());
        return;
    }
    let mut files = Vec::new();
    collect_files(&root, &mut files);
    files.sort();

    let mut model = Model::new();
    for path in &files {
        let text = std::fs::read_to_string(path).unwrap();
        let dialect = match path.extension().and_then(|e| e.to_str()) {
            Some("kerml") => Dialect::KerML,
            _ => Dialect::SysML,
        };
        build_into(&mut model, &parse_dialect(&text, dialect));
    }
    assert!(model.len() > 10_000);

    let json = sysml_interchange::to_json(&model);
    // no UUID collisions
    let ids: std::collections::HashSet<&str> = json
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["@id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), model.len(), "duplicate element UUIDs");

    let (rebuilt, _roots) = sysml_interchange::from_json(&json).unwrap();
    assert_eq!(rebuilt.len(), model.len());
    assert_eq!(sysml_interchange::to_json(&rebuilt), json);
}

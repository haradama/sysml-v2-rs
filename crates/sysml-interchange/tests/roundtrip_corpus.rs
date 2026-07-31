//! JSON round-trip over the whole standard library, resolved first so the
//! reified typings and specializations -- and the derived properties
//! computed from them -- are exercised at full scale. Skipped when the
//! corpus submodule is not checked out.

use std::path::Path;

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

    let mut ws = sysml_semantics::Workspace::new();
    for path in &files {
        let text = std::fs::read_to_string(path).unwrap();
        ws.add_file(path.to_string_lossy(), &text);
    }
    ws.resolve_all();
    // the strict pipeline: the implied specializations become part of the
    // model before it is serialized
    let implied = ws.materialize_implied();
    assert!(implied > 1_000, "hardly anything was implied: {implied}");
    let model = ws.model();
    assert!(model.len() > 10_000);

    let json = sysml_interchange::to_json(model);
    // no UUID collisions -- among the elements and among the memberships
    // synthesized between them alike
    let objects = json.as_array().unwrap();
    let ids: std::collections::HashSet<&str> =
        objects.iter().map(|e| e["@id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), objects.len(), "duplicate element UUIDs");
    assert!(objects.len() > model.len(), "no memberships were written");

    // strict shape: every object carries exactly the property set its
    // metaclass declares, across the whole library
    for object in objects {
        let kind = sysml_model::ElementKind::from_name(object["@type"].as_str().unwrap()).unwrap();
        let mut declared: std::collections::BTreeSet<&str> = std::iter::once(kind)
            .chain(kind.ancestors().iter().copied())
            .flat_map(|k| k.own_features())
            .map(|meta| meta.name)
            .collect();
        declared.insert("@type");
        declared.insert("@id");
        let written: std::collections::BTreeSet<&str> = object
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(written, declared, "for a {}", kind.name());
    }

    let (rebuilt, _roots) = sysml_interchange::from_json(&json).unwrap();
    assert_eq!(rebuilt.len(), model.len());
    assert_eq!(sysml_interchange::to_json(&rebuilt), json);
}

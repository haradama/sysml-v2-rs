//! Structural checks of a drawing against the model it came from, run over
//! the vendored SysML-v2-Release corpus.
//!
//! The other corpus tests ask whether the model is right. These ask whether
//! the picture is faithful to it: that every box stands for an element in
//! scope, that no element is drawn twice or lost, that every line ends on a
//! box, and that the specializations the model holds are exactly the ones
//! drawn.
//!
//! Skipped when the submodule is not checked out.

use std::collections::HashSet;

use sysml_diagram::{definition_diagram, interconnection_diagram, Diagram, Relation, Shape};
use sysml_model::{ElementId, ElementKind, Model, Value};
use sysml_semantics::Workspace;

fn corpus() -> Option<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/sysml-v2-release/sysml/src");
    if root.is_dir() {
        return Some(root);
    }
    eprintln!("skipping: {} not checked out", root.display());
    None
}

fn sysml_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sysml_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "sysml") {
            out.push(path);
        }
    }
}

fn loaded(path: &std::path::Path) -> Workspace {
    let mut ws = Workspace::new();
    ws.add_file(
        path.to_string_lossy(),
        &std::fs::read_to_string(path).unwrap(),
    );
    ws.resolve_all();
    ws
}

/// Properties every drawing must have, whatever it is a drawing of.
fn check_shape(diagram: &Diagram, model: &Model, where_: &str) {
    let mut drawn = HashSet::new();
    for node in &diagram.nodes {
        assert!(
            drawn.insert(node.id),
            "{where_}: {:?} is drawn twice",
            node.id
        );
        match node.shape {
            // a box says what it stands for, and says the same thing the
            // model does
            Shape::Box => {
                let name = model.name(node.id).unwrap_or_default();
                assert!(!node.name.is_empty(), "{where_}: a box with no name");
                assert!(
                    node.name == name || node.name.starts_with(&format!("{name} :")),
                    "{where_}: box `{}` does not name {:?}",
                    node.name,
                    node.id
                );
            }
            // only the start marker is allowed to carry no label, and only
            // a succession can put one there
            Shape::Initial => {
                assert!(node.name.is_empty(), "{where_}: a marker with a name");
                assert_eq!(
                    model.kind(node.id),
                    ElementKind::SuccessionAsUsage,
                    "{where_}: a start marker that is not a succession"
                );
            }
        }
    }
    for edge in &diagram.edges {
        assert!(
            edge.from < diagram.nodes.len() && edge.to < diagram.nodes.len(),
            "{where_}: an edge leaves the diagram"
        );
        assert_ne!(edge.from, edge.to, "{where_}: an edge onto itself");
    }
}

/// The specializations the model holds between two drawn definitions --
/// worked out from the model rather than from the drawing code.
fn expected_specializations(model: &Model, drawn: &[ElementId]) -> Vec<(ElementId, ElementId)> {
    let mut out = Vec::new();
    for &subtype in drawn {
        for &relationship in model.owned(subtype) {
            if model.kind(relationship) != ElementKind::Subclassification {
                continue;
            }
            let Some(Value::Ref(supertype)) = model.get(relationship, "superclassifier") else {
                continue;
            };
            if drawn.contains(supertype) {
                out.push((subtype, *supertype));
            }
        }
    }
    out
}

#[test]
fn definition_diagrams_are_faithful_to_their_models() {
    let Some(root) = corpus() else { return };
    let mut files = Vec::new();
    sysml_files(&root, &mut files);
    files.sort();
    assert!(files.len() > 100, "the corpus looks truncated");

    for path in &files {
        let ws = loaded(path);
        let model = ws.model();
        let diagram = definition_diagram(model, &[ws.root()]);
        let where_ = path.file_name().unwrap().to_string_lossy().to_string();
        check_shape(&diagram, model, &where_);

        // every box is a named definition of this file, and nothing else is
        let drawn: Vec<ElementId> = diagram.nodes.iter().map(|node| node.id).collect();
        for &id in &drawn {
            assert!(
                model.kind(id).is_a(ElementKind::Definition),
                "{where_}: {:?} is not a definition",
                id
            );
        }
        let definitions: Vec<ElementId> = model
            .descendants(ws.root())
            .into_iter()
            .filter(|&id| model.kind(id).is_a(ElementKind::Definition) && model.name(id).is_some())
            .collect();
        assert_eq!(
            drawn, definitions,
            "{where_}: the boxes are not the model's"
        );

        // and the specializations drawn are exactly the ones it holds
        let expected = expected_specializations(model, &drawn);
        let actual: Vec<(ElementId, ElementId)> = diagram
            .edges
            .iter()
            .filter(|edge| edge.relation == Relation::Specialization)
            .map(|edge| (drawn[edge.from], drawn[edge.to]))
            .collect();
        assert_eq!(actual, expected, "{where_}: specializations do not match");
    }
}

#[test]
fn interconnection_diagrams_only_draw_what_is_in_scope() {
    let Some(root) = corpus() else { return };
    let mut files = Vec::new();
    sysml_files(&root, &mut files);
    files.sort();

    let mut drawn_any = 0usize;
    for path in &files {
        let ws = loaded(path);
        let model = ws.model();
        let owners: Vec<ElementId> = ws
            .named_elements()
            .map(|(id, _)| id)
            .filter(|&id| !model.owned(id).is_empty())
            .collect();
        for owner in owners {
            let diagram = interconnection_diagram(model, owner);
            if diagram.nodes.is_empty() {
                continue;
            }
            drawn_any += 1;
            let where_ = format!(
                "{}::{}",
                path.file_name().unwrap().to_string_lossy(),
                model.name(owner).unwrap_or_default()
            );
            check_shape(&diagram, model, &where_);

            // an internal view shows what the element itself holds
            for node in &diagram.nodes {
                assert!(
                    model.owned(owner).contains(&node.id),
                    "{where_}: `{}` is not a member of it",
                    node.name
                );
            }
        }
    }
    assert!(drawn_any > 50, "hardly anything was drawn: {drawn_any}");
}

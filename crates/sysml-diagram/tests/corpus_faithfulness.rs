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

/// The KerML half of the corpus, drawn through the same checks.
fn kerml_corpus() -> Option<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/sysml-v2-release/kerml/src");
    root.is_dir().then_some(root)
}

fn sysml_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sysml_files(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e == "sysml" || e == "kerml")
        {
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

/// The name an element answers to: its own, or -- for `part redefines
/// mcu : Atmega328p;`, which declares none -- that of what it redefines.
fn answers_to(model: &Model, element: ElementId) -> Option<String> {
    if let Some(name) = model.name(element) {
        return Some(name.to_string());
    }
    model.owned(element).iter().find_map(|&rel| {
        if model.kind(rel) != ElementKind::Redefinition {
            return None;
        }
        match model.get(rel, "redefinedFeature") {
            Some(sysml_model::Value::Ref(target)) => model.name(*target).map(str::to_string),
            _ => None,
        }
    })
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
            // a box says what it stands for, and says the same thing
            // the model does: the name first, then the type it was
            // declared with and how many of it there are, if either
            Shape::Box => {
                let name = answers_to(model, node.id).unwrap_or_default();
                assert!(!node.name.is_empty(), "{where_}: a box with no name");
                let said = node.name.strip_prefix(&name).is_some_and(|rest| {
                    rest.is_empty() || rest.starts_with(" :") || rest.starts_with('[')
                });
                assert!(
                    said,
                    "{where_}: box `{}` does not name {:?}",
                    node.name, node.id
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
            // the dot three or more ends meet at, named -- when it is
            // named at all -- by the connection itself
            Shape::ConnectionDot => {
                let name = answers_to(model, node.id).unwrap_or_default();
                assert_eq!(
                    node.name, name,
                    "{where_}: a connection dot that is not named by its connection"
                );
            }
            // a control node is drawn as its glyph rather than a box, and
            // only a control node is: the shape has to say what the model
            // says the element is
            Shape::Bar | Shape::Diamond | Shape::Cross => {
                let kind = model.kind(node.id);
                let expected = match node.shape {
                    Shape::Bar => [ElementKind::ForkNode, ElementKind::JoinNode],
                    Shape::Diamond => [ElementKind::MergeNode, ElementKind::DecisionNode],
                    _ => [
                        ElementKind::TerminateActionUsage,
                        ElementKind::TerminateActionUsage,
                    ],
                };
                assert!(
                    expected.contains(&kind),
                    "{where_}: {:?} drawn as {:?}",
                    kind,
                    node.shape
                );
            }
        }
    }
    for edge in &diagram.edges {
        assert!(
            edge.from < diagram.nodes.len() && edge.to < diagram.nodes.len(),
            "{where_}: an edge leaves the diagram"
        );
        // A membership may land back on the box it left -- `part
        // subparts : Assembly;` inside `Assembly` -- and only a
        // membership may: a specialization of itself, a connection to
        // itself or a transition to itself is a model saying nothing.
        if edge.from == edge.to {
            assert!(
                matches!(edge.relation, Relation::Composition | Relation::Reference),
                "{where_}: a {:?} onto itself",
                edge.relation
            );
        }
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
    if let Some(kerml) = kerml_corpus() {
        sysml_files(&kerml, &mut files);
    }
    files.sort();
    assert!(files.len() > 100, "the corpus looks truncated");

    for path in &files {
        let ws = loaded(path);
        let model = ws.model();
        let diagram = definition_diagram(model, &[ws.root()]);
        let where_ = path.file_name().unwrap().to_string_lossy().to_string();
        check_shape(&diagram, model, &where_);

        // every box is a named classifier of this file, and nothing else is
        let drawn: Vec<ElementId> = diagram.nodes.iter().map(|node| node.id).collect();
        for &id in &drawn {
            assert!(
                model.kind(id).is_a(ElementKind::Classifier),
                "{where_}: {:?} is not a classifier",
                id
            );
        }
        let definitions: Vec<ElementId> = model
            .descendants(ws.root())
            .into_iter()
            .filter(|&id| model.kind(id).is_a(ElementKind::Classifier) && model.name(id).is_some())
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

/// Whether `sub` reaches `sup` through the specializations the model
/// reified, so that an inherited member can be told from a stray one.
fn specializes(model: &Model, sub: ElementId, sup: ElementId) -> bool {
    let mut queue = vec![sub];
    let mut visited = std::collections::HashSet::new();
    while let Some(current) = queue.pop() {
        if !visited.insert(current) {
            continue;
        }
        if current == sup {
            return true;
        }
        for &rel in model.owned(current) {
            if model.kind(rel) != sysml_model::ElementKind::Subclassification {
                continue;
            }
            if let Some(sysml_model::Value::Ref(target)) = model.get(rel, "superclassifier") {
                queue.push(*target);
            }
        }
    }
    false
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

            // an internal view shows what the element is assembled
            // from: what it holds, and what it inherits from what it
            // specializes -- and nothing from anywhere else
            for node in &diagram.nodes {
                let holder = model.owner(node.id).expect("a drawn member has an owner");
                assert!(
                    holder == owner || specializes(model, owner, holder),
                    "{where_}: `{}` is neither its own nor inherited",
                    node.name
                );
            }
        }
    }
    assert!(drawn_any > 50, "hardly anything was drawn: {drawn_any}");
}

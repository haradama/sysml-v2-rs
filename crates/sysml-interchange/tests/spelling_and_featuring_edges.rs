//! Corners of the document writer that a text-built model rarely reaches:
//! what tells two unnamed top-level members apart, the ends a dependency
//! states itself, and a featuring that names no featuring type.

use sysml_interchange::{element_uuid, to_json};
use sysml_model::{ElementKind, Model, Value};

/// Two unnamed members of the root namespace that say the same thing but
/// for an enumeration literal, or for the element a membership reaches,
/// are told apart by what they say.
#[test]
fn unnamed_top_level_members_are_told_apart_by_an_enum_and_a_reference() {
    let mut model = Model::new();
    let root = model.create(ElementKind::Namespace);
    let target = model.create(ElementKind::Package);
    model.add_owned(root, target);
    model.set(target, "declaredName", Value::String("T".into()));

    let mut in_and_out = Vec::new();
    for direction in ["in", "out"] {
        let feature = model.create(ElementKind::Feature);
        model.add_owned(root, feature);
        model.set(feature, "direction", Value::EnumLit(direction));
        in_and_out.push(feature);
    }
    let membership = model.create(ElementKind::Membership);
    model.add_owned(root, membership);
    model.set(membership, "memberElement", Value::Ref(target));

    let uuids: Vec<_> = in_and_out
        .iter()
        .map(|&feature| element_uuid(&model, feature))
        .collect();
    assert_ne!(uuids[0], uuids[1]);
    // each is the first of its own kind: neither is counted among the other
    assert_eq!(uuids[0], element_uuid(&model, in_and_out[0]));
    let reaching = element_uuid(&model, membership);
    assert!(!uuids.contains(&reaching));
}

/// A dependency written inside a type states the elements it relates
/// itself, as its `client`s, so it is a statement of its own there.
#[test]
fn a_dependency_in_a_type_is_a_statement_of_its_own() {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file(
        "dep.sysml",
        "part def A { part x; part y; dependency d from x to y; }\n",
    );
    ws.resolve_all();
    let json = to_json(ws.model());
    let objects = json.as_array().unwrap();
    let dependency = objects
        .iter()
        .find(|it| it["@type"] == "Dependency")
        .expect("the dependency is written");
    assert_eq!(dependency["client"].as_array().unwrap().len(), 1);
    assert_eq!(dependency["source"], dependency["client"]);
}

/// A `TypeFeaturing` that names no featuring type contributes no featuring
/// to the feature it stands under, and is still written.
#[test]
fn a_featuring_without_a_featuring_type_is_written_alone() {
    let mut model = Model::new();
    let owner = model.create(ElementKind::Namespace);
    let feature = model.create(ElementKind::Feature);
    model.add_owned(owner, feature);
    let featuring = model.create(ElementKind::TypeFeaturing);
    model.add_owned(feature, featuring);

    let json = to_json(&model);
    let objects = json.as_array().unwrap();
    assert!(objects.iter().any(|it| it["@type"] == "TypeFeaturing"));
}

/// A relationship a type owns that states its source as a list is
/// written with that list's elements as the source it states itself.
#[test]
fn a_source_stated_as_a_list_is_a_statement_of_its_own() {
    let mut model = Model::new();
    let root = model.create(ElementKind::Namespace);
    let owner = model.create(ElementKind::Classifier);
    let other = model.create(ElementKind::Classifier);
    model.add_owned(root, owner);
    model.add_owned(root, other);
    let specialization = model.create(ElementKind::Specialization);
    model.add_owned(owner, specialization);
    model.set(specialization, "source", Value::RefList(vec![other]));

    let json = to_json(&model);
    let objects = json.as_array().unwrap();
    assert!(objects.iter().any(|it| it["@type"] == "Specialization"));
}

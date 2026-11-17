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

/// The object written for `id`.
fn object_of<'a>(
    json: &'a serde_json::Value,
    model: &Model,
    id: sysml_model::ElementId,
) -> &'a serde_json::Value {
    let uuid = element_uuid(model, id).to_string();
    json.as_array()
        .unwrap()
        .iter()
        .find(|it| it["@id"] == uuid.as_str())
        .expect("written")
}

fn reference(model: &Model, id: sysml_model::ElementId) -> serde_json::Value {
    serde_json::json!({ "@id": element_uuid(model, id).to_string() })
}

/// A `TypeFeaturing` that names no `featureOfType` features the feature
/// that owns it.
#[test]
fn a_featuring_without_a_feature_features_its_owner() {
    let mut model = Model::new();
    let root = model.create(ElementKind::Namespace);
    let ty = model.create(ElementKind::Classifier);
    model.add_owned(root, ty);
    let feature = model.create(ElementKind::Feature);
    model.add_owned(root, feature);
    let featuring = model.create(ElementKind::TypeFeaturing);
    model.add_owned(feature, featuring);
    model.set(featuring, "featuringType", Value::Ref(ty));

    let json = to_json(&model);
    assert_eq!(
        object_of(&json, &model, feature)["featuringType"],
        serde_json::json!([reference(&model, ty)])
    );
}

/// A chain of one written as a lone reference rather than a list is
/// featured as that one feature is.
#[test]
fn a_chain_of_one_reference_is_featured_as_its_feature() {
    let mut model = Model::new();
    let root = model.create(ElementKind::Namespace);
    let ty = model.create(ElementKind::Classifier);
    model.add_owned(root, ty);
    let first = model.create(ElementKind::Feature);
    model.add_owned(ty, first);
    let chain = model.create(ElementKind::Feature);
    model.add_owned(root, chain);
    model.set(chain, "chainingFeature", Value::Ref(first));

    let json = to_json(&model);
    assert_eq!(
        object_of(&json, &model, chain)["featuringType"],
        serde_json::json!([reference(&model, ty)])
    );
}

/// An annotation that owns no annotating element and is owned by none is
/// about what owns it, and names no annotating element.
#[test]
fn an_annotation_with_no_comment_is_about_its_owner() {
    let mut model = Model::new();
    let root = model.create(ElementKind::Namespace);
    let package = model.create(ElementKind::Package);
    model.add_owned(root, package);
    let annotation = model.create(ElementKind::Annotation);
    model.add_owned(package, annotation);

    let json = to_json(&model);
    let written = object_of(&json, &model, annotation);
    assert_eq!(
        written["annotatedElement"],
        reference(&model, package),
        "{written}"
    );
    assert_eq!(
        written["annotatingElement"],
        serde_json::Value::Null,
        "{written}"
    );
}

/// A membership import whose imported end is an element rather than a
/// membership imports that element.
#[test]
fn an_import_of_an_element_imports_the_element() {
    let mut model = Model::new();
    let root = model.create(ElementKind::Namespace);
    let package = model.create(ElementKind::Package);
    model.add_owned(root, package);
    let import = model.create(ElementKind::MembershipImport);
    model.add_owned(package, import);
    let imported = model.create(ElementKind::Package);
    model.add_owned(import, imported);

    let json = to_json(&model);
    let written = object_of(&json, &model, import);
    assert_eq!(
        written["importedElement"],
        reference(&model, imported),
        "{written}"
    );
}

/// A relationship owned by an element that is no namespace is not a
/// member of anything, and is written as a part of what owns it.
#[test]
fn a_relationship_under_no_namespace_belongs_to_its_owner() {
    let mut model = Model::new();
    let root = model.create(ElementKind::Namespace);
    let package = model.create(ElementKind::Package);
    model.add_owned(root, package);
    let comment = model.create(ElementKind::Comment);
    model.add_owned(package, comment);
    let relationship = model.create(ElementKind::Subclassification);
    model.add_owned(comment, relationship);

    let json = to_json(&model);
    let written = object_of(&json, &model, relationship);
    assert_eq!(
        written["owningRelatedElement"],
        reference(&model, comment),
        "{written}"
    );
}

//! Standard JSON interchange for [`sysml_model::Model`].
//!
//! Follows the serialization style of the SysML v2 API & Services standard:
//! every element is a JSON object with `"@type"` (metaclass name), `"@id"`
//! (UUID) and its properties, where element references are `{"@id": ...}`
//! objects. Element UUIDs are deterministic (UUIDv5 over the element's
//! ownership path), so exporting the same model twice yields identical JSON.
//!
//! Deliberate simplifications versus the full standard: ownership is
//! emitted as `owner` / `ownedElement` directly rather than through reified
//! `owningMembership` chains, and derived properties are not synthesized.

use std::collections::HashMap;

use serde_json::{json, Map, Value as Json};
use sysml_model::{ElementId, ElementKind, FeatureType, Model, PrimitiveType, Value};
use uuid::Uuid;

/// Errors produced when reading interchange JSON.
#[derive(Debug)]
pub enum ImportError {
    NotAnArray,
    MissingType(usize),
    UnknownType(String),
    MissingId(usize),
    UnknownReference(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::NotAnArray => write!(f, "expected a JSON array of elements"),
            ImportError::MissingType(i) => write!(f, "element {i} has no \"@type\""),
            ImportError::UnknownType(t) => write!(f, "unknown metaclass {t:?}"),
            ImportError::MissingId(i) => write!(f, "element {i} has no \"@id\""),
            ImportError::UnknownReference(id) => write!(f, "reference to unknown element {id:?}"),
        }
    }
}

impl std::error::Error for ImportError {}

/// Deterministic UUID for an element: v5 over its ownership path. Each
/// segment includes the sibling index (so same-named siblings stay unique)
/// plus the member name where available.
pub fn element_uuid(model: &Model, id: ElementId) -> Uuid {
    let mut segments = Vec::new();
    let mut current = Some(id);
    while let Some(elem) = current {
        let index = model
            .owner(elem)
            .map(|o| {
                model
                    .owned(o)
                    .iter()
                    .position(|c| *c == elem)
                    .unwrap_or_default()
            })
            .unwrap_or(elem.index());
        let segment = match model.name(elem) {
            Some(name) => format!("{index}:{name}"),
            None => format!("{index}"),
        };
        segments.push(segment);
        current = model.owner(elem);
    }
    segments.reverse();
    let path = format!("sysml-v2-rs:{}", segments.join("/"));
    Uuid::new_v5(&Uuid::NAMESPACE_URL, path.as_bytes())
}

/// Serialize the whole model as an array of element objects (stable order).
pub fn to_json(model: &Model) -> Json {
    let uuids: HashMap<ElementId, Uuid> = model
        .ids()
        .map(|id| (id, element_uuid(model, id)))
        .collect();
    let reference = |id: &ElementId| json!({ "@id": uuids[id].to_string() });

    let elements: Vec<Json> = model
        .ids()
        .map(|id| {
            let mut object = Map::new();
            object.insert("@type".into(), model.kind(id).name().into());
            object.insert("@id".into(), uuids[&id].to_string().into());
            for (name, value) in model.props(id) {
                let value = match value {
                    Value::Bool(b) => Json::from(*b),
                    Value::Int(i) => Json::from(*i),
                    Value::Real(r) => Json::from(*r),
                    Value::String(s) => Json::from(s.clone()),
                    Value::EnumLit(l) => Json::from(*l),
                    Value::Ref(r) => reference(r),
                    Value::RefList(rs) => Json::Array(rs.iter().map(reference).collect()),
                };
                object.insert(name.into(), value);
            }
            object.insert(
                "owner".into(),
                match model.owner(id) {
                    Some(owner) => reference(&owner),
                    None => Json::Null,
                },
            );
            object.insert(
                "ownedElement".into(),
                Json::Array(model.owned(id).iter().map(reference).collect()),
            );
            Json::Object(object)
        })
        .collect();
    Json::Array(elements)
}

/// Rebuild a model from interchange JSON. Returns the model and the root
/// elements (those without an owner). Unknown properties are ignored;
/// unknown metaclasses and dangling references are errors.
pub fn from_json(json: &Json) -> Result<(Model, Vec<ElementId>), ImportError> {
    let array = json.as_array().ok_or(ImportError::NotAnArray)?;
    let mut model = Model::new();
    let mut by_uuid: HashMap<&str, ElementId> = HashMap::new();

    // pass 1: create all elements
    let mut ids = Vec::with_capacity(array.len());
    for (index, object) in array.iter().enumerate() {
        let type_name = object["@type"]
            .as_str()
            .ok_or(ImportError::MissingType(index))?;
        let kind = ElementKind::from_name(type_name)
            .ok_or_else(|| ImportError::UnknownType(type_name.to_string()))?;
        let uuid = object["@id"]
            .as_str()
            .ok_or(ImportError::MissingId(index))?;
        let id = model.create(kind);
        by_uuid.insert(uuid, id);
        ids.push(id);
    }

    let resolve = |value: &Json| -> Result<ElementId, ImportError> {
        let uuid = value["@id"].as_str().unwrap_or_default();
        by_uuid
            .get(uuid)
            .copied()
            .ok_or_else(|| ImportError::UnknownReference(uuid.to_string()))
    };

    // pass 2: properties and ownership
    for (object, id) in array.iter().zip(&ids) {
        let kind = model.kind(*id);
        let object = object.as_object().expect("validated in pass 1");
        for (key, value) in object {
            match key.as_str() {
                "@type" | "@id" | "owner" => continue,
                "ownedElement" => {
                    for child in value.as_array().into_iter().flatten() {
                        let child = resolve(child)?;
                        model.add_owned(*id, child);
                    }
                }
                _ => {
                    let Some(meta) = kind.feature(key) else {
                        continue; // tolerate foreign properties
                    };
                    let converted = convert_value(meta.ty, value, &resolve)?;
                    if let Some(converted) = converted {
                        model.set(*id, key, converted);
                    }
                }
            }
        }
    }

    let roots = ids
        .iter()
        .copied()
        .filter(|id| model.owner(*id).is_none())
        .collect();
    Ok((model, roots))
}

fn convert_value(
    ty: FeatureType,
    value: &Json,
    resolve: &dyn Fn(&Json) -> Result<ElementId, ImportError>,
) -> Result<Option<Value>, ImportError> {
    let converted = match (ty, value) {
        (_, Json::Null) => None,
        (FeatureType::Data(PrimitiveType::Boolean), Json::Bool(b)) => Some(Value::Bool(*b)),
        (FeatureType::Data(PrimitiveType::Real), Json::Number(n)) => n.as_f64().map(Value::Real),
        (FeatureType::Data(_), Json::Number(n)) => n
            .as_i64()
            .map(Value::Int)
            .or_else(|| n.as_f64().map(Value::Real)),
        (FeatureType::Data(_), Json::String(s)) => Some(Value::String(s.clone())),
        (FeatureType::Enumeration(_), Json::String(s)) => Some(Value::String(s.clone())),
        (FeatureType::Class(_), Json::Object(_)) => Some(Value::Ref(resolve(value)?)),
        (FeatureType::Class(_), Json::Array(items)) => {
            let refs: Result<Vec<_>, _> = items.iter().map(resolve).collect();
            Some(Value::RefList(refs?))
        }
        _ => None,
    };
    Ok(converted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_model::build_model;

    fn example_model() -> Model {
        let parse = sysml_syntax::parse(
            "package P {\n  doc /* docs */\n  abstract part def Vehicle {\n    attribute mass : Real = 10.0;\n  }\n  part car : Vehicle;\n}",
        );
        assert!(parse.ok());
        build_model(&parse).0
    }

    #[test]
    fn export_is_deterministic_and_typed() {
        let model = example_model();
        let a = to_json(&model);
        let b = to_json(&model);
        assert_eq!(a, b);
        let first = &a.as_array().unwrap()[0];
        assert_eq!(first["@type"], "Package");
        assert_eq!(first["declaredName"], "P");
        assert!(first["@id"].as_str().unwrap().len() == 36);
    }

    #[test]
    fn round_trip_preserves_structure() {
        let model = example_model();
        let json = to_json(&model);
        let (rebuilt, roots) = from_json(&json).unwrap();

        assert_eq!(rebuilt.len(), model.len());
        assert_eq!(roots.len(), 1);
        let pkg = roots[0];
        assert_eq!(rebuilt.kind(pkg), ElementKind::Package);
        assert_eq!(rebuilt.name(pkg), Some("P"));
        let members = rebuilt.owned(pkg);
        assert_eq!(rebuilt.kind(members[0]), ElementKind::Documentation);
        let vehicle = members[1];
        assert_eq!(rebuilt.kind(vehicle), ElementKind::PartDefinition);
        assert_eq!(rebuilt.get(vehicle, "isAbstract"), Some(&Value::Bool(true)));
        assert_eq!(rebuilt.name(rebuilt.owned(vehicle)[0]), Some("mass"));

        // and the re-export matches the first export exactly
        assert_eq!(to_json(&rebuilt), json);
    }

    #[test]
    fn import_errors_display() {
        for (error, needle) in [
            (ImportError::NotAnArray, "array"),
            (ImportError::MissingType(3), "no \"@type\""),
            (ImportError::UnknownType("X".into()), "unknown metaclass"),
            (ImportError::MissingId(1), "no \"@id\""),
            (ImportError::UnknownReference("u".into()), "unknown element"),
        ] {
            assert!(error.to_string().contains(needle), "{error}");
            assert!(!format!("{error:?}").is_empty());
        }
    }

    #[test]
    fn value_variants_and_unnamed_roots_round_trip() {
        use sysml_model::ElementKind;
        let mut model = Model::new();
        // unnamed root element: uuid falls back to the arena index
        let root = model.create(ElementKind::Package);
        let a = model.create(ElementKind::LiteralInteger);
        let b = model.create(ElementKind::LiteralRational);
        let m = model.create(ElementKind::MembershipImport);
        model.add_owned(root, a);
        model.add_owned(root, b);
        model.add_owned(root, m);
        model.set(a, "value", Value::Int(42));
        model.set(b, "value", Value::Real(2.5));
        model.set(m, "visibility", Value::EnumLit("private"));
        model.set(m, "isImportAll", Value::Bool(true));
        model.set(m, "importedMembership", Value::Ref(a));
        model.set(root, "filterCondition", Value::RefList(vec![a, b]));

        let json = to_json(&model);
        let (rebuilt, roots) = from_json(&json).unwrap();
        assert_eq!(roots.len(), 1);
        let ra = rebuilt.owned(roots[0])[0];
        let rb = rebuilt.owned(roots[0])[1];
        let rm = rebuilt.owned(roots[0])[2];
        assert_eq!(rebuilt.get(ra, "value"), Some(&Value::Int(42)));
        assert_eq!(rebuilt.get(rb, "value"), Some(&Value::Real(2.5)));
        // enum literals come back as strings
        assert_eq!(
            rebuilt.get(rm, "visibility").and_then(Value::as_str),
            Some("private")
        );
        assert_eq!(rebuilt.get(rm, "isImportAll"), Some(&Value::Bool(true)));
        assert_eq!(rebuilt.get(rm, "importedMembership"), Some(&Value::Ref(ra)));
        assert_eq!(
            rebuilt.get(roots[0], "filterCondition"),
            Some(&Value::RefList(vec![ra, rb]))
        );
        assert_eq!(to_json(&rebuilt), json);
    }

    #[test]
    fn tolerates_nulls_and_foreign_properties() {
        let json = serde_json::json!([
            { "@type": "Package", "@id": "00000000-0000-0000-0000-000000000001",
              "declaredName": null,
              "someToolSpecificThing": 5,
              "isImpliedIncluded": [1, 2] }
        ]);
        let (model, roots) = from_json(&json).unwrap();
        assert_eq!(model.props(roots[0]).count(), 0);
    }

    #[test]
    fn rejects_unknown_types_and_dangling_refs() {
        let bad = serde_json::json!([{ "@type": "NotAClass", "@id": "x" }]);
        assert!(matches!(from_json(&bad), Err(ImportError::UnknownType(_))));
        let dangling = serde_json::json!([
            { "@type": "Package", "@id": "11111111-1111-1111-1111-111111111111",
              "ownedElement": [{ "@id": "22222222-2222-2222-2222-222222222222" }] }
        ]);
        assert!(matches!(
            from_json(&dangling),
            Err(ImportError::UnknownReference(_))
        ));
    }
}

//! The fixture crate's rustdoc JSON, all the way through: generated SysML
//! that parses, resolves against a scalar library, and hands its `@rust`
//! bindings back out of the resolved model -- what a code generator will
//! do with it.

use sysml_model::{ElementId, ElementKind, Model, Value};

fn fixture() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/inventory_store.rustdoc.json");
    std::fs::read_to_string(path).expect("the fixture JSON is checked in")
}

/// The scalar types the generated package imports, enough to resolve.
const SCALARS: &str = "package ScalarValues {\n\
    \tabstract datatype Boolean;\n\
    \tabstract datatype String;\n\
    \tabstract datatype Real;\n\
    \tabstract datatype Integer;\n\
    \tabstract datatype Natural;\n\
}\n";

#[test]
fn the_generated_package_is_deterministic_and_names_the_api() {
    let first = sysml_import_api::rustdoc_to_sysml(&fixture(), None).unwrap();
    let second = sysml_import_api::rustdoc_to_sysml(&fixture(), None).unwrap();
    assert_eq!(first, second, "two runs must write identical bytes");

    assert!(first.starts_with("// generated from crate `inventory_store`"));
    assert!(first.contains("package InventoryStoreApi {"));
    // structs, fields and containers
    assert!(first.contains("item def StockQuery {"));
    assert!(first.contains("attribute warehouses : Natural[*];"));
    assert!(first.contains("attribute note : String[0..1];"));
    assert!(first.contains("attribute fill_ratio : Real;"));
    // keyword collisions are quoted
    assert!(first.contains("attribute 'filter' : Boolean;"));
    assert!(first.contains("attribute 'message' : String;"));
    // enums
    assert!(first.contains("enum def Region {"));
    assert!(first.contains("\t\tenum Tokyo;"));
    // the trait, its methods, and their signatures
    assert!(first.contains("port def InventoryStore {"));
    assert!(first.contains("action def GetStock {"));
    assert!(first.contains("in query : StockQuery;"));
    assert!(first.contains("out result : StockLevel;"));
    assert!(first.contains("out error : StoreError[0..1];"));
    assert!(first.contains(":>> isAsync = true;"), "watch is async");
    assert!(first.contains(":>> takesSelf = \"&mut self\";"), "refresh");
    // free functions come along, generic ones are listed instead
    assert!(first.contains("action def DefaultRegion {"));
    assert!(first.contains("//   pick -- unsupported signature"));
    // shapes with no monomorphic SysML form are listed, not dropped
    assert!(first.contains("//   Pair -- not a plain struct"));
    assert!(first.contains("//   Event -- not a plain enum"));
    assert!(first.contains("//   Session::tally -- unsupported signature"));
    // a consumed receiver, signed and character scalars, boxed recursion
    assert!(first.contains(":>> takesSelf = \"self\";"));
    assert!(first.contains("attribute delta : Integer;"));
    assert!(first.contains("attribute grade : String;"));
    assert!(first.contains("attribute cause : Adjustment[0..1];"));
    assert!(first.contains("//   Adjustment.bounds -- unmappable type"));
    assert!(first.contains("//   Session::dump -- unsupported signature"));
    assert!(first.contains("//   Session::fetch -- unsupported signature"));
    assert!(first.contains("in name : String;"), "&str reads as String");
    // `*/` inside a doc cannot end the block early
    assert!(first.contains("*\\/ inside."));
    // docs travel
    assert!(first.contains("doc /* What to look up. */"));

    // a chosen package name wins over the derived one
    let named = sysml_import_api::rustdoc_to_sysml(&fixture(), Some("Warehouse")).unwrap();
    assert!(named.contains("package Warehouse {"));
}

#[test]
fn the_generated_package_parses_resolves_and_binds() {
    let sysml = sysml_import_api::rustdoc_to_sysml(&fixture(), None).unwrap();
    let parse = sysml_syntax::parse(&sysml);
    assert!(
        parse.ok(),
        "the generated package must parse: {:?}",
        parse.errors().first()
    );

    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file("scalars.kerml", SCALARS);
    ws.add_file("api.sysml", &sysml);
    let stats = ws.resolve_all();
    assert_eq!(
        stats.unresolved, 0,
        "every reference in the generated package must resolve"
    );

    // what a code generator does: find GetStock, read its binding
    let model = ws.model();
    let get_stock = model
        .ids()
        .find(|&id| {
            model.name(id) == Some("GetStock") && model.kind(id) == ElementKind::ActionDefinition
        })
        .expect("GetStock is in the model");
    let binding = bindings_of(model, get_stock);
    assert_eq!(
        binding.get("path").map(String::as_str),
        Some("inventory_store::InventoryStore::get_stock")
    );
    assert_eq!(binding.get("takesSelf").map(String::as_str), Some("&self"));
    assert_eq!(binding.get("isAsync").map(String::as_str), Some("false"));
    assert_eq!(binding.get("isFallible").map(String::as_str), Some("true"));

    // and the directions a signature generator needs
    let directions: Vec<(String, String)> = model
        .owned(get_stock)
        .iter()
        .filter_map(|&child| {
            let direction = model.get(child, "direction")?.as_str()?;
            Some((model.name(child)?.to_string(), direction.to_string()))
        })
        .collect();
    assert_eq!(
        directions,
        [
            ("query".to_string(), "in".to_string()),
            ("result".to_string(), "out".to_string()),
            ("error".to_string(), "out".to_string()),
        ]
    );

    // the quoted field kept its plain name
    assert!(model.ids().any(
        |id| model.name(id) == Some("filter") && model.kind(id) == ElementKind::AttributeUsage
    ));
}

/// The `@rust { :>> name = value; ... }` pairs of one element, read the
/// way a code generator reads them: each value names the metadata
/// attribute its reified redefinition points at.
fn bindings_of(model: &Model, element: ElementId) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for &child in model.owned(element) {
        if model.kind(child) != ElementKind::MetadataUsage {
            continue;
        }
        for &setting in model.owned(child) {
            let Some(redefined) = model.owned(setting).iter().copied().find_map(|rel| {
                if model.kind(rel) != ElementKind::Redefinition {
                    return None;
                }
                match model.get(rel, "redefinedFeature") {
                    Some(Value::Ref(target)) => Some(*target),
                    _ => None,
                }
            }) else {
                continue;
            };
            let Some(name) = model.name(redefined) else {
                continue;
            };
            let Some(value) = model.owned(setting).iter().copied().find_map(|part| {
                if model.kind(part) != ElementKind::FeatureValue {
                    return None;
                }
                match model.get(part, "value") {
                    Some(Value::Ref(literal)) => Some(*literal),
                    _ => None,
                }
            }) else {
                continue;
            };
            let rendered = match model.get(value, "value") {
                Some(Value::String(text)) => text.clone(),
                Some(Value::Bool(flag)) => flag.to_string(),
                Some(Value::Int(int)) => int.to_string(),
                other => format!("{other:?}"),
            };
            out.insert(name.to_string(), rendered);
        }
    }
    out
}

#[test]
fn a_clean_crate_has_no_skip_list() {
    // string-keyed ids, nothing to skip: the trailing comment block must
    // not appear at all
    let json = r#"{
        "root": "r",
        "index": {
            "r": { "name": "tiny", "inner": { "module": { "items": ["s"] } } },
            "s": { "name": "Only", "inner": { "struct": { "kind": { "plain": { "fields": ["f"] } } } } },
            "f": { "name": "flag", "inner": { "struct_field": { "primitive": "bool" } } }
        }
    }"#;
    let sysml = sysml_import_api::rustdoc_to_sysml(json, None).unwrap();
    assert!(sysml.contains("package TinyApi {"));
    assert!(sysml.contains("attribute flag : Boolean;"));
    assert!(!sysml.contains("not imported"));
}

#[test]
fn odd_shapes_are_skipped_not_dropped() {
    // ids that resolve to nothing, items with no name or body, kinds and
    // types this importer has no shape for, and a one-armed Result
    let json = r#"{
        "root": "r",
        "index": {
            "r": { "name": "odd", "inner": { "module": { "items": ["missing", "nameless", "bodyless", "c", "s", "half", "anon", "ghostly", "sealed", "hollow"] } } },
            "nameless": { "inner": { "module": {} } },
            "bodyless": { "name": "husk" },
            "c": { "name": "LIMIT", "inner": { "constant": {} } },
            "s": { "name": "Strange", "inner": { "struct": { "kind": { "plain": { "fields": ["f"] } } } } },
            "f": { "name": "off", "inner": { "struct_field": { "primitive": "never" } } },
            "half": { "name": "half_result", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [], "output": { "resolved_path": { "path": "Result",
                    "args": { "angle_bracketed": { "args": [ { "type": { "primitive": "bool" } } ] } } } } },
                "header": { "is_async": false } } } },
            "anon": { "name": "anon_param", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [ [ null ] ], "output": null },
                "header": { "is_async": false } } } },
            "ghostly": { "name": "Ghostly", "inner": { "struct": { "kind": { "plain": { "fields": ["nowhere", "veiled"] } } } } },
            "veiled": { "inner": { "struct_field": { "primitive": "bool" } } },
            "sealed": { "name": "Sealed", "inner": { "trait": {} } },
            "hollow": { "name": "Hollow", "inner": { "trait": { "items": ["nowhere", "unnamed"] } } },
            "unnamed": { "inner": { "function": {} } }
        }
    }"#;
    let sysml = sysml_import_api::rustdoc_to_sysml(json, None).unwrap();
    assert!(sysml.contains("//   LIMIT -- constant"));
    assert!(sysml.contains("//   Strange.off -- unmappable type"));
    assert!(sysml.contains("//   half_result -- unsupported signature"));
    // an input with no name contributes nothing but does not crash
    assert!(sysml.contains("action def AnonParam {"));
    // a field or method whose id resolves to nothing is simply absent,
    // and a trait without a body still gets its port
    assert!(sysml.contains("item def Ghostly {"));
    assert!(sysml.contains("port def Sealed {"));
    assert!(sysml.contains("port def Hollow {"));
}

#[test]
fn broken_inputs_are_named_errors() {
    use sysml_import_api::ImportError;
    assert!(matches!(
        sysml_import_api::rustdoc_to_sysml("not json", None),
        Err(ImportError::NotJson(_))
    ));
    assert_eq!(
        sysml_import_api::rustdoc_to_sysml("{}", None),
        Err(ImportError::Malformed("its item index".to_string()))
    );
    assert_eq!(
        sysml_import_api::rustdoc_to_sysml(r#"{"index":{}}"#, None),
        Err(ImportError::Malformed("its root module".to_string()))
    );
    assert_eq!(
        sysml_import_api::rustdoc_to_sysml(r#"{"index":{},"root":0}"#, None),
        Err(ImportError::Malformed("the root module item".to_string()))
    );
    for error in [
        ImportError::NotJson("eof".to_string()),
        ImportError::Malformed("its item index".to_string()),
    ] {
        assert!(!error.to_string().is_empty());
    }
}

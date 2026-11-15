//! The fixture crate's rustdoc JSON, all the way through: generated SysML
//! that parses, resolves against a scalar library, and hands its `@code`
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
    let first = sysml_rust::rustdoc_to_sysml(&fixture(), None)
        .unwrap()
        .sysml;
    let second = sysml_rust::rustdoc_to_sysml(&fixture(), None)
        .unwrap()
        .sysml;
    assert_eq!(first, second, "two runs must write identical bytes");

    assert!(first.starts_with("// generated from crate `inventory_store`"));
    assert!(first.contains("package InventoryStoreApi {"));
    // structs, fields and containers
    assert!(first.contains("item def StockQuery {"));
    assert!(first.contains("attribute warehouses : RustU32[*];"));
    assert!(first.contains("attribute note : String[0..1];"));
    assert!(first.contains("attribute fill_ratio : Real;"));
    // keyword collisions are quoted
    assert!(first.contains("attribute 'filter' : Boolean"));
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
    assert!(first.contains("attribute grade : RustChar;"));
    // a field holding a struct is an item of it: an attribute is a value,
    // and may not own an occurrence
    assert!(first.contains("\t\titem cause : Adjustment[0..1];"));
    assert!(first.contains("//   Adjustment.bounds -- unmappable type"));
    assert!(first.contains("//   Session::dump -- unsupported signature"));
    assert!(first.contains("//   Session::fetch -- unsupported signature"));
    assert!(first.contains("in name : RustStr;"), "&str is its own type");
    // a Rust scalar no SysML one is the width of is declared here, so
    // that what comes back out of the generator is the type the crate
    // takes; the five that do round trip are left as they are
    assert!(first.contains(
        "attribute def RustU32 :> Natural { @code { :>> writtenIn = \"rust\"; :>> path = \"u32\"; \
         :>> capabilities = \"Debug, Clone, PartialEq, Default\"; } }"
    ));
    assert!(first.contains("attribute def RustStr :> String"));
    assert!(
        !first.contains("RustI128"),
        "a width nothing uses is not declared"
    );
    assert!(first.contains("attribute quantity : Natural;"));
    assert!(first.contains("attribute delta : Integer;"));
    // `*/` inside a doc cannot end the block early
    assert!(first.contains("*\\/ inside."));
    // docs travel
    assert!(first.contains("doc /* What to look up. */"));

    // a chosen package name wins over the derived one
    let named = sysml_rust::rustdoc_to_sysml(&fixture(), Some("Warehouse"))
        .unwrap()
        .sysml;
    assert!(named.contains("package Warehouse {"));
}

#[test]
fn the_generated_package_parses_resolves_and_binds() {
    let sysml = sysml_rust::rustdoc_to_sysml(&fixture(), None)
        .unwrap()
        .sysml;
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
            let direction = model.maybe(child, "direction")?.as_str()?;
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

/// The `@code { :>> writtenIn = \"rust\"; :>> name = value; ... }` pairs of one element, read the
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
                match model.maybe(rel, "redefinedFeature") {
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
                match model.maybe(part, "value") {
                    Some(Value::Ref(literal)) => Some(*literal),
                    _ => None,
                }
            }) else {
                continue;
            };
            let rendered = match model.maybe(value, "value") {
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
    let sysml = sysml_rust::rustdoc_to_sysml(json, None).unwrap().sysml;
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
    let sysml = sysml_rust::rustdoc_to_sysml(json, None).unwrap().sysml;
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

/// What a model cannot perform is not imported as though it could be:
/// an `unsafe fn`, and a trait's function with no receiver. Imported as
/// ordinary actions, both generated calls -- `self.api.danger()`,
/// `self.api.stat()` -- that did not compile.
#[test]
fn what_cannot_be_called_from_a_model_is_said_and_not_imported() {
    let json = r#"{
        "root": "r",
        "index": {
            "r": { "name": "careful", "inner": { "module": { "items": ["t", "loose"] } } },
            "t": { "name": "Api", "inner": { "trait": { "items": ["stat", "danger", "fine"] } } },
            "stat": { "name": "stat", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [], "output": { "primitive": "u32" } },
                "header": { "is_async": false, "is_unsafe": false } } } },
            "danger": { "name": "danger", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [ [ "self", { "borrowed_ref": { "is_mutable": false } } ] ], "output": { "primitive": "u32" } },
                "header": { "is_async": false, "is_unsafe": true } } } },
            "fine": { "name": "fine", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [ [ "self", { "borrowed_ref": { "is_mutable": false } } ] ], "output": { "primitive": "u32" } },
                "header": { "is_async": false, "is_unsafe": false } } } },
            "loose": { "name": "loose", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [], "output": { "primitive": "u32" } },
                "header": { "is_async": false, "is_unsafe": true } } } }
        }
    }"#;
    let sysml = sysml_rust::rustdoc_to_sysml(json, None).unwrap().sysml;
    assert!(
        sysml.contains("action def ApiFine {") || sysml.contains("action def Fine {"),
        "{sysml}"
    );
    assert!(
        !sysml.contains("action def ApiStat") && !sysml.contains("action def Stat "),
        "{sysml}"
    );
    assert!(!sysml.contains("Danger {"), "{sysml}");
    assert!(!sysml.contains("action def Loose"), "{sysml}");
    assert!(
        sysml.contains("//   Api::stat -- takes no receiver"),
        "{sysml}"
    );
    assert!(sysml.contains("//   Api::danger -- unsafe"), "{sysml}");
    assert!(sysml.contains("//   loose -- unsafe"), "{sysml}");
}

/// A crate that keeps its types in modules and re-exports them is the
/// normal shape of a Rust library, and it used to import as almost
/// nothing: only what `lib.rs` spelled out was seen, while the
/// signatures naming the rest were written anyway, leaving the package
/// full of names it never declared.
#[test]
fn a_re_exported_type_is_imported_and_can_be_referred_to() {
    let json = r#"{
        "root": "r",
        "index": {
            "r": { "name": "layered", "inner": { "module": { "items": ["use_shape", "use_glob", "use_gone", "use_idless", "fun"] } } },
            "use_shape": { "name": "Shape", "inner": { "use": { "source": "inner::Shape", "name": "Shape", "id": "shape", "is_glob": false } } },
            "use_glob": { "name": "*", "inner": { "use": { "source": "inner", "name": "*", "id": "shape", "is_glob": true } } },
            "use_gone": { "name": "Gone", "inner": { "use": { "source": "inner::Gone", "name": "Gone", "id": "nowhere", "is_glob": false } } },
            "use_idless": { "name": "Idless", "inner": { "use": { "source": "elsewhere::Idless", "name": "Idless", "is_glob": false } } },
            "shape": { "name": "Shape", "inner": { "struct": { "kind": { "plain": { "fields": ["w"] } } } } },
            "w": { "name": "width", "inner": { "struct_field": { "primitive": "f64" } } },
            "fun": { "name": "area", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [ [ "shape", { "resolved_path": { "path": "Shape", "id": "shape" } } ] ],
                         "output": { "primitive": "f64" } },
                "header": { "is_async": false } } } }
        }
    }"#;
    let sysml = sysml_rust::rustdoc_to_sysml(json, None).unwrap().sysml;
    assert!(sysml.contains("item def Shape {"), "{sysml}");
    assert!(sysml.contains("in shape : Shape;"), "{sysml}");
    // the glob re-export names the same struct and must not double it
    assert_eq!(sysml.matches("item def Shape {").count(), 1, "{sysml}");
    // a re-export of something outside the crate names no item here
    assert!(!sysml.contains("Gone"), "{sysml}");
    assert!(!sysml.contains("Idless"), "{sysml}");
}

/// The other half of the same rule: a signature may not name a type the
/// importer refused to write. `Fickle` is an enum with a payload, which
/// has no SysML shape, so the function returning it is skipped whole --
/// a model that mentioned it would have a name nothing answers to.
#[test]
fn a_signature_never_names_a_type_that_was_refused() {
    let json = r#"{
        "root": "r",
        "index": {
            "r": { "name": "picky", "inner": { "module": { "items": ["e", "fun", "plain"] } } },
            "e": { "name": "Fickle", "inner": { "enum": { "variants": ["v"] } } },
            "v": { "name": "Sometimes", "inner": { "variant": { "kind": { "tuple": ["w"] } } } },
            "plain": { "name": "Mood", "inner": { "enum": { "variants": ["m"] } } },
            "m": { "name": "Calm", "inner": { "variant": { "kind": "plain" } } },
            "fun": { "name": "try_it", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [], "output": { "resolved_path": { "path": "Result",
                    "args": { "angle_bracketed": { "args": [
                        { "type": { "resolved_path": { "path": "Mood", "id": "plain" } } },
                        { "type": { "resolved_path": { "path": "Fickle", "id": "e" } } } ] } } } } },
                "header": { "is_async": false } } } }
        }
    }"#;
    let sysml = sysml_rust::rustdoc_to_sysml(json, None).unwrap().sysml;
    assert!(sysml.contains("enum def Mood {"), "{sysml}");
    assert!(sysml.contains("//   Fickle -- not a plain enum"), "{sysml}");
    assert!(
        sysml.contains("//   try_it -- unsupported signature"),
        "{sysml}"
    );
    assert!(!sysml.contains(": Fickle"), "{sysml}");
}

#[test]
fn broken_inputs_are_named_errors() {
    use sysml_rust::ImportError;
    assert!(matches!(
        sysml_rust::rustdoc_to_sysml("not json", None),
        Err(ImportError::NotJson(_))
    ));
    assert_eq!(
        sysml_rust::rustdoc_to_sysml("{}", None),
        Err(ImportError::Malformed("its item index".to_string()))
    );
    assert_eq!(
        sysml_rust::rustdoc_to_sysml(r#"{"index":{}}"#, None),
        Err(ImportError::Malformed("its root module".to_string()))
    );
    assert_eq!(
        sysml_rust::rustdoc_to_sysml(r#"{"index":{},"root":0}"#, None),
        Err(ImportError::Malformed("the root module item".to_string()))
    );
    for error in [
        ImportError::NotJson("eof".to_string()),
        ImportError::Malformed("its item index".to_string()),
    ] {
        assert!(!error.to_string().is_empty());
    }
}

/// Two ways an input used to import as something quietly wrong: a
/// document from before rustdoc renamed `decl` to `sig`, whose every
/// action would have had no parameters, and an associated type of a
/// trait, which is not a method and became an `action def` anyway.
#[test]
fn an_input_this_importer_cannot_read_is_refused_by_name() {
    use sysml_rust::ImportError;
    let old = r#"{
        "format_version": 26,
        "root": "r",
        "index": {
            "r": { "name": "old", "inner": { "module": { "items": ["f"] } } },
            "f": { "name": "old_fn", "inner": { "function": {
                "generics": { "params": [] },
                "decl": { "inputs": [], "output": null },
                "header": { "is_async": false } } } }
        }
    }"#;
    assert_eq!(
        sysml_rust::rustdoc_to_sysml(old, None),
        Err(ImportError::OldFormat(26))
    );
    assert!(ImportError::OldFormat(26).to_string().contains("format 26"));

    let associated = r#"{
        "format_version": 60,
        "root": "r",
        "index": {
            "r": { "name": "assoc", "inner": { "module": { "items": ["t"] } } },
            "t": { "name": "Feeder", "inner": { "trait": { "items": ["a"] } } },
            "a": { "name": "Item", "inner": { "assoc_type": {} } }
        }
    }"#;
    let sysml = sysml_rust::rustdoc_to_sysml(associated, None)
        .unwrap()
        .sysml;
    assert!(sysml.contains("port def Feeder {"), "{sysml}");
    assert!(!sysml.contains("action def Item"), "{sysml}");
    assert!(
        sysml.contains("//   Feeder::Item -- not a method"),
        "{sysml}"
    );
}

/// Rust keeps types and functions in namespaces of their own; SysML
/// keeps everything a package declares in one. `struct Config` and
/// `fn config` both wanted to be `Config`, and the package came out with
/// two members of that name and nothing said about it -- the resolver
/// picked one and `check` reported it clean.
#[test]
fn two_rust_items_that_want_one_sysml_name_are_told_apart() {
    let json = r#"{
        "root": "r",
        "index": {
            "r": { "name": "clash", "inner": { "module": { "items": ["s", "f", "t", "n", "u"] } } },
            "s": { "name": "Config", "inner": { "struct": { "kind": { "plain": { "fields": [] } } } } },
            "f": { "name": "config", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [], "output": { "primitive": "bool" } },
                "header": { "is_async": false } } } },
            "t": { "name": "Store", "inner": { "trait": { "items": ["m"] } } },
            "m": { "name": "store", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [ [ "self", { "borrowed_ref": { "is_mutable": false } } ] ], "output": null },
                "header": { "is_async": false } } } },
            "n": { "name": "StoreStore", "inner": { "struct": { "kind": { "plain": { "fields": [] } } } } },
            "u": { "name": "second_store", "inner": { "trait": { "items": ["m2"] } } },
            "m2": { "name": "store", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [ [ "self", { "borrowed_ref": { "is_mutable": false } } ] ], "output": null },
                "header": { "is_async": false } } } }
        }
    }"#;
    let sysml = sysml_rust::rustdoc_to_sysml(json, None).unwrap().sysml;
    // the type keeps the name, since a signature may refer to it
    assert!(sysml.contains("item def Config {"), "{sysml}");
    assert!(sysml.contains("port def Store {"), "{sysml}");
    // a free function has no trait to be told apart by, so it is numbered
    assert!(sysml.contains("action def Config2 {"), "{sysml}");
    // a trait method takes its trait's name as a prefix
    assert!(sysml.contains("action def SecondStoreStore {"), "{sysml}");
    // and where even that is taken -- there is a `struct StoreStore` --
    // the number comes back
    assert!(sysml.contains("action def StoreStore2 {"), "{sysml}");
    // and each renamed member says so where a reader will find it
    assert!(
        sysml.contains(
            "doc /* `clash::config` is imported as `Config2`: `Config` is another member's name. */"
        ),
        "{sysml}"
    );

    // nothing is declared twice, and every name in it resolves
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file("scalars.kerml", SCALARS);
    let file = ws.add_file("api.sysml", &sysml);
    assert_eq!(ws.resolve_all().unresolved, 0, "{sysml}");
    let names: Vec<&str> = ws
        .model()
        .owned(ws.file_roots(file)[0])
        .iter()
        .filter_map(|&id| ws.model().name(id))
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len(), "{names:?}");
}

/// A generic item has no path that can be spelled without its
/// parameters. Only functions were checked for them, so `struct
/// Wrapper<T>` bound to `clash::Wrapper` and the generator would have
/// written a reference to a type that does not exist.
#[test]
fn a_generic_item_of_any_kind_is_listed_rather_than_bound() {
    let json = r#"{
        "root": "r",
        "index": {
            "r": { "name": "wrapped", "inner": { "module": { "items": ["s", "e", "t", "f"] } } },
            "s": { "name": "Wrapper", "inner": { "struct": {
                "generics": { "params": [ { "name": "T", "kind": { "type": {} } } ] },
                "kind": { "plain": { "fields": [] } } } } },
            "e": { "name": "Either", "inner": { "enum": {
                "generics": { "params": [ { "name": "A", "kind": { "type": {} } } ] },
                "variants": ["v"] } } },
            "v": { "name": "Left", "inner": { "variant": { "kind": "plain" } } },
            "t": { "name": "Feeder", "inner": { "trait": {
                "generics": { "params": [ { "name": "T", "kind": { "type": {} } } ] },
                "items": [] } } },
            "f": { "name": "hold", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [ [ "it", { "resolved_path": { "path": "Wrapper", "id": "s" } } ] ], "output": null },
                "header": { "is_async": false } } } }
        }
    }"#;
    let sysml = sysml_rust::rustdoc_to_sysml(json, None).unwrap().sysml;
    assert!(sysml.contains("//   Wrapper -- generic"), "{sysml}");
    assert!(sysml.contains("//   Either -- generic"), "{sysml}");
    assert!(sysml.contains("//   Feeder -- generic"), "{sysml}");
    // and nothing names one of them afterwards
    assert!(!sysml.contains("Wrapper;"), "{sysml}");
    assert!(
        sysml.contains("//   hold -- unsupported signature"),
        "{sysml}"
    );
}

/// A multiplicity is said once. `Option<Vec<u8>>` used to arrive as
/// `Natural[0..1]`, which says the crate takes at most one number where
/// it takes a list that may be missing -- a level of the type quietly
/// gone. A borrow of anything but a string went the same way: the value
/// was written where the crate takes a reference to it.
#[test]
fn a_container_in_a_container_and_a_borrowed_value_are_refused() {
    let json = r#"{
        "root": "r",
        "index": {
            "r": { "name": "deep", "inner": { "module": { "items": ["s", "b", "m", "flat"] } } },
            "s": { "name": "Layers", "inner": { "struct": { "kind": { "plain": { "fields": ["o", "vv", "boxed"] } } } } },
            "o": { "name": "maybe", "inner": { "struct_field": { "resolved_path": { "path": "Option",
                "args": { "angle_bracketed": { "args": [ { "type": { "resolved_path": { "path": "Vec",
                    "args": { "angle_bracketed": { "args": [ { "type": { "primitive": "u8" } } ] } } } } } ] } } } } } },
            "vv": { "name": "grid", "inner": { "struct_field": { "resolved_path": { "path": "Vec",
                "args": { "angle_bracketed": { "args": [ { "type": { "resolved_path": { "path": "Vec",
                    "args": { "angle_bracketed": { "args": [ { "type": { "primitive": "u8" } } ] } } } } } ] } } } } } },
            "boxed": { "name": "held", "inner": { "struct_field": { "resolved_path": { "path": "Box",
                "args": { "angle_bracketed": { "args": [ { "type": { "resolved_path": { "path": "Vec",
                    "args": { "angle_bracketed": { "args": [ { "type": { "primitive": "u8" } } ] } } } } } ] } } } } } },
            "b": { "name": "weigh", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [ [ "it", { "borrowed_ref": { "is_mutable": false,
                    "type": { "resolved_path": { "path": "Layers", "id": "s" } } } } ] ], "output": null },
                "header": { "is_async": false } } } },
            "m": { "name": "scrub", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [ [ "text", { "borrowed_ref": { "is_mutable": true,
                    "type": { "primitive": "str" } } } ] ], "output": null },
                "header": { "is_async": false } } } },
            "flat": { "name": "count", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [ [ "of", { "resolved_path": { "path": "Vec",
                    "args": { "angle_bracketed": { "args": [ { "type": { "primitive": "u8" } } ] } } } } ] ], "output": null },
                "header": { "is_async": false } } } }
        }
    }"#;
    let sysml = sysml_rust::rustdoc_to_sysml(json, None).unwrap().sysml;
    assert!(
        sysml.contains("//   Layers.maybe -- unmappable type"),
        "{sysml}"
    );
    assert!(
        sysml.contains("//   Layers.grid -- unmappable type"),
        "{sysml}"
    );
    // a Box is not a level of its own: it disappears, and what it held
    // keeps the multiplicity it asked for
    assert!(sysml.contains("attribute held : RustU8[*];"), "{sysml}");
    // one level is still one level
    assert!(sysml.contains("in 'of' : RustU8[*];"), "{sysml}");
    assert!(
        sysml.contains("//   weigh -- unsupported signature"),
        "{sysml}"
    );
    assert!(
        sysml.contains("//   scrub -- unsupported signature"),
        "{sysml}"
    );
}

/// The round trip, against the crate itself. A `&str` folded into
/// `String` came back out of the generator as a `String`, so the
/// delegating method read `fn label(&self, name: String)` where the
/// trait declares `fn label(&self, name: &str)` -- generated code that
/// compiles on its own and does not compile against what it calls. The
/// only way to know is to compile it against the real crate.
#[test]
fn what_the_importer_writes_calls_the_crate_it_was_read_from() {
    let api = sysml_rust::rustdoc_to_sysml(&fixture(), None)
        .unwrap()
        .sysml;
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file("scalars.kerml", SCALARS);
    ws.add_file("api.sysml", &api);
    let file = ws.add_file(
        "desk.sysml",
        "package Desk {\n\
         \tprivate import InventoryStoreApi::*;\n\
         \tpart def Counter {\n\
         \t\tport sess : Session;\n\
         \t\tperform action label : Label;\n\
         \t}\n\
         }\n",
    );
    assert_eq!(ws.resolve_all().unresolved, 0, "{api}");
    let roots = ws.file_roots(file).to_vec();
    let rust = sysml_rust::generate(ws.model(), &roots).unwrap().rust;
    assert!(
        rust.contains("pub fn label(&self, name: &str) -> bool {"),
        "{rust}"
    );
    assert!(rust.contains("self.sess.label(name)"), "{rust}");

    let dir = std::env::temp_dir().join("sysml-rust-round-trip");
    std::fs::create_dir_all(&dir).unwrap();
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/inventory-store/src/lib.rs");
    let rmeta = dir.join("libinventory_store.rmeta");
    let built = std::process::Command::new(&rustc)
        .args([
            "--crate-type",
            "lib",
            "--edition",
            "2021",
            "--crate-name",
            "inventory_store",
            "--emit=metadata",
        ])
        .arg("-o")
        .arg(&rmeta)
        .arg(&crate_root)
        .output()
        .expect("rustc runs");
    assert!(
        built.status.success(),
        "the fixture crate must compile:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let at = dir.join("counter.rs");
    std::fs::write(&at, &rust).unwrap();
    let out = std::process::Command::new(&rustc)
        .args([
            "--crate-type",
            "lib",
            "--edition",
            "2021",
            "--emit=metadata",
        ])
        .arg("--extern")
        .arg(format!("inventory_store={}", rmeta.display()))
        .arg("-o")
        .arg(dir.join("counter.rmeta"))
        .arg(&at)
        .output()
        .expect("rustc runs");
    assert!(
        out.status.success(),
        "generated Rust does not compile against the crate it calls:\n{}\n---\n{rust}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// What `sysml check` asks of a package, against the standard library:
/// every name resolved, and every constraint the specification states
/// held.
fn checked(sysml: &str) -> sysml_semantics::Workspace {
    let mut ws = sysml_semantics::Workspace::new();
    for (name, text) in sysml_stdlib::FILES {
        ws.add_file(*name, text);
    }
    let file = ws.add_file("api.sysml", sysml);
    ws.resolve_all();
    let diagnosis = ws.diagnose(&[file]);
    assert!(
        diagnosis.found.syntax.is_empty() && diagnosis.found.names.is_empty(),
        "{:?}\n{sysml}",
        diagnosis.found
    );
    assert!(diagnosis.asked, "the constraints were asked\n{sysml}");
    let broken: Vec<&str> = diagnosis.rules.violations.iter().map(|v| v.rule).collect();
    assert!(broken.is_empty(), "{broken:?}\n{sysml}");
    ws
}

/// `pub use b::Dup as Renamed;` makes `b::Dup` public as `Renamed`, and
/// only as that: the binding said `crate::Dup`, a path nothing answers
/// to. The definition takes the name the crate exports, and a signature
/// naming the struct names it by that name too.
#[test]
fn a_renamed_re_export_is_bound_by_the_name_it_is_exported_under() {
    let json = r#"{
        "root": "r",
        "index": {
            "r": { "name": "renaming", "inner": { "module": { "items": ["use_dup", "fun"] } } },
            "use_dup": { "name": null, "visibility": "public", "inner": { "use": { "source": "b::Dup", "name": "Renamed", "id": "dup", "is_glob": false } } },
            "dup": { "name": "Dup", "visibility": "public", "inner": { "struct": { "kind": { "plain": { "fields": ["y"] } } } } },
            "y": { "name": "y", "inner": { "struct_field": { "primitive": "f64" } } },
            "fun": { "name": "make", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [], "output": { "resolved_path": { "path": "b::Dup", "id": "dup" } } },
                "header": { "is_async": false } } } }
        }
    }"#;
    let sysml = sysml_rust::rustdoc_to_sysml(json, None).unwrap().sysml;
    assert!(sysml.contains("item def Renamed {"), "{sysml}");
    assert!(
        sysml.contains(":>> path = \"renaming::Renamed\";"),
        "{sysml}"
    );
    assert!(sysml.contains("out result : Renamed;"), "{sysml}");
    assert!(!sysml.contains("Dup"), "{sysml}");
    checked(&sysml);
}

/// `pub use private::*;` is how a crate makes a private module's items
/// its own, and they were neither imported nor listed as not imported.
/// The glob is followed into the module -- and into a glob inside it --
/// for what is public there; a name the root spells out itself wins, as
/// it does in Rust; and a glob over an enum's variants is listed.
#[test]
fn a_glob_re_export_brings_the_module_s_public_items() {
    let json = r#"{
        "root": "r",
        "index": {
            "r": { "name": "globby", "inner": { "module": { "items": ["glob", "glob_again", "own", "enum_glob", "mode"] } } },
            "glob": { "name": null, "visibility": "public", "inner": { "use": { "source": "private", "name": "private", "id": "private", "is_glob": true } } },
            "glob_again": { "name": null, "visibility": "public", "inner": { "use": { "source": "private", "name": "private", "id": "private", "is_glob": true } } },
            "enum_glob": { "name": null, "visibility": "public", "inner": { "use": { "source": "Mode", "name": "Mode", "id": "mode", "is_glob": true } } },
            "private": { "name": "private", "visibility": "crate", "inner": { "module": { "items": ["globbed", "fun", "hidden", "shadowed", "inner_glob"] } } },
            "inner_glob": { "name": null, "visibility": "public", "inner": { "use": { "source": "self::deeper", "name": "deeper", "id": "deeper", "is_glob": true } } },
            "deeper": { "name": "deeper", "visibility": "public", "inner": { "module": { "items": ["deep"] } } },
            "deep": { "name": "Deep", "visibility": "public", "inner": { "struct": { "kind": { "plain": { "fields": ["x"] } } } } },
            "globbed": { "name": "Globbed", "visibility": "public", "inner": { "struct": { "kind": { "plain": { "fields": ["x"] } } } } },
            "x": { "name": "x", "inner": { "struct_field": { "primitive": "i64" } } },
            "hidden": { "name": "Hidden", "visibility": "crate", "inner": { "struct": { "kind": { "plain": { "fields": ["x"] } } } } },
            "shadowed": { "name": "Own", "visibility": "public", "inner": { "struct": { "kind": { "plain": { "fields": ["x"] } } } } },
            "own": { "name": "Own", "visibility": "public", "inner": { "struct": { "kind": { "plain": { "fields": [] } } } } },
            "mode": { "name": "Mode", "visibility": "public", "inner": { "enum": { "variants": ["on"] } } },
            "on": { "name": "On", "inner": { "variant": { "kind": "plain" } } },
            "fun": { "name": "globbed_fn", "visibility": "public", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [], "output": { "resolved_path": { "path": "Globbed", "id": "globbed" } } },
                "header": { "is_async": false } } } }
        }
    }"#;
    let imported = sysml_rust::rustdoc_to_sysml(json, None).unwrap();
    let sysml = &imported.sysml;
    assert!(sysml.contains("item def Globbed {"), "{sysml}");
    assert!(sysml.contains(":>> path = \"globby::Globbed\";"), "{sysml}");
    assert!(sysml.contains("action def GlobbedFn {"), "{sysml}");
    assert!(sysml.contains("item def Deep {"), "{sysml}");
    // read once, however many globs reach it
    assert_eq!(sysml.matches("item def Globbed {").count(), 1, "{sysml}");
    // what the module keeps to the crate stays there
    assert!(!sysml.contains("Hidden"), "{sysml}");
    // the root's own `Own` is the one bound, not the glob's
    assert_eq!(sysml.matches("item def Own {").count(), 1, "{sysml}");
    assert!(!sysml.contains("declared twice"), "{sysml}");
    assert!(
        sysml.contains("//   Mode::* -- re-exports an enum's variants"),
        "{sysml}"
    );
    checked(sysml);
}

/// A type's own functions -- `impl Part { pub fn new() -> Self; pub fn
/// total(&self) -> f64 }` -- were neither imported nor listed: rustdoc
/// lists an `impl` under the type and not under the module. They are
/// imported as a trait's methods are, `Self` read as the type, and what
/// cannot be is listed under the type's name.
#[test]
fn a_type_s_own_functions_are_imported_or_listed() {
    let json = r#"{
        "root": "r",
        "index": {
            "r": { "name": "inherent", "inner": { "module": { "items": ["part", "mode"] } } },
            "part": { "name": "Part", "visibility": "public", "inner": { "struct": {
                "kind": { "plain": { "fields": ["w", "c"] } },
                "generics": { "params": [] },
                "impls": ["own", "debug", "auto", "blanket"] } } },
            "w": { "name": "weight", "inner": { "struct_field": { "primitive": "f64" } } },
            "c": { "name": "code", "inner": { "struct_field": { "primitive": "i64" } } },
            "own": { "inner": { "impl": { "trait": null, "is_synthetic": false, "blanket_impl": null,
                "generics": { "params": [] },
                "items": ["new", "total", "grow", "pick", "danger", "secret", "LIMIT"] } } },
            "debug": { "inner": { "impl": { "trait": { "path": "Debug", "id": "dbg" }, "is_synthetic": false, "blanket_impl": null,
                "generics": { "params": [] }, "items": ["fmt"] } } },
            "auto": { "inner": { "impl": { "trait": { "path": "Send", "id": "send" }, "is_synthetic": true, "blanket_impl": null,
                "generics": { "params": [] }, "items": [] } } },
            "blanket": { "inner": { "impl": { "trait": { "path": "From", "id": "from" }, "is_synthetic": false, "blanket_impl": { "generic": "T" },
                "generics": { "params": [] }, "items": ["from_fn"] } } },
            "new": { "name": "new", "visibility": "public", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [], "output": { "generic": "Self" } },
                "header": { "is_async": false, "is_unsafe": false } } } },
            "total": { "name": "total", "visibility": "public", "docs": "The whole of it.", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [ [ "self", { "borrowed_ref": { "is_mutable": false, "type": { "generic": "Self" } } } ] ], "output": { "primitive": "f64" } },
                "header": { "is_async": false, "is_unsafe": false } } } },
            "grow": { "name": "grow", "visibility": "public", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [ [ "self", { "borrowed_ref": { "is_mutable": true, "type": { "generic": "Self" } } } ], [ "amount", { "primitive": "f64" } ] ], "output": null },
                "header": { "is_async": false, "is_unsafe": false } } } },
            "pick": { "name": "pick", "visibility": "public", "inner": { "function": {
                "generics": { "params": [ { "name": "T" } ] },
                "sig": { "inputs": [], "output": null },
                "header": { "is_async": false, "is_unsafe": false } } } },
            "danger": { "name": "danger", "visibility": "public", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [], "output": null },
                "header": { "is_async": false, "is_unsafe": true } } } },
            "secret": { "name": "secret", "visibility": "crate", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [], "output": null },
                "header": { "is_async": false, "is_unsafe": false } } } },
            "LIMIT": { "name": "LIMIT", "visibility": "public", "inner": { "assoc_const": { "type": { "primitive": "u32" } } } },
            "fmt": { "name": "fmt", "inner": { "function": { "generics": { "params": [] }, "sig": { "inputs": [], "output": null }, "header": {} } } },
            "from_fn": { "name": "from", "inner": { "function": { "generics": { "params": [] }, "sig": { "inputs": [], "output": null }, "header": {} } } },
            "mode": { "name": "Mode", "visibility": "public", "inner": { "enum": {
                "variants": ["on"], "generics": { "params": [] }, "impls": ["mode_own"] } } },
            "on": { "name": "On", "inner": { "variant": { "kind": "plain" } } },
            "mode_own": { "inner": { "impl": { "trait": null, "is_synthetic": false, "blanket_impl": null,
                "generics": { "params": [] }, "items": ["flip", "mode_new"] } } },
            "flip": { "name": "flip", "visibility": "public", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [ [ "self", { "generic": "Self" } ] ], "output": { "generic": "Self" } },
                "header": { "is_async": false, "is_unsafe": false } } } },
            "mode_new": { "name": "new", "visibility": "public", "inner": { "function": {
                "generics": { "params": [] },
                "sig": { "inputs": [], "output": { "generic": "Self" } },
                "header": { "is_async": false, "is_unsafe": false } } } }
        }
    }"#;
    let imported = sysml_rust::rustdoc_to_sysml(json, None).unwrap();
    let sysml = &imported.sysml;
    assert!(
        sysml.contains(
            "\taction def New {\n\t\t@code { :>> writtenIn = \"rust\"; :>> path = \"inherent::Part::new\"; \
             :>> module = \"inherent\"; :>> takesSelf = \"\"; :>> isAsync = false; :>> isFallible = false; }\n\
             \t\tout result : Part;\n"
        ),
        "{sysml}"
    );
    assert!(
        sysml.contains(
            "\taction def Total {\n\t\tdoc /* The whole of it. */\n\t\t@code { :>> writtenIn = \"rust\"; \
             :>> path = \"inherent::Part::total\"; :>> module = \"inherent\"; :>> takesSelf = \"&self\";"
        ),
        "{sysml}"
    );
    assert!(sysml.contains(":>> path = \"inherent::Part::grow\"; :>> module = \"inherent\"; :>> takesSelf = \"&mut self\";"), "{sysml}");
    assert!(sysml.contains("\t\tin amount : Real;\n"), "{sysml}");
    // an enum's own, `Self` its own type; a second `new` under its type's name
    assert!(sysml.contains(":>> path = \"inherent::Mode::flip\"; :>> module = \"inherent\"; :>> takesSelf = \"self\";"), "{sysml}");
    assert!(sysml.contains("\taction def ModeNew {\n"), "{sysml}");
    // what is not imported is listed under the type's name
    for line in [
        "Part::pick -- unsupported signature",
        "Part::danger -- unsafe",
        "Part::LIMIT -- not a method",
    ] {
        assert!(
            imported.skipped.iter().any(|it| it.starts_with(line)),
            "{line}: {:?}",
            imported.skipped
        );
    }
    // a trait's impl, an auto trait's and a blanket one bring nothing
    assert!(!sysml.contains("fmt") && !sysml.contains("From"), "{sysml}");
    // what is not public is not the crate's API
    assert!(!sysml.contains("secret"), "{sysml}");
    // and the package checks clean, its field called `code` included
    checked(sysml);
}

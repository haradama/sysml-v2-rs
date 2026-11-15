//! A surface all the way through: read from JSON, written as a package
//! that parses and resolves, and every way the writer refuses part of
//! one rather than writing a name nothing answers to.

use sysml_rust::surface::{
    Callable, Enumeration, Field, Imported, Interface, Item, Multiplicity, Primitive, Record,
    Scalar, Surface, SurfaceError, TypeRef,
};

/// The scalar types the written package imports, enough to resolve.
const SCALARS: &str = "package ScalarValues {\n\
    \tabstract datatype Boolean;\n\
    \tabstract datatype String;\n\
    \tabstract datatype Real;\n\
    \tabstract datatype Integer;\n\
    \tabstract datatype Natural;\n\
}\n";

/// A surface with one of everything, the way a reader of some other
/// language would write it.
const INVENTORY: &str = r#"{
    "language": "python",
    "module": "inventory",
    "unit": "package",
    "tool": "sysml-import-python",
    "scalars": [
        { "name": "PyBytes", "of": "String", "path": "bytes", "capabilities": ["Hashable"] }
    ],
    "items": [
        {
            "kind": "record",
            "name": "StockQuery",
            "path": "inventory.StockQuery",
            "doc": "What to look up.\nOn two lines, with */ inside.",
            "capabilities": ["Eq", "Hash"],
            "fields": [
                { "name": "sku", "type": "String", "doc": "The article." },
                { "name": "warehouses", "type": "Integer", "multiplicity": "many" },
                { "name": "note", "type": "String", "multiplicity": "optional" },
                { "name": "filter", "type": "Boolean" },
                { "name": "raw", "type": "PyBytes" }
            ]
        },
        {
            "kind": "enumeration",
            "name": "Region",
            "path": "inventory.Region",
            "doc": "Where a warehouse is.",
            "values": ["Tokyo", "Osaka", "import"]
        },
        {
            "kind": "interface",
            "name": "InventoryStore",
            "path": "inventory.InventoryStore",
            "doc": "The inventory service.",
            "methods": [
                {
                    "name": "get_stock",
                    "path": "inventory.InventoryStore.get_stock",
                    "doc": "Look one article up.",
                    "receiver": "self",
                    "fallible": true,
                    "parameters": [
                        { "name": "query", "type": "StockQuery", "doc": "What to look up." }
                    ],
                    "result": { "type": "Region", "multiplicity": "optional" },
                    "error": "StockQuery"
                },
                {
                    "name": "watch",
                    "path": "inventory.InventoryStore.watch",
                    "receiver": "self",
                    "async": true,
                    "parameters": [ { "name": "sku", "type": "String" } ]
                }
            ]
        },
        {
            "kind": "function",
            "name": "default_region",
            "path": "inventory.default_region",
            "result": { "type": "Region" }
        }
    ],
    "skipped": [ "Pair -- a tuple, which has no field names" ]
}"#;

/// Every name in a package resolves, or the test says which did not.
fn resolved(sysml: &str) -> sysml_semantics::Workspace {
    let parse = sysml_syntax::parse(sysml);
    assert!(
        parse.ok(),
        "the package must parse: {:?}\n{sysml}",
        parse.errors().first()
    );
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file("scalars.kerml", SCALARS);
    ws.add_file("api.sysml", sysml);
    let stats = ws.resolve_all();
    assert_eq!(
        stats.unresolved,
        0,
        "every reference must resolve: {:?}\n{sysml}",
        ws.unresolved()
    );
    ws
}

#[test]
fn a_surface_of_everything_is_written_as_a_package_that_resolves() {
    let surface = sysml_rust::surface::read(INVENTORY).unwrap();
    let first = sysml_rust::surface::to_sysml(&surface, None);
    let second = sysml_rust::surface::to_sysml(&surface, None);
    assert_eq!(first, second, "two runs must write identical bytes");
    let sysml = &first.sysml;
    resolved(sysml);

    // the header names the unit and the tool the surface said
    assert!(
        sysml.starts_with("// generated from package `inventory` by `sysml-import-python`"),
        "{sysml}"
    );
    assert!(sysml.contains("package InventoryApi {"), "{sysml}");
    assert!(sysml.contains("metadata def code {"), "{sysml}");
    // the language's own scalar, under the SysML one it is a kind of
    assert!(
        sysml.contains(
            "attribute def PyBytes :> String { @code { :>> writtenIn = \"python\"; \
             :>> path = \"bytes\"; :>> capabilities = \"Hashable\"; } }"
        ),
        "{sysml}"
    );
    // a record, with a multiplicity per container, a reserved word
    // quoted, a doc flattened and kept from ending early, capabilities
    // written where the surface said any
    assert!(sysml.contains("item def StockQuery {"), "{sysml}");
    assert!(
        sysml.contains(
            "@code { :>> writtenIn = \"python\"; :>> path = \"inventory.StockQuery\"; \
             :>> module = \"inventory\"; :>> capabilities = \"Eq, Hash\"; }"
        ),
        "{sysml}"
    );
    // a doc is inside what it documents, where it is that element's own:
    // beside it, it was the package's
    assert!(
        sysml.contains(
            "item def StockQuery {\n\t\tdoc /* What to look up. On two lines, with *\\/ inside. */\n"
        ),
        "{sysml}"
    );
    assert!(
        sysml.contains("\t\tattribute sku : String { doc /* The article. */ }\n"),
        "{sysml}"
    );
    assert!(
        sysml.contains("attribute warehouses : Integer[*];"),
        "{sysml}"
    );
    assert!(sysml.contains("attribute note : String[0..1];"), "{sysml}");
    assert!(sysml.contains("attribute 'filter' : Boolean;"), "{sysml}");
    assert!(sysml.contains("attribute raw : PyBytes;"), "{sysml}");
    // an enumeration, its reserved-word member quoted
    assert!(sysml.contains("enum def Region {"), "{sysml}");
    assert!(sysml.contains("\t\tenum 'import';"), "{sysml}");
    // an interface and its methods beside it
    assert!(sysml.contains("port def InventoryStore {"), "{sysml}");
    assert!(sysml.contains("action def GetStock {"), "{sysml}");
    assert!(
        sysml.contains(
            ":>> path = \"inventory.InventoryStore.get_stock\"; :>> module = \"inventory\"; \
             :>> takesSelf = \"self\"; :>> isAsync = false; :>> isFallible = true; }"
        ),
        "{sysml}"
    );
    assert!(
        sysml.contains("\t\tin query : StockQuery { doc /* What to look up. */ }\n"),
        "{sysml}"
    );
    assert!(sysml.contains("out result : Region[0..1];"), "{sysml}");
    // an error is at most one of its type, whatever else is said of it
    assert!(sysml.contains("out error : StockQuery[0..1];"), "{sysml}");
    assert!(sysml.contains(":>> isAsync = true;"), "{sysml}");
    // a function, CamelCased, called on nothing
    assert!(sysml.contains("action def DefaultRegion {"), "{sysml}");
    assert!(
        sysml.contains(":>> takesSelf = \"\"; :>> isAsync = false; :>> isFallible = false;"),
        "{sysml}"
    );
    // what the front end could not state is listed, in the package and
    // in the answer
    assert!(
        sysml.contains("//   Pair -- a tuple, which has no field names"),
        "{sysml}"
    );
    assert_eq!(first.skipped, ["Pair -- a tuple, which has no field names"]);

    // a name given wins over the surface's, which wins over the derived
    let named = sysml_rust::surface::to_sysml(&surface, Some("Warehouse")).sysml;
    assert!(named.contains("package Warehouse {"), "{named}");
    let mut own = surface.clone();
    own.package = Some("Stock".to_string());
    assert!(sysml_rust::surface::to_sysml(&own, None)
        .sysml
        .contains("package Stock {"));
    assert!(sysml_rust::surface::to_sysml(&own, Some("Warehouse"))
        .sysml
        .contains("package Warehouse {"));
}

#[test]
fn what_is_read_is_what_is_written_back() {
    let surface = sysml_rust::surface::read(INVENTORY).unwrap();
    assert_eq!(surface.language, "python");
    assert_eq!(surface.scalars[0].of, Primitive::String);
    let Item::Record(record) = &surface.items[0] else {
        panic!("the first item is the record");
    };
    assert_eq!(record.fields[1].multiplicity, Multiplicity::Many);
    assert_eq!(record.fields[3].multiplicity, Multiplicity::One);
    // and back through JSON to the same surface, with nothing that was
    // left out written out
    let json = serde_json::to_string_pretty(&surface).unwrap();
    assert!(!json.contains("\"multiplicity\": \"one\""), "{json}");
    assert!(!json.contains("\"async\": false"), "{json}");
    assert!(!json.contains("\"receiver\": \"\""), "{json}");
    assert_eq!(sysml_rust::surface::read(&json).unwrap(), surface);
}

#[test]
fn nothing_written_means_one_of_a_module() {
    let surface = Surface {
        language: "c".to_string(),
        module: String::new(),
        items: vec![Item::Function(Callable {
            name: "reset".to_string(),
            path: "reset".to_string(),
            ..Callable::default()
        })],
        ..Surface::default()
    };
    let sysml = sysml_rust::surface::to_sysml(&surface, None).sysml;
    resolved(&sysml);
    assert!(
        sysml.starts_with("// generated from module `` by `sysml import-code`"),
        "{sysml}"
    );
    // a module with no name is not written as one
    assert!(sysml.contains("package Api {"), "{sysml}");
    assert!(
        sysml
            .contains("@code { :>> writtenIn = \"c\"; :>> path = \"reset\"; :>> takesSelf = \"\";"),
        "{sysml}"
    );
    assert!(!sysml.contains(":>> module"), "{sysml}");
    // and nothing skipped means no list of it
    assert!(!sysml.contains("not imported"), "{sysml}");
}

/// The other half of what makes a package usable: a signature may not
/// name a type the surface never declared. A field is left out on its
/// own; a callable is left out whole, since a call with a parameter
/// missing is a different call.
#[test]
fn a_name_the_surface_does_not_declare_is_refused_and_listed() {
    let surface = Surface {
        language: "python".to_string(),
        module: "shapes".to_string(),
        items: vec![
            Item::Record(Record {
                name: "Shape".to_string(),
                path: "shapes.Shape".to_string(),
                fields: vec![
                    Field {
                        name: "width".to_string(),
                        of: "Real".to_string(),
                        ..Field::default()
                    },
                    Field {
                        name: "colour".to_string(),
                        of: "Colour".to_string(),
                        ..Field::default()
                    },
                ],
                ..Record::default()
            }),
            Item::Function(Callable {
                name: "paint".to_string(),
                path: "shapes.paint".to_string(),
                parameters: vec![Field {
                    name: "in_colour".to_string(),
                    of: "Colour".to_string(),
                    ..Field::default()
                }],
                ..Callable::default()
            }),
            Item::Function(Callable {
                name: "pick".to_string(),
                path: "shapes.pick".to_string(),
                result: Some(TypeRef {
                    of: "Colour".to_string(),
                    multiplicity: Multiplicity::One,
                }),
                ..Callable::default()
            }),
            Item::Function(Callable {
                name: "try_paint".to_string(),
                path: "shapes.try_paint".to_string(),
                error: Some("PaintError".to_string()),
                ..Callable::default()
            }),
            Item::Function(Callable {
                name: "area".to_string(),
                path: "shapes.area".to_string(),
                parameters: vec![Field {
                    name: "of".to_string(),
                    of: "Shape".to_string(),
                    ..Field::default()
                }],
                result: Some(TypeRef {
                    of: "Real".to_string(),
                    multiplicity: Multiplicity::One,
                }),
                ..Callable::default()
            }),
        ],
        ..Surface::default()
    };
    let imported = sysml_rust::surface::to_sysml(&surface, None);
    resolved(&imported.sysml);
    assert!(imported.sysml.contains("attribute width : Real;"));
    assert!(!imported.sysml.contains(": Colour"), "{}", imported.sysml);
    assert!(imported.sysml.contains("action def Area {"));
    assert!(imported.sysml.contains("in 'of' : Shape;"));
    assert_eq!(
        imported.skipped,
        [
            "Shape.colour -- names `Colour`, which the surface does not declare",
            "shapes.paint -- names `Colour`, which the surface does not declare",
            "shapes.pick -- names `Colour`, which the surface does not declare",
            "shapes.try_paint -- names `PaintError`, which the surface does not declare",
        ]
    );
}

/// A package declares everything in one namespace. A type keeps its
/// name, since a signature refers to it; a callable that wants a taken
/// name is told apart -- by its interface's name, or by a number -- and
/// says so where a reader will find it. A second type under a taken
/// name is refused instead, because renaming it would change what every
/// signature naming it means, and so is a type named after one of the
/// scalars the package imports.
#[test]
fn what_wants_a_taken_name_is_told_apart_or_refused() {
    let function = |name: &str| {
        Item::Function(Callable {
            name: name.to_string(),
            path: format!("clash.{name}"),
            ..Callable::default()
        })
    };
    let record = |name: &str| {
        Item::Record(Record {
            name: name.to_string(),
            path: format!("clash.{name}"),
            ..Record::default()
        })
    };
    let surface = Surface {
        language: "python".to_string(),
        module: "clash".to_string(),
        scalars: vec![
            Scalar {
                name: "PyBytes".to_string(),
                of: Primitive::String,
                path: "bytes".to_string(),
                capabilities: Vec::new(),
            },
            Scalar {
                name: "PyBytes".to_string(),
                of: Primitive::Natural,
                path: "bytearray".to_string(),
                capabilities: Vec::new(),
            },
            Scalar {
                name: "Integer".to_string(),
                of: Primitive::Integer,
                path: "int".to_string(),
                capabilities: Vec::new(),
            },
        ],
        items: vec![
            record("Config"),
            function("config"),
            Item::Interface(Interface {
                name: "Store".to_string(),
                path: "clash.Store".to_string(),
                methods: vec![Callable {
                    name: "store".to_string(),
                    path: "clash.Store.store".to_string(),
                    receiver: "self".to_string(),
                    ..Callable::default()
                }],
                ..Interface::default()
            }),
            record("StoreStore"),
            Item::Interface(Interface {
                name: "second_store".to_string(),
                path: "clash.second_store".to_string(),
                methods: vec![Callable {
                    name: "store".to_string(),
                    path: "clash.second_store.store".to_string(),
                    ..Callable::default()
                }],
                ..Interface::default()
            }),
            record("Config"),
            Item::Enumeration(Enumeration {
                name: "String".to_string(),
                path: "clash.String".to_string(),
                ..Enumeration::default()
            }),
            function("py_bytes"),
            function("integer"),
        ],
        ..Surface::default()
    };
    let imported = sysml_rust::surface::to_sysml(&surface, None);
    let sysml = &imported.sysml;
    let ws = resolved(sysml);
    // the type keeps the name, since a signature may refer to it
    assert!(sysml.contains("item def Config {"), "{sysml}");
    assert!(sysml.contains("port def Store {"), "{sysml}");
    // a function has no interface to be told apart by, so it is numbered
    assert!(sysml.contains("action def Config2 {"), "{sysml}");
    // a method takes its interface's name as a prefix
    assert!(sysml.contains("action def SecondStoreStore {"), "{sysml}");
    // and where even that is taken -- there is a record `StoreStore` --
    // the number comes back
    assert!(sysml.contains("action def StoreStore2 {"), "{sysml}");
    // each renamed member says so where a reader will find it
    assert!(
        sysml.contains(
            "doc /* `clash.config` is imported as `Config2`: `Config` is another member's name. */"
        ),
        "{sysml}"
    );
    // a scalar the package imports is a name nothing here may take,
    // neither a type nor a callable
    assert!(sysml.contains("action def PyBytes2 {"), "{sysml}");
    assert!(sysml.contains("action def Integer2 {"), "{sysml}");
    assert!(!sysml.contains("enum def String"), "{sysml}");
    assert_eq!(
        imported.skipped,
        [
            "PyBytes -- declared twice; the first is kept",
            "Integer -- the name of a scalar the package imports",
            "Config -- declared twice; the first is kept",
            "String -- the name of a scalar the package imports",
        ]
    );
    assert_eq!(sysml.matches("attribute def PyBytes").count(), 1, "{sysml}");
    assert_eq!(sysml.matches("item def Config {").count(), 1, "{sysml}");

    // nothing is declared twice
    let root = ws
        .model()
        .ids()
        .find(|&id| ws.model().name(id) == Some("ClashApi"))
        .expect("the package is in the model");
    let names: Vec<&str> = ws
        .model()
        .owned(root)
        .iter()
        .filter_map(|&id| ws.model().name(id))
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len(), "{names:?}");
}

#[test]
fn a_quote_in_what_is_bound_does_not_end_the_string() {
    let surface = Surface {
        language: "c".to_string(),
        module: "lib \"one\"".to_string(),
        items: vec![Item::Function(Callable {
            name: "say".to_string(),
            path: "say(\"hi\\n\")".to_string(),
            ..Callable::default()
        })],
        ..Surface::default()
    };
    let sysml = sysml_rust::surface::to_sysml(&surface, Some("Lib")).sysml;
    resolved(&sysml);
    assert!(
        sysml.contains(":>> path = \"say(\\\"hi\\\\n\\\")\"; :>> module = \"lib \\\"one\\\"\";"),
        "{sysml}"
    );
}

#[test]
fn a_package_named_after_a_reserved_word_is_quoted() {
    let surface = Surface {
        language: "c".to_string(),
        module: "filter".to_string(),
        ..Surface::default()
    };
    let sysml = sysml_rust::surface::to_sysml(&surface, Some("filter")).sysml;
    resolved(&sysml);
    assert!(sysml.contains("package 'filter' {"), "{sysml}");
    // the derived name is not one
    assert!(sysml_rust::surface::to_sysml(&surface, None)
        .sysml
        .contains("package FilterApi {"));
}

#[test]
fn text_that_is_not_a_surface_is_refused_by_name() {
    assert!(matches!(
        sysml_rust::surface::read("not json"),
        Err(SurfaceError::NotJson(_))
    ));
    assert!(matches!(
        sysml_rust::surface::read(""),
        Err(SurfaceError::NotJson(_))
    ));
    // a field the surface has no name for is refused rather than
    // ignored: a front end that misspells one would otherwise be told
    // nothing
    let misspelt = r#"{ "language": "python", "module": "m",
        "items": [ { "kind": "record", "name": "R", "path": "m.R",
            "fields": [ { "name": "f", "type": "String", "multiplicty": "many" } ] } ] }"#;
    let Err(SurfaceError::NotASurface(why)) = sysml_rust::surface::read(misspelt) else {
        panic!("a misspelt field is refused");
    };
    assert!(why.contains("multiplicty"), "{why}");
    // and a wrong word, a wrong kind, a missing field
    for (text, word) in [
        (
            r#"{ "language": "python", "module": "m",
                "items": [ { "kind": "record", "name": "R", "path": "m.R",
                    "fields": [ { "name": "f", "type": "String", "multiplicity": "some" } ] } ] }"#,
            "some",
        ),
        (
            r#"{ "language": "python", "module": "m",
                "items": [ { "kind": "class", "name": "R", "path": "m.R" } ] }"#,
            "class",
        ),
        (
            r#"{ "language": "python", "module": "m",
                "scalars": [ { "name": "S", "of": "Text", "path": "str" } ] }"#,
            "Text",
        ),
        (r#"{ "module": "m" }"#, "language"),
    ] {
        let Err(SurfaceError::NotASurface(why)) = sysml_rust::surface::read(text) else {
            panic!("refused: {text}");
        };
        assert!(why.contains(word), "{why} should name `{word}`");
    }
    for error in [
        SurfaceError::NotJson("eof".to_string()),
        SurfaceError::NotASurface("missing field `language`".to_string()),
    ] {
        assert!(error.to_string().contains("the input is not"));
    }
}

#[test]
fn the_primitives_answer_to_their_names_and_nothing_else() {
    for primitive in Primitive::ALL {
        assert_eq!(Primitive::named(primitive.name()), Some(primitive));
    }
    assert_eq!(Primitive::named("Number"), None);
    assert_eq!(Multiplicity::default(), Multiplicity::One);
    assert!(Multiplicity::One.is_one());
    assert!(!Multiplicity::Many.is_one());
}

#[test]
fn what_is_imported_is_one_value() {
    let imported = Imported {
        sysml: "package P {}\n".to_string(),
        skipped: Vec::new(),
    };
    assert_eq!(imported.clone(), imported);
    assert!(format!("{imported:?}").contains("package P"));
}

#[cfg(feature = "schema")]
#[test]
fn the_schema_says_what_a_surface_is() {
    let schema = sysml_rust::surface::schema();
    let properties = schema["properties"].as_object().unwrap();
    for field in [
        "language", "module", "unit", "tool", "package", "scalars", "items", "skipped",
    ] {
        assert!(properties.contains_key(field), "{field} is in the schema");
    }
    assert_eq!(
        schema["required"],
        serde_json::json!(["language", "module"])
    );
    // the doc comments are what a front end's author reads
    assert!(schema["properties"]["language"]["description"]
        .as_str()
        .unwrap()
        .contains("rust"));
    let text = schema.to_string();
    for word in [
        "record",
        "enumeration",
        "interface",
        "function",
        "optional",
        "many",
        "Natural",
    ] {
        assert!(text.contains(word), "{word} is in the schema");
    }
}

/// A package checked as `sysml check` checks it: against the standard
/// library, every name resolved and then every constraint the
/// specification states asked -- the bar an imported package has to
/// clear, since it is the first thing a model is built on.
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

/// The one element of a kind under a name.
fn named(
    ws: &sysml_semantics::Workspace,
    kind: sysml_model::ElementKind,
    name: &str,
) -> sysml_model::ElementId {
    let model = ws.model();
    model
        .ids()
        .find(|&id| model.kind(id) == kind && model.name(id) == Some(name))
        .unwrap_or_else(|| panic!("no {kind:?} named {name}"))
}

/// A `doc` documents the element it is inside. Written as a sibling in
/// front of a definition, every definition's doc was the package's --
/// and a field's was the definition's, so a plan of the package read
/// the record as documented by its first field.
#[test]
fn a_doc_documents_what_it_was_written_for() {
    let surface = sysml_rust::surface::read(INVENTORY).unwrap();
    let sysml = sysml_rust::surface::to_sysml(&surface, None).sysml;
    let ws = checked(&sysml);
    use sysml_model::ElementKind as K;
    let doc = |kind, name| ws.documentation_of(named(&ws, kind, name));
    assert_eq!(doc(K::Package, "InventoryApi"), None, "{sysml}");
    assert_eq!(
        doc(K::ItemDefinition, "StockQuery").as_deref(),
        Some("What to look up. On two lines, with *\\/ inside."),
        "{sysml}"
    );
    assert_eq!(
        doc(K::AttributeUsage, "sku").as_deref(),
        Some("The article.")
    );
    assert_eq!(
        doc(K::EnumerationDefinition, "Region").as_deref(),
        Some("Where a warehouse is.")
    );
    assert_eq!(
        doc(K::PortDefinition, "InventoryStore").as_deref(),
        Some("The inventory service.")
    );
    assert_eq!(
        doc(K::ActionDefinition, "GetStock").as_deref(),
        Some("Look one article up.")
    );
    assert_eq!(
        doc(K::ReferenceUsage, "query").as_deref(),
        Some("What to look up.")
    );
}

/// A field holding a record is an item of it, and one holding an
/// interface a port: an attribute is a value, and the specification
/// does not let one own an occurrence. Written as attributes, a struct
/// holding another struct imported cleanly and failed `sysml check`.
#[test]
fn a_field_holding_a_record_is_an_item_and_checks_clean() {
    let surface = sysml_rust::surface::read(
        r#"{
        "language": "python",
        "module": "nest",
        "items": [
            { "kind": "record", "name": "Inner", "path": "nest.Inner",
              "fields": [ { "name": "v", "type": "Integer" } ] },
            { "kind": "interface", "name": "Store", "path": "nest.Store" },
            { "kind": "enumeration", "name": "Mode", "path": "nest.Mode", "values": ["on"] },
            { "kind": "record", "name": "Outer", "path": "nest.Outer",
              "fields": [
                  { "name": "inner", "type": "Inner" },
                  { "name": "many", "type": "Inner", "multiplicity": "many", "doc": "Several." },
                  { "name": "store", "type": "Store" },
                  { "name": "mode", "type": "Mode" },
                  { "name": "count", "type": "Integer" }
              ] }
        ]
    }"#,
    )
    .unwrap();
    let sysml = sysml_rust::surface::to_sysml(&surface, None).sysml;
    assert!(sysml.contains("\t\titem inner : Inner;\n"), "{sysml}");
    assert!(
        sysml.contains("\t\titem many : Inner[*] { doc /* Several. */ }\n"),
        "{sysml}"
    );
    assert!(sysml.contains("\t\tport store : Store;\n"), "{sysml}");
    assert!(sysml.contains("\t\tattribute mode : Mode;\n"), "{sysml}");
    assert!(
        sysml.contains("\t\tattribute count : Integer;\n"),
        "{sysml}"
    );
    checked(&sysml);
}

/// A member called `code` hides the binding's metadata definition from
/// everything inside the definition that declares it, `@code` among
/// them: its `:>> writtenIn` then resolved to nothing, and `import-code`
/// refused what it had written. There the binding names the definition
/// through the package -- and through the global namespace, where a
/// member hides the package as well.
#[test]
fn a_member_called_code_does_not_hide_the_binding() {
    let surface = sysml_rust::surface::read(
        r#"{
        "language": "rust",
        "module": "shadow",
        "items": [
            { "kind": "record", "name": "Err", "path": "shadow::Err",
              "fields": [ { "name": "code", "type": "Integer" } ] },
            { "kind": "record", "name": "Both", "path": "shadow::Both",
              "fields": [ { "name": "code", "type": "Integer" },
                          { "name": "ShadowApi", "type": "Integer" } ] },
            { "kind": "enumeration", "name": "Kind", "path": "shadow::Kind",
              "values": ["code", "other"] },
            { "kind": "function", "name": "send", "path": "shadow::send",
              "parameters": [ { "name": "code", "type": "Integer" } ] },
            { "kind": "record", "name": "Plain", "path": "shadow::Plain",
              "fields": [ { "name": "v", "type": "Integer" } ] },
            { "kind": "record", "name": "code", "path": "shadow::code" }
        ]
    }"#,
    )
    .unwrap();
    let imported = sysml_rust::surface::to_sysml(&surface, None);
    let sysml = &imported.sysml;
    assert!(
        sysml.contains("item def Err {\n\t\t@ShadowApi::code { :>> writtenIn"),
        "{sysml}"
    );
    assert!(
        sysml.contains("item def Both {\n\t\tmetadata code_ : $::ShadowApi::code {"),
        "{sysml}"
    );
    assert!(
        sysml.contains("enum def Kind {\n\t\t@ShadowApi::code {"),
        "{sysml}"
    );
    assert!(
        sysml.contains("action def Send {\n\t\t@ShadowApi::code {"),
        "{sysml}"
    );
    // where nothing hides it, the binding is written as it always was
    assert!(sysml.contains("item def Plain {\n\t\t@code {"), "{sysml}");
    // and a type under the metadata definition's own name is refused,
    // since every `@code` in the package would mean it
    assert!(!sysml.contains("item def code"), "{sysml}");
    assert!(
        imported
            .skipped
            .iter()
            .any(|line| line.starts_with("code -- the name of the metadata definition")),
        "{:?}",
        imported.skipped
    );
    let ws = checked(sysml);
    // and each binding is the package's own metadata definition
    let model = ws.model();
    let code = named(&ws, sysml_model::ElementKind::MetadataDefinition, "code");
    for def in ["Err", "Both", "Kind", "Send"] {
        let def = model
            .ids()
            .find(|&id| {
                model.name(id) == Some(def)
                    && model.kind(id).is_a(sysml_model::ElementKind::Definition)
            })
            .unwrap();
        let bound = model
            .owned(def)
            .iter()
            .filter(|&&child| model.kind(child) == sysml_model::ElementKind::MetadataUsage)
            .filter_map(|&child| model.type_of(child))
            .collect::<Vec<_>>();
        assert_eq!(bound, [code], "{sysml}");
    }
}

/// A parameter called `result` or `error` used to be written beside
/// the `out result` or `out error` the call's outputs are, two members
/// of one definition under one name. A generator reads the outputs by
/// those names and passes the inputs in order, so the parameter is the
/// one to give its name up -- and its doc says what it was called.
#[test]
fn a_parameter_does_not_take_an_output_s_name() {
    let surface = sysml_rust::surface::read(
        r#"{
        "language": "rust",
        "module": "outs",
        "items": [
            { "kind": "function", "name": "free", "path": "outs::free",
              "parameters": [ { "name": "result", "type": "Integer" },
                              { "name": "error", "type": "Integer" },
                              { "name": "result_", "type": "Integer" } ],
              "result": { "type": "Integer" } },
            { "kind": "function", "name": "fallible", "path": "outs::fallible",
              "fallible": true,
              "parameters": [ { "name": "error", "type": "Integer", "doc": "The code." } ],
              "result": { "type": "Integer" }, "error": "String" }
        ]
    }"#,
    )
    .unwrap();
    let sysml = sysml_rust::surface::to_sysml(&surface, None).sysml;
    // `error` is no output of `free`, and keeps its name
    assert!(
        sysml.contains(
            "\t\tin result__ : Integer { doc /* Called `result` in the code; `result` here is \
             what the call hands back. */ }\n\t\tin error : Integer;\n\t\tin result_ : Integer;\n\
             \t\tout result : Integer;\n"
        ),
        "{sysml}"
    );
    assert!(
        sysml.contains(
            "\t\tin error_ : Integer { doc /* The code. Called `error` in the code; `error` here \
             is what the call fails with. */ }\n\t\tout result : Integer;\n\t\tout error : String[0..1];\n"
        ),
        "{sysml}"
    );
    let ws = checked(&sysml);
    let model = ws.model();
    for def in ["Free", "Fallible"] {
        let def = named(&ws, sysml_model::ElementKind::ActionDefinition, def);
        let mut names: Vec<&str> = model
            .owned(def)
            .iter()
            .filter_map(|&child| model.name(child))
            .collect();
        let all = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all, "{sysml}");
    }
}

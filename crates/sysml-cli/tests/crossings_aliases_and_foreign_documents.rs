//! The corners of the notation writer and of the server that a document
//! from another tool, or a hand-edited one, is what reaches: the cross
//! multiplicity of an end and the name it is given, a `const end`
//! feature, a result that cannot be said, a name an import makes
//! ambiguous -- and the skin file a diagram is asked to be drawn in, and
//! the references a document makes to the library that it does not carry.

use std::collections::HashMap;
use std::io::Cursor;

use serde_json::{json, Value};
use sysml_cli::textual::{write_with, Written};
use sysml_model::ElementId;

/// The document `sysml export` writes for `text`, as JSON elements.
fn document(name: &str, text: &str) -> Vec<Value> {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file(name, text);
    ws.resolve_all();
    let mut extras = sysml_interchange::Extras::default();
    let ids: Vec<ElementId> = ws.model().ids().collect();
    for id in ids {
        if let Some(target) = ws.imported_alias(id).or_else(|| ws.import_of(id)) {
            extras.import_targets.insert(id, target);
        }
    }
    match sysml_interchange::to_json_with(ws.model(), &extras) {
        Value::Array(all) => all,
        other => panic!("a document is an array, not {other}"),
    }
}

/// A document, read the way `import_json` reads one, and written.
fn written_from(doc: &[Value], kept_text: bool) -> Written {
    let bytes = serde_json::to_vec(doc).unwrap();
    let (model, roots, _) = sysml_interchange::read_json(&bytes).unwrap();
    write_with(&model, &roots, &HashMap::new(), kept_text)
}

fn written(name: &str, text: &str) -> String {
    written_from(&document(name, text), true).sysml
}

/// Each of `said` is a line of `sysml`, as written.
fn says(sysml: &str, said: &[&str]) {
    for line in said {
        assert!(
            sysml.lines().any(|it| it.trim() == *line),
            "no line `{line}` in\n{sysml}"
        );
    }
}

// ------------------------------------------------------------ notation

/// A result the notation has no way to say is left out, and said to be.
#[test]
fn the_result_of_a_calculation_that_cannot_be_said_is_left_out() {
    let mut doc = document(
        "m.sysml",
        "package P { calc def C { in x : Integer; x + 1 } }",
    );
    let sum = doc
        .iter()
        .position(|it| it["@type"] == "OperatorExpression")
        .unwrap();
    doc[sum]["@type"] = json!("InvocationExpression");
    doc[sum].as_object_mut().unwrap().remove("operator");
    let w = written_from(&doc, false);
    says(&w.sysml, &["in x;", "// not written: Expression"]);
    assert_eq!(w.unwritten, [("Expression".to_string(), "C".to_string())]);
}

/// A KerML feature is `const` and an `end` at once, and a classifier has
/// a short name as a definition has.
#[test]
fn a_kerml_end_feature_is_a_constant_end_and_a_classifier_has_its_short_name() {
    let sysml = written(
        "m.kerml",
        "package P { classifier <s> C { end const feature e; } class <t> D; }",
    );
    says(
        &sysml,
        &["classifier <s> C {", "const end feature e;", "class <t> D;"],
    );
}

const CONNECTED: &str = "package P { part def A; part def S { part a : A; part b : A; \
    connection m : M connect [1] a to [0..1] b; \
    connection def M { end x : A; end y : A; } } }";

/// The multiplicity a connector's end carries on the far side of it is
/// written in front of the end.
#[test]
fn the_cross_multiplicity_of_a_connectors_end_is_written_in_front_of_it() {
    let sysml = written("m.sysml", CONNECTED);
    says(&sysml, &["connection m : M connect [1] a to [0..1] b;"]);
}

/// A cross feature that has a name has it said before the multiplicity.
#[test]
fn a_cross_feature_that_has_a_name_says_it_before_its_multiplicity() {
    let mut doc = document("m.sysml", CONNECTED);
    let range = doc
        .iter()
        .position(|it| it["@type"] == "MultiplicityRange")
        .unwrap();
    let owner = doc[range]["owningNamespace"]["@id"].clone();
    let cross = doc.iter().position(|it| it["@id"] == owner).unwrap();
    doc[cross]["declaredName"] = json!("inCart");
    doc[cross]["name"] = json!("inCart");
    let w = written_from(&doc, true);
    says(
        &w.sysml,
        &["connection m : M connect inCart [1] a to [0..1] b;"],
    );
    assert!(w.unwritten.is_empty(), "{:?}", w.unwritten);
}

/// A succession is written `then b;` from the step before it, passing
/// over what is neither a step nor an occurrence on the way.
#[test]
fn a_succession_runs_from_the_step_before_it_past_an_attribute_between() {
    let sysml = written(
        "m.sysml",
        "package P { action a { action a1; attribute w; first a1 then b; action b; } }",
    );
    says(&sysml, &["action a1;", "attribute w;", "then action b;"]);
}

/// An import of another element of the same name makes the name no
/// longer enough to refer to the one that is meant.
#[test]
fn a_name_that_a_membership_import_gives_to_another_element_is_qualified() {
    let sysml = written(
        "m.sysml",
        "package P { part def x; package Q { part def x; } \
         package R { import Q::x; part y : P::x; } }",
    );
    says(&sysml, &["private import Q::x;", "part y : P::x;"]);
    // an import of another name leaves the name alone
    let sysml = written(
        "m.sysml",
        "package P { part def x; package Q { part def z; } \
         package R { import Q::z; part y : x; } }",
    );
    says(&sysml, &["private import Q::z;", "part y : x;"]);
    // and so does one that names no membership at all
    let mut doc = document(
        "m.sysml",
        "package P { part def x; package Q { part def z; } \
         package R { import Q::z; part y : x; } }",
    );
    let import = doc
        .iter()
        .position(|it| it["@type"] == "MembershipImport")
        .unwrap();
    doc[import]
        .as_object_mut()
        .unwrap()
        .remove("importedMembership");
    let w = written_from(&doc, true);
    says(
        &w.sysml,
        &["part y : x;", "// not written: MembershipImport"],
    );
}

// -------------------------------------------------------------- server

fn drive(mut server: sysml_cli::mcp::Server, lines: &[Value]) -> Vec<Value> {
    let input: String = lines.iter().map(|line| format!("{line}\n")).collect();
    let mut out: Vec<u8> = Vec::new();
    sysml_cli::mcp::serve(&mut server, Cursor::new(input.as_bytes()), &mut out).unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).expect("every answer is JSON"))
        .collect()
}

fn call(name: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
    })
}

fn answered(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("a text content: {response}"));
    serde_json::from_str(text).expect("the content is JSON")
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// What a skin file that says nothing usable is refused with.
#[test]
fn a_skin_file_that_is_not_json_or_not_a_skin_is_refused_for_what_it_is() {
    let dir = scratch("skin-files");
    let prose = dir.join("prose.skin");
    std::fs::write(&prose, "paint it blue, please").unwrap();
    let odd = dir.join("odd.skin");
    std::fs::write(&odd, r#"{"nope": 1}"#).unwrap();
    let answers = drive(
        sysml_cli::mcp::Server::without_library(None),
        &[
            call(
                "diagram",
                json!({ "text": "package P { part def A; }", "skin": prose.to_str().unwrap() }),
            ),
            call(
                "diagram",
                json!({ "text": "package P { part def A; }", "skin": odd.to_str().unwrap() }),
            ),
        ],
    );
    for it in &answers {
        assert_eq!(it["result"]["isError"], true, "{it}");
    }
    let missing = drive(
        sysml_cli::mcp::Server::without_library(None),
        &[call(
            "diagram",
            json!({ "text": "package P { part def A; }", "skin": "no-such-skin-or-file" }),
        )],
    );
    let gone = answered(&missing[0])["error"].as_str().unwrap().to_string();
    assert!(
        gone.starts_with("no skin `no-such-skin-or-file`, and cannot read `no-such-skin-or-file`"),
        "{gone}"
    );
    let first = answered(&answers[0])["error"].as_str().unwrap().to_string();
    assert!(
        first.starts_with(&format!("`{}` is not JSON: ", prose.display())),
        "{first}"
    );
    let second = answered(&answers[1])["error"].as_str().unwrap().to_string();
    assert!(
        second.starts_with(&format!("{}: ", odd.display())),
        "{second}"
    );
    assert!(!second.contains("is not JSON"), "{second}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The library as a server with it built in publishes it: the alias
/// membership of `ISQElectromagnetism` and the feature it is for.
fn an_alias_and_what_it_is_for() -> (Value, Value) {
    let said = answered(
        &drive(
            sysml_cli::mcp::Server::new(None),
            &[call(
                "export_json",
                json!({
                    "text": "package P { }", "include_library": true,
                    "within": "ISQElectromagnetism", "max_bytes": 64_000_000,
                }),
            )],
        )[0],
    );
    let all = said["document"].as_array().expect("a document");
    let alias = all
        .iter()
        .find(|it| it["memberName"] == "volumicElectricCharge")
        .expect("the alias");
    (alias["@id"].clone(), alias["memberElement"]["@id"].clone())
}

/// A document that refers to the library by a property its reader
/// resolves though it is derived -- the `referent` of an expression --
/// is read all the same, the element it names standing for the
/// library's. A reference to an alias is to the alias, named as it is
/// there.
#[test]
fn a_document_naming_the_library_by_a_derived_property_is_read_with_it_stood_in() {
    let (alias, target) = an_alias_and_what_it_is_for();
    let referring = |to: &Value| {
        json!([
            {"@type":"Package","@id":"p","declaredName":"P","ownedRelationship":[{"@id":"m"}]},
            {"@type":"OwningMembership","@id":"m","ownedRelatedElement":[{"@id":"a"}]},
            {"@type":"AttributeUsage","@id":"a","declaredName":"a","ownedRelationship":[{"@id":"fv"}]},
            {"@type":"FeatureValue","@id":"fv","ownedRelatedElement":[{"@id":"e"}]},
            {"@type":"FeatureReferenceExpression","@id":"e","referent":{"@id":to}}
        ])
    };
    let answers = drive(
        sysml_cli::mcp::Server::new(None),
        &[
            call("import_json", json!({ "json": referring(&target) })),
            call("import_json", json!({ "json": referring(&alias) })),
            call(
                "import_json",
                json!({ "json": referring(&json!("nobody-has-this")) }),
            ),
        ],
    );
    let feature = answered(&answers[0]);
    assert_eq!(feature["library_references"], 1, "{feature}");
    assert_eq!(
        feature["library"],
        json!(["ISQElectromagnetism::electricChargeDensity"]),
        "{feature}"
    );
    let aliased = answered(&answers[1]);
    assert_eq!(
        aliased["library"],
        json!(["ISQElectromagnetism::volumicElectricCharge"]),
        "{aliased}"
    );
    // and what nothing answers to is still refused, by its id
    assert_eq!(answers[2]["result"]["isError"], true, "{}", answers[2]);
    let why = answered(&answers[2])["error"].as_str().unwrap().to_string();
    assert!(
        why.contains("the document names element nobody-has-this"),
        "{why}"
    );
}

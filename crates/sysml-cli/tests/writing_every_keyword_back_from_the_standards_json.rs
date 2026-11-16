//! What `import_json` writes back, for the declarations and expressions
//! the notation has a word for and the readers of a document least often
//! meet.

use std::io::Cursor;

use serde_json::{json, Value};

fn session(lines: &[Value]) -> Vec<Value> {
    let input: String = lines.iter().map(|line| format!("{line}\n")).collect();
    let mut server = sysml_cli::mcp::Server::without_library(None);
    let mut out: Vec<u8> = Vec::new();
    sysml_cli::mcp::serve(&mut server, Cursor::new(input.as_bytes()), &mut out).unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).expect("every answer is JSON"))
        .collect()
}

fn answered(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("a text content");
    serde_json::from_str(text).expect("the content is JSON")
}

fn call(name: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
    })
}

/// `import_json`'s answer for a document, as a program reads it.
fn import(document: Value) -> Value {
    answered(&session(&[call("import_json", json!({ "json": document }))])[0])
}

/// The text exported from `model`, read back and written.
fn back(model: &str) -> Value {
    back_as("model.sysml", model)
}

/// As [`back`], from a document that kept no text of its expressions.
fn back_without_text(model: &str) -> Value {
    let exported = answered(&session(&[call("export_json", json!({ "text": model }))])[0]);
    import(without_text(exported["document"].clone()))
}

fn sysml_of(read: &Value) -> String {
    read["sysml"].as_str().expect("the notation").to_string()
}

fn has_all(sysml: &str, lines: &[&str]) {
    for line in lines {
        assert!(sysml.contains(line), "no `{line}` in\n{sysml}");
    }
}

fn without_text(document: Value) -> Value {
    let Value::Array(objects) = document else {
        panic!("a document is an array");
    };
    let mut dropped: Vec<String> = Vec::new();
    for object in &objects {
        if object["@type"] == "TextualRepresentation" {
            dropped.push(object["@id"].as_str().unwrap().to_string());
            if let Some(owner) = object["owningMembership"]["@id"].as_str() {
                dropped.push(owner.to_string());
            }
        }
    }
    fn strip(value: &mut Value, dropped: &[String]) {
        match value {
            Value::Array(items) => {
                items.retain(|it| {
                    !it["@id"]
                        .as_str()
                        .is_some_and(|id| dropped.iter().any(|d| d == id))
                });
                items.iter_mut().for_each(|it| strip(it, dropped));
            }
            Value::Object(fields) => fields.values_mut().for_each(|it| strip(it, dropped)),
            _ => {}
        }
    }
    let mut kept: Vec<Value> = objects
        .into_iter()
        .filter(|it| {
            !it["@id"]
                .as_str()
                .is_some_and(|id| dropped.iter().any(|d| d == id))
        })
        .collect();
    kept.iter_mut().for_each(|it| strip(it, &dropped));
    Value::Array(kept)
}

fn back_as(name: &str, model: &str) -> Value {
    let exported =
        answered(&session(&[call("export_json", json!({ "text": model, "name": name }))])[0]);
    assert!(exported["document"].is_array(), "{exported}");
    import(exported["document"].clone())
}

const KEYWORDS: &str = "package K {
    occurrence def Oc; occurrence oc : Oc;
    item def It; item it : It;
    enum def En { enum a; enum b; }
    ref r : It;
    connection def Cd; connection cd : Cd;
    interface def Id; interface i1 : Id;
    allocation def Ad; allocation al : Ad;
    flow def Fd;
    action def Act { action x; }
    state def St; state s : St;
    constraint def Cn; constraint cu : Cn;
    requirement def Rq; requirement rq : Rq;
    concern def Cc; concern cc : Cc;
    calc def Cl; calc cl : Cl;
    case def Cs; case cs : Cs;
    analysis def An; analysis an : An;
    verification def Vf; verification vf : Vf;
    use case def Uc; use case uc : Uc;
    view def Vw; view vw : Vw;
    viewpoint def Vp; viewpoint vp : Vp;
    rendering def Rd; rendering rd : Rd;
    metadata def Md;
    port def Pd; port pp : Pd;
}";

#[test]
fn each_metaclass_the_notation_has_a_word_for_is_written_with_that_word() {
    let read = back(KEYWORDS);
    let sysml = sysml_of(&read);
    has_all(
        &sysml,
        &[
            "occurrence def Oc;",
            "occurrence oc : Oc;",
            "item def It;",
            "item it : It;",
            "enum def En {",
            "ref r : It;",
            "connection def Cd;",
            "connection cd : Cd;",
            "interface def Id;",
            "interface i1 : Id;",
            "allocation def Ad;",
            "allocation al : Ad;",
            "flow def Fd;",
            "action def Act {",
            "state def St;",
            "state s : St;",
            "constraint def Cn;",
            "constraint cu : Cn;",
            "requirement def Rq;",
            "requirement rq : Rq;",
            "concern def Cc;",
            "concern cc : Cc;",
            "calc def Cl;",
            "calc cl : Cl;",
            "case def Cs;",
            "case cs : Cs;",
            "analysis def An;",
            "analysis an : An;",
            "verification def Vf;",
            "verification vf : Vf;",
            "use case def Uc;",
            "use case uc : Uc;",
            "view def Vw;",
            "view vw : Vw;",
            "viewpoint def Vp;",
            "viewpoint vp : Vp;",
            "rendering def Rd;",
            "rendering rd : Rd;",
            "metadata def Md;",
            "port def Pd;",
            "port pp : Pd;",
        ],
    );
    assert_eq!(read["unwritten"], json!([]), "{read}");
}

const STATEMENTS: &str = "package K {
    action def A1 { action step1; action step2; }
    part def Eng { perform action go : A1; exhibit state sm : S1; }
    state def S1;
    action a2 {
        then decide d;
        then m1;
        action m1;
        merge mg;
        fork fk;
        join jn;
    }
    constraint def C1;
    requirement def R1 { assert constraint c : C1; }
    use case def U1 { include use case i : U2; }
    use case def U2;
    part def P1 { event occurrence ev; flow f1 from a to b; ref a; ref b; }
    metadata def service;
    #service def Svc;
    interface def Idef { end e1; end e2; }
    part q {
        port p1; port p2;
        interface ic : Idef connect e1 ::> p1 to e2 ::> p2;
        connection cn connect p1 to p2;
        allocation al allocate p1 to p2;
    }
}";

#[test]
fn performances_control_nodes_and_connectors_are_written_with_their_own_words() {
    let sysml = sysml_of(&back(STATEMENTS));
    has_all(
        &sysml,
        &[
            "perform action go : A1;",
            "exhibit state sm : S1;",
            "decide d;",
            "merge mg;",
            "fork fk;",
            "join jn;",
            "assert constraint c : C1;",
            "include use case i : U2;",
            "event occurrence ev;",
            "flow f1 from a to b;",
            "#service def Svc;",
            "interface ic : Idef connect e1 ::> p1 to e2 ::> p2;",
            "connection cn connect p1 to p2;",
            "allocation al allocate p1 to p2;",
        ],
    );
}

const KERML: &str = "package K {
    namespace Ns;
    type T1;
    classifier C1;
    class Cl1;
    datatype D1;
    struct S1;
    assoc As1;
    assoc struct AS2;
    behavior B1;
    function F1;
    predicate Pr1;
    interaction I1;
    metaclass M1;
    feature f1;
    feature f2;
    step st1;
    expr e1;
    bool b1;
    inv v1;
    connector cn1;
    succession sc1 first st1 then st1;
    flow fl1;
    succession flow sf1;
    specialization Gen subtype C1 specializes Cl1;
    disjoint C1 from Cl1;
    conjugation cj conjugate C1 conjugates Cl1;
    featuring ft of f1 by C1;
    binding bd2;
    succession sc2;
    inverse f1 of f2;
    subset f1 subsets f2;
}";

#[test]
fn a_model_of_classes_and_features_is_written_in_kerml_with_its_own_words() {
    let read = back_as("m.kerml", KERML);
    assert_eq!(read["dialect"], "kerml", "{read}");
    let sysml = sysml_of(&read);
    has_all(
        &sysml,
        &[
            "namespace Ns;",
            "type T1;",
            "classifier C1;",
            "class Cl1;",
            "datatype D1;",
            "struct S1;",
            "assoc As1;",
            "assoc struct AS2;",
            "behavior B1;",
            "function F1;",
            "predicate Pr1;",
            "interaction I1;",
            "metaclass M1;",
            "feature f1;",
            "step st1;",
            "expr e1;",
            "bool b1;",
            "inv v1;",
            "connector cn1;",
            "succession sc1 first st1 then st1;",
            "flow fl1;",
            "succession flow sf1;",
            "specialization Gen subtype C1 specializes Cl1;",
            "disjoint C1 from Cl1;",
            "conjugation cj conjugate C1 conjugates Cl1;",
            "featuring ft of f1 by C1;",
            "binding bd2;",
            "succession sc2;",
            "inverse f1 of f2;",
            "subset f1 subsets f2;",
        ],
    );
}

const EXPRESSIONS: &str = "package K {
    attribute def Real;
    part def Vehicle { attribute speed; attribute xs; attribute flag; attribute fl; }
    part v : Vehicle {
        attribute a = xs#(1);
        attribute b = v istype Vehicle;
        attribute c = v as Vehicle;
        attribute d = v @ Vehicle;
        attribute g = 2.5;
        attribute h = 1e3;
        attribute j = xs->select {in x; x > 1};
        attribute m = (1 , (2, 3));
        attribute o = v.speed;
        attribute p = not flag;
        attribute q = flag and fl;
        attribute r = all Vehicle;
        attribute s = Vehicle meta Vehicle;
        attribute t = - 1;
        attribute u = if flag ? 1 else 2;
        attribute w = *;
        attribute x = true;
        attribute z = null;
    }
}";

#[test]
fn an_expression_is_written_from_what_it_is_when_the_document_kept_no_text_of_it() {
    let read = back_without_text(EXPRESSIONS);
    let sysml = sysml_of(&read);
    has_all(
        &sysml,
        &[
            "attribute a = (Vehicle::xs)#(1);",
            "attribute b = (v istype Vehicle);",
            "attribute c = (v as Vehicle);",
            "attribute d = (v @ Vehicle);",
            "attribute g = 2.5;",
            "attribute h = 1000.0;",
            "attribute j = (Vehicle::xs)->select { in x; (x > 1) };",
            "attribute m = (1, 2, 3);",
            "attribute p = (not Vehicle::flag);",
            "attribute r = all Vehicle;",
            "attribute s = (Vehicle meta Vehicle);",
            "attribute t = -1;",
            "attribute u = (if Vehicle::flag ? 1 else 2);",
            "attribute w = *;",
            "attribute x = true;",
            "attribute z = null;",
        ],
    );
}

/// The document `model` exports, with each of its objects altered by `change`.
fn exported_then(model: &str, change: impl Fn(&mut serde_json::Map<String, Value>)) -> Value {
    exported_with(model, |objects| {
        for object in objects {
            change(object.as_object_mut().unwrap());
        }
    })
}

/// The same, for a change that adds to the document or looks across it.
fn exported_with(model: &str, change: impl FnOnce(&mut Vec<Value>)) -> Value {
    let exported = answered(&session(&[call("export_json", json!({ "text": model }))])[0]);
    let Value::Array(mut objects) = without_text(exported["document"].clone()) else {
        panic!("a document is an array");
    };
    change(&mut objects);
    import(Value::Array(objects))
}

fn id_of(objects: &[Value], name: &str) -> String {
    objects
        .iter()
        .find(|it| it["declaredName"] == name)
        .unwrap_or_else(|| panic!("no `{name}`"))["@id"]
        .as_str()
        .unwrap()
        .to_string()
}

const ALTERED: &str = "package K {
    part def Vehicle { attribute x; attribute y; attribute f; }
    part v : Vehicle {
        attribute a = x + y;
        attribute b = if f ? 1 else 2;
        attribute c = -x;
        attribute d = 2.5;
        attribute e = true;
    }
}";

#[test]
fn an_expression_the_document_alters_is_written_as_what_it_has_become_or_not_at_all() {
    let read = exported_then(ALTERED, |object| {
        let operator = object
            .get("operator")
            .and_then(Value::as_str)
            .map(String::from);
        match (object["@type"].as_str().unwrap(), operator.as_deref()) {
            ("OperatorExpression", Some("+")) => {
                object.insert("operator".into(), json!("as"));
            }
            ("OperatorExpression", Some("if")) => {
                object.insert("operator".into(), json!("+"));
            }
            ("OperatorExpression", Some("-")) => {
                object.insert("@type".into(), json!("TriggerInvocationExpression"));
                object.insert("kind".into(), json!("at"));
            }
            ("LiteralRational", _) => {
                object.insert("value".into(), json!(3));
            }
            ("LiteralBoolean", _) => {
                object.insert("@type".into(), json!("LiteralString"));
            }
            _ => {}
        }
    });
    let sysml = sysml_of(&read);
    has_all(
        &sysml,
        &[
            "attribute c = at Vehicle::x;",
            "attribute d = 3.0;",
            "attribute a;",
            "attribute b;",
            "attribute e;",
        ],
    );
    // `x + y` that became a classification of two things, `if` that became
    // an addition of three, a true that became a string with no text: none of them is a thing the notation says
    let unwritten: Vec<(&str, &str)> = read["unwritten"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| (it["kind"].as_str().unwrap(), it["in"].as_str().unwrap()))
        .collect();
    assert_eq!(
        unwritten,
        vec![
            ("expression", "a"),
            ("expression", "b"),
            ("expression", "e")
        ],
        "{sysml}"
    );
}

#[test]
fn a_chain_the_document_holds_only_as_chainings_is_still_written_as_a_chain() {
    let model = "package P { part a { part b; } part c :> a.b; connection cc connect a.b to c; }";
    // a document whose features do not list the features they chain, as
    // the standard derives them, and say it only by what they own
    let read = exported_then(model, |object| {
        if object["@type"] == "Feature" {
            object.remove("chainingFeature");
        }
    });
    let sysml = sysml_of(&read);
    has_all(
        &sysml,
        &["part c :> a.b;", "connection cc connect a.b to c;"],
    );
    // and one whose chaining names nothing writes no chain
    let read = exported_then(model, |object| {
        if object["@type"] == "Feature" {
            object.remove("chainingFeature");
        }
        if object["@type"] == "FeatureChaining" {
            object.insert("chainingFeature".into(), json!(null));
        }
    });
    let sysml = sysml_of(&read);
    assert!(sysml.contains("part c;"), "{sysml}");
    assert!(!sysml.contains("part c :>"), "{sysml}");
}

const PORTS_AND_FLOWS: &str = "library package LP { part def Base; }
package K {
    protected part def Hidden;
    port def PD { port inner; ref port held; }
    part src { out item o; }
    part dst { in item i; }
    flow flowed from src.o to dst.i;
    succession flow sflow from src.o to dst.i;
    flow;
    succession flow;
}";

#[test]
fn ports_in_ports_a_protected_member_and_flows_without_ends_are_written_as_they_stand() {
    let read = back(PORTS_AND_FLOWS);
    let sysml = sysml_of(&read);
    has_all(
        &sysml,
        &[
            "library package LP {",
            "protected part def Hidden;",
            "port inner;",
            "ref port held;",
            "flow flowed from src.o to dst.i;",
            "succession flow sflow from src.o to dst.i;",
            "    flow;",
        ],
    );
    // a succession flow that flows from nothing to nothing says nothing
    assert_eq!(
        read["unwritten"],
        json!([{"kind": "SuccessionFlowUsage", "in": "K"}]),
        "{sysml}"
    );
}

/// What is not written, at the top of a document, is in no namespace
/// that has a name to say.
#[test]
fn what_is_not_written_at_the_top_of_a_document_is_said_to_be_at_the_top() {
    let read = import(json!([{ "@type": "NullExpression", "@id": "n" }]));
    let sysml = sysml_of(&read);
    assert!(sysml.contains("// not written: NullExpression"), "{sysml}");
    assert_eq!(
        read["unwritten"],
        json!([{"kind": "NullExpression", "in": "the top level"}]),
        "{read}"
    );
}

const NAMES: &str = "package Lib { part def Wheel; part def Axle; }
package P {
    import Lib::Wheel;
    part def Base;
    part def Left :> Base;
    part def Right :> Base;
    part def Both :> Left, Right {
        part x : Lib::Axle;
        part def Inner { part def T; }
        part r :> x { part z : Inner::T; }
    }
    part def Q { part :>> w { part k : Lib::Axle; } part w : Wheel; }
    part def S :> Q { part v :>> w { part u : Lib::Axle; } }
}";

#[test]
fn a_name_is_found_from_where_it_is_written_through_what_is_specialized_and_redefined() {
    let read = back(NAMES);
    let sysml = sysml_of(&read);
    has_all(
        &sysml,
        &[
            "private import Lib::Wheel;",
            "part def Both :> Left, Right {",
            "part x : Lib::Axle;",
            "part z : Inner::T;",
            "part k : Lib::Axle;",
            "part u : Lib::Axle;",
        ],
    );
    assert_eq!(read["unwritten"], json!([]), "{read}");
}

#[test]
fn a_classification_handed_two_things_to_classify_is_not_written() {
    let read = exported_with(ALTERED, |objects| {
        let vehicle = id_of(objects, "Vehicle");
        for object in objects.iter_mut() {
            if object["operator"] == "+" {
                object["operator"] = json!("as");
                object["ownedRelationship"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"@id": "classified-by"}));
            }
        }
        objects.push(json!({
            "@type": "Membership", "@id": "classified-by",
            "memberElement": {"@id": vehicle},
        }));
    });
    let sysml = sysml_of(&read);
    // the rest of what was in the document is written as it was
    has_all(
        &sysml,
        &[
            "attribute a;",
            "attribute b = (if Vehicle::f ? 1 else 2);",
            "attribute c = (-Vehicle::x);",
        ],
    );
    assert_eq!(
        read["unwritten"],
        json!([{"kind": "expression", "in": "a"}])
    );
}

/// A package of the model's own called what the library's first segment is
/// called hides the library's: it is named from `$::`.
#[test]
fn a_library_name_past_a_package_of_the_same_first_name_is_written_from_the_root() {
    let model = "package P {
    package Decoy;
    part def Q { attribute m : ScalarValues::Real; }
}";
    let input: String = [call("export_json", json!({ "text": model }))]
        .iter()
        .map(|line| format!("{line}\n"))
        .collect();
    let mut server = sysml_cli::mcp::Server::new(None);
    let mut out: Vec<u8> = Vec::new();
    sysml_cli::mcp::serve(&mut server, Cursor::new(input.as_bytes()), &mut out).unwrap();
    let answer: Value =
        serde_json::from_str(String::from_utf8(out).unwrap().lines().next().unwrap()).unwrap();
    let Value::Array(mut objects) = answered(&answer)["document"].clone() else {
        panic!("a document is an array");
    };
    for object in &mut objects {
        if object["declaredName"] == "Decoy" {
            object["declaredName"] = json!("ScalarValues");
            object["name"] = json!("ScalarValues");
        }
    }
    let input = format!(
        "{}\n",
        call("import_json", json!({ "json": Value::Array(objects) }))
    );
    let mut server = sysml_cli::mcp::Server::new(None);
    let mut out: Vec<u8> = Vec::new();
    sysml_cli::mcp::serve(&mut server, Cursor::new(input.as_bytes()), &mut out).unwrap();
    let read = answered(
        &serde_json::from_str(String::from_utf8(out).unwrap().lines().next().unwrap()).unwrap(),
    );
    let sysml = sysml_of(&read);
    has_all(
        &sysml,
        &[
            "package ScalarValues;",
            "attribute m : $::ScalarValues::Real;",
        ],
    );
}

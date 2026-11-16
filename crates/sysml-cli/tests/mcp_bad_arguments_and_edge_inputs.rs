//! The server at its edges: the argument that is the wrong kind of thing,
//! the model that gives a tool nothing to draw or follow, the resource
//! asked for by a URI that names none, and the process whose stdout is
//! gone. Each answer is checked for what it says, since these are the
//! answers a client has only the words of to go by.

use std::io::{Cursor, Write};

use serde_json::{json, Value};

/// Drive a server over the lines given and read the answers back.
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

/// A server with no library: the protocol and the shape of an answer, not
/// a library name, is what most of these are about.
fn bare(lines: &[Value]) -> Vec<Value> {
    drive(sysml_cli::mcp::Server::without_library(None), lines)
}

/// A server with the library built into the binary.
fn built_in(lines: &[Value]) -> Vec<Value> {
    drive(sysml_cli::mcp::Server::new(None), lines)
}

fn call(name: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
    })
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// The JSON a tool answered with.
fn answered(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("a text content: {response}"));
    serde_json::from_str(text).expect("the content is JSON")
}

/// What a refused call said, having checked that it was a refusal.
fn refused(response: &Value) -> String {
    assert_eq!(response["result"]["isError"], true, "{response}");
    answered(response)["error"].as_str().unwrap().to_string()
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------- outline

/// A package of `count` definitions, big enough in an outline that two of
/// them are more than a page is allowed to hold.
fn crowded(name: &str, count: usize) -> String {
    let mut text = format!("package {name} {{\n");
    for i in 0..count {
        text.push_str(&format!("\tpart def D{i};\n"));
    }
    text.push_str("}\n");
    text
}

#[test]
fn an_outline_too_large_for_one_answer_is_cut_and_says_where_to_go_on() {
    let text = format!("{}{}", crowded("A", 700), crowded("B", 700));
    let answers = bare(&[
        call("outline", json!({ "text": text, "depth": 2 })),
        call("outline", json!({ "text": text, "depth": 2, "offset": 1 })),
    ]);
    let first = answered(&answers[0]);
    // the first element is always answered, however large
    assert_eq!(first["elements"].as_array().unwrap().len(), 1, "{first}");
    assert_eq!(first["elements"][0]["name"], "A");
    assert_eq!(first["total"], 2);
    assert_eq!(first["truncated"], true);
    assert_eq!(first["next_offset"], 1);
    let note = first["note"].as_str().unwrap();
    assert!(note.contains("cut at 1 of 2 top-level elements"), "{note}");
    assert!(note.contains("`offset: 1`"), "{note}");
    // and what was asked for next is the rest, with nothing left over
    let rest = answered(&answers[1]);
    assert_eq!(rest["elements"][0]["name"], "B", "{rest}");
    assert!(rest["next_offset"].is_null(), "{rest}");
}

// ------------------------------------------------------------ definition

#[test]
fn definition_says_a_short_name_and_what_an_alias_stands_for() {
    let text = "package P { part def <v1> Vehicle; alias Rig for Vehicle; }";
    let answers = bare(&[
        call("definition", json!({ "text": text, "element": "v1" })),
        call("definition", json!({ "text": text, "element": "P::Rig" })),
    ]);
    let short = answered(&answers[0]);
    assert_eq!(short["element"], "P::Vehicle", "{short}");
    assert_eq!(short["short_name"], "v1");
    let alias = answered(&answers[1]);
    assert_eq!(alias["alias_for"], "P::Vehicle", "{alias}");
    assert!(alias["short_name"].is_null(), "{alias}");
}

#[test]
fn a_name_asked_of_a_server_that_holds_nothing_is_nothing() {
    let answers = bare(&[call("definition", json!({ "text": "", "element": "P" }))]);
    let why = refused(&answers[0]);
    assert!(why.contains("nothing is named `P`"), "{why}");
}

// ---------------------------------------------------------------- rename

#[test]
fn a_rename_asked_by_short_name_edits_the_short_name_it_gives_up() {
    let text = "package P { part def <v1> Vehicle; part x : v1; }";
    let answers = bare(&[call(
        "rename",
        json!({ "text": text, "element": "v1", "new_name": "w1" }),
    )]);
    let said = answered(&answers[0]);
    assert_eq!(said["element"], "P::Vehicle", "{said}");
    let edits = said["files"][0]["edits"].as_array().unwrap();
    // the declaration `<v1>` and the use `: v1`, not the name `Vehicle`
    assert_eq!(edits.len(), 2, "{said}");
    assert_eq!(edits[0]["offset"], 22, "{said}");
    assert!(edits.iter().all(|it| it["new_text"] == "w1"), "{said}");
    assert_eq!(said["checked"]["ok"], true, "{said}");
}

#[test]
fn a_rename_of_a_model_that_does_not_parse_says_so_in_what_it_checked() {
    let text = "package P { part def A; part x : A; } part def {";
    let answers = bare(&[call(
        "rename",
        json!({ "text": text, "element": "P::A", "new_name": "B" }),
    )]);
    let said = answered(&answers[0]);
    assert_eq!(said["edits"], 2, "{said}");
    assert_eq!(said["checked"]["ok"], false, "{said}");
    let errors = said["checked"]["parse_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "{said}");
    assert_eq!(errors[0]["path"], "model.sysml");
    assert!(errors[0]["message"].as_str().unwrap().contains("EOF"));
}

#[test]
fn the_root_namespace_is_written_nowhere_so_it_is_no_rename() {
    let answers = bare(&[call(
        "rename",
        json!({ "text": "package P;", "element": "", "new_name": "B" }),
    )]);
    assert_eq!(
        refused(&answers[0]),
        "`` is not written anywhere, so there is no name to rename"
    );
}

// --------------------------------------------------------------- diagram

const DRAWN: &str = "package P { part def A; }";

#[test]
fn diagram_takes_a_skin_as_an_object_and_refuses_what_is_none() {
    let answers = bare(&[
        call("diagram", json!({ "text": DRAWN, "skin": {} })),
        call("diagram", json!({ "text": DRAWN, "skin": 5 })),
        call("diagram", json!({ "text": DRAWN, "skin": { "nope": 1 } })),
        call("diagram", json!({ "text": DRAWN, "image": 5 })),
    ]);
    let drawn = answered(&answers[0]);
    assert_eq!(drawn["drawn"]["boxes"], json!(["P::A"]), "{drawn}");
    assert!(drawn["svg"].as_str().unwrap().starts_with("<svg"));
    let why = refused(&answers[1]);
    assert!(why.contains("the skin itself as an object"), "{why}");
    let why = refused(&answers[2]);
    assert!(why.starts_with("`skin`: "), "{why}");
    assert_eq!(refused(&answers[3]), "`image` must be true or false");
}

#[test]
fn a_drawing_of_a_model_that_does_not_parse_says_what_it_is_a_guess_at() {
    let answers = bare(&[call(
        "diagram",
        json!({ "text": "package P { part def A; } part def {" }),
    )]);
    let drawn = answered(&answers[0]);
    let warning = drawn["warnings"][0].as_str().unwrap();
    assert!(
        warning.contains("model.sysml does not parse, so what is drawn of it is a guess"),
        "{warning}"
    );
}

#[test]
fn a_view_is_of_what_it_is_asked_within_and_says_when_it_has_nothing() {
    let answers = bare(&[
        call("diagram", json!({ "text": DRAWN, "within": "P::A" })),
        call("diagram", json!({ "text": "package P { part x; }" })),
        call(
            "diagram",
            json!({ "text": DRAWN, "view": "internal", "element": "P::A" }),
        ),
        call("diagram", json!({ "text": "", "view": "browser" })),
        call(
            "diagram",
            json!({ "text": DRAWN, "view": "browser", "within": "P" }),
        ),
    ]);
    let within = answered(&answers[0]);
    assert_eq!(within["drawn"]["boxes"], json!(["P::A"]), "{within}");
    assert!(refused(&answers[1]).contains("there are no definitions here"));
    assert!(refused(&answers[2]).contains("no parts or connections to draw"));
    assert_eq!(refused(&answers[3]), "nothing to draw");
    assert_eq!(answered(&answers[4])["drawn"]["rows"], 2);
}

// ---------------------------------------------------------------- impact

#[test]
fn impact_refuses_relations_that_are_not_a_list() {
    let answers = bare(&[call(
        "impact",
        json!({ "text": DRAWN, "element": "P::A", "relations": "owns" }),
    )]);
    assert_eq!(
        refused(&answers[0]),
        "`relations` must be an array of relation names"
    );
}

#[test]
fn impact_stops_at_a_thousand_and_says_it_was_cut() {
    let mut text = String::from("package P {\n\tpart def Base;\n");
    for i in 0..1100 {
        text.push_str(&format!("\tpart def D{i} :> Base;\n"));
    }
    text.push_str("}\n");
    let answers = bare(&[call(
        "impact",
        json!({ "text": text, "element": "P::Base" }),
    )]);
    let said = answered(&answers[0]);
    assert_eq!(said["reached"].as_array().unwrap().len(), 1000, "{said}");
    assert_eq!(said["truncated"], true);
}

// ----------------------------------------------------------- export_json

#[test]
fn export_json_refuses_what_is_not_true_or_false_and_what_has_no_source() {
    let answers = bare(&[
        call(
            "export_json",
            json!({ "text": "package P;", "include_library": 5 }),
        ),
        call("export_json", json!({ "text": "package P;", "summary": 5 })),
        call("export_json", json!({})),
    ]);
    assert_eq!(
        refused(&answers[0]),
        "`include_library` must be true or false"
    );
    assert_eq!(refused(&answers[1]), "`summary` must be true or false");
    assert_eq!(
        refused(&answers[2]),
        "give either `text` or `path`, or start the server with `--project`"
    );
}

#[test]
fn export_json_of_a_model_with_a_dangling_name_says_what_the_document_lacks() {
    let answers = bare(&[call(
        "export_json",
        json!({ "text": "package P { part x : Nope; }" }),
    )]);
    let said = answered(&answers[0]);
    let warning = said["warnings"][0].as_str().unwrap();
    assert!(
        warning.starts_with("1 name(s) resolve to nothing"),
        "{warning}"
    );
    // it is still written, as far as it goes
    assert!(said["document"].is_array(), "{said}");
}

#[test]
fn export_json_with_the_library_counts_what_it_wrote_of_it() {
    let text = "package P { import ScalarValues::*; attribute x : Real; }";
    let answers = built_in(&[
        call(
            "export_json",
            json!({ "text": text, "include_library": true, "summary": true }),
        ),
        call(
            "export_json",
            json!({ "text": text, "include_library": true, "max_bytes": 1000 }),
        ),
    ]);
    let whole = answered(&answers[0]);
    assert_eq!(whole["library"], "included", "{whole}");
    assert!(
        whole["library_elements"].as_u64().unwrap() > 1000,
        "{whole}"
    );
    assert!(whole["document"].is_null(), "a summary has no document");
    // and a document too large to answer says how to narrow it, the
    // library being what to leave out first
    let large = answered(&answers[1]);
    assert_eq!(large["truncated"], true, "{large}");
    let note = large["note"].as_str().unwrap();
    assert!(note.contains(", leave out the library"), "{note}");
}

// ----------------------------------------------------------- import_json

#[test]
fn import_json_reads_a_file_and_refuses_one_it_cannot() {
    let dir = scratch("sysml-mcp-import-path");
    let document = answered(
        &bare(&[call(
            "export_json",
            json!({ "text": "package P { part def A; }" }),
        )])[0],
    )["document"]
        .clone();
    let file = dir.join("model.json");
    std::fs::write(&file, document.to_string()).unwrap();
    let answers = bare(&[
        call("import_json", json!({ "path": file.to_str().unwrap() })),
        call("import_json", json!({ "path": "/nonexistent/model.json" })),
        call("import_json", json!({ "json": 5 })),
    ]);
    let read = answered(&answers[0]);
    assert!(
        read["sysml"].as_str().unwrap().contains("part def A;"),
        "{read}"
    );
    let why = refused(&answers[1]);
    assert!(
        why.starts_with("cannot read `/nonexistent/model.json`"),
        "{why}"
    );
    assert_eq!(
        refused(&answers[2]),
        "`json` must be the document as text, or the array itself"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_json_of_a_kerml_model_is_written_as_kerml() {
    let read = answered(
        &bare(&[call(
            "import_json",
            json!({ "json": r#"[{"@type":"Class","@id":"c","declaredName":"K"}]"# }),
        )])[0],
    );
    assert_eq!(read["dialect"], "kerml", "{read}");
    assert_eq!(read["sysml"], "class K;\n");
}

/// A model with one of most of what the notation writes, so that what a
/// document refers to in the library is every sort of thing it can be.
const EVERYTHING: &str = "package Rich {
    private import ScalarValues::*;
    metadata def Safety { attribute isMandatory : Boolean; }
    attribute def Go;
    item def Fuel;
    enum def Color { enum red; enum green; }
    port def FuelPort { out item fuel : Fuel; }
    part def Engine { in port fuelIn : ~FuelPort; attribute power : Real; }
    part def Tank { port fuelOut : FuelPort; }
    interface def FuelLine { end port supply : FuelPort; end port demand : ~FuelPort; }
    comment about Engine /* engines
     *   burn fuel */
    doc Overview /* The rich model. */
    alias Motor for Engine;
    requirement def PowerReq {
        subject e : Engine;
        attribute minimum : Real;
        require constraint { e.power >= minimum }
    }
    requirement powerReq : PowerReq;
    calc def Double { in x : Real; return : Real; x * 2 }
    part car {
        part engine : Engine { @Safety { isMandatory = true; } }
        part tank : Tank;
        private attribute color : Color = Color::red;
        attribute doubled : Real = Double(engine.power);
        interface : FuelLine connect supply ::> tank.fuelOut to demand ::> engine.fuelIn;
        flow of Fuel from tank.fuelOut.fuel to engine.fuelIn.fuel;
        bind engine.power = doubled;
        satisfy powerReq by engine;
        perform drive;
        exhibit state modes : Modes;
        timeslice running;
    }
    action drive {
        attribute n : Integer := 0;
        first start;
        then action check;
        then decide;
            if n > 3 then stop;
            else go;
        action go { assign n := n + 1; }
        then fork;
            then left;
            then right;
        action left;
        action right;
        then join together;
        action stop {
            if n == 0 { action nothing; } else if n == 1 { action one; } else { action many; }
            while n > 0 { assign n := n - 1; } until n == 0;
            loop { action spin; } until true;
            for i : Integer in (1, 2, 3) { assign n := n + i; }
            send Go() to car;
            accept after 5;
        }
        then done;
    }
    state def Modes {
        entry; then off;
        state off;
        accept Go then on;
        state on { do action run; exit action halt; }
        accept when car.engine.power > 100 do send Go() to car then off;
        transition stopped first on if false then off;
    }
    package Safe { public import car::**[@Safety]; }
    view tree { expose car::*; render Views::asTreeDiagram; }
    dependency trace from car to powerReq;
}
";

#[test]
fn a_document_that_names_the_library_in_every_way_is_read_back_whole() {
    let alias = "package Al { alias Moe for ParametersOfInterestMetadata::moe; \
                 import ParametersOfInterestMetadata::moe; part def A { attribute x : Moe; } }";
    // an alias of the library is named by its own name, and what an
    // expression refers to is a library feature none of the document owns
    let aliased = "package Ali { private import ISQThermodynamics::TemperatureValue; \
                   attribute t : TemperatureValue; }";
    let referring = "package Ref { import ISQ::*; import SI::*; attribute m : MassValue = 5 [kg]; \
                     part def A { attribute k = m.num; } }";
    let models: Vec<String> = vec![
        EVERYTHING.to_string(),
        alias.to_string(),
        aliased.to_string(),
        referring.to_string(),
    ];
    let exported: Vec<Value> = models
        .iter()
        .map(|text| {
            call(
                "export_json",
                json!({ "text": text, "max_bytes": 64_000_000 }),
            )
        })
        .collect();
    let documents: Vec<Value> = built_in(&exported)
        .iter()
        .map(|it| answered(it)["document"].clone())
        .collect();
    let imports: Vec<Value> = documents
        .iter()
        .map(|document| call("import_json", json!({ "json": document })))
        .collect();
    let read: Vec<Value> = built_in(&imports).iter().map(answered).collect();
    for it in &read {
        assert!(
            it["library_references"].as_u64().is_some_and(|n| n > 0),
            "{it}"
        );
    }
    let sysml = read[0]["sysml"].as_str().unwrap();
    assert!(sysml.contains("part def Engine {"), "{sysml}");
    assert_eq!(read[0]["checked"]["ok"], true, "{}", read[0]["checked"]);
    for (at, name) in [(1, "alias"), (2, "aliased"), (3, "referring")] {
        assert_eq!(
            read[at]["checked"]["ok"], true,
            "{name}: {}",
            read[at]["checked"]
        );
    }
    let sysml = read[2]["sysml"].as_str().unwrap();
    assert!(
        sysml.contains("private import ISQThermodynamics::TemperatureValue;"),
        "{sysml}"
    );
    let sysml = read[3]["sysml"].as_str().unwrap();
    assert!(sysml.contains("attribute k = m.num;"), "{sysml}");
}

#[test]
fn a_document_that_mixes_kerml_into_a_package_of_sysml_says_it_does_not_parse() {
    let document = json!([
        {"@type":"Package","@id":"p","declaredName":"P",
         "ownedRelationship":[{"@id":"m0"},{"@id":"m1"}]},
        {"@type":"OwningMembership","@id":"m0","ownedRelatedElement":[{"@id":"k"}]},
        {"@type":"Class","@id":"k","declaredName":"K"},
        {"@type":"OwningMembership","@id":"m1","ownedRelatedElement":[{"@id":"a"}]},
        {"@type":"PartDefinition","@id":"a","declaredName":"A"}
    ]);
    let read = answered(&bare(&[call("import_json", json!({ "json": document }))])[0]);
    // it is said, in the text it wrote and where, rather than passed off
    assert_eq!(read["dialect"], "kerml", "{read}");
    assert_eq!(read["checked"]["ok"], false, "{read}");
    let errors = read["checked"]["parse_errors"].as_array().unwrap();
    assert!(!errors.is_empty(), "{read}");
    assert_eq!(errors[0]["line"], 3, "{read}");
    assert!(errors[0]["message"].as_str().unwrap().contains("expected"));
    assert!(read["checked"]["unresolved"].as_array().unwrap().len() >= 2);
}

#[test]
fn a_server_reads_one_document_after_another_what_the_library_publishes() {
    // the same server answers each, having worked out once which `@id`s
    // the library gives out
    let models = [
        "package P { import ParametersOfInterestMetadata::moe; }",
        "package Q { alias Mass for ISQ::MassValue; attribute m : Mass; }",
        "package R { import ScalarValues::*; attribute x : Real; }",
    ];
    let exported: Vec<Value> = models
        .iter()
        .map(|text| call("export_json", json!({ "text": text })))
        .collect();
    let documents: Vec<Value> = built_in(&exported)
        .iter()
        .map(|it| answered(it)["document"].clone())
        .collect();
    let imports: Vec<Value> = documents
        .iter()
        .map(|document| call("import_json", json!({ "json": document })))
        .collect();
    let answers = built_in(&imports);
    let read: Vec<Value> = answers.iter().map(answered).collect();
    for it in &read {
        assert_eq!(it["checked"]["ok"], true, "{it}");
    }
    assert!(read[0]["sysml"]
        .as_str()
        .unwrap()
        .contains("ParametersOfInterestMetadata::MeasureOfEffectiveness"));
    assert!(read[1]["sysml"]
        .as_str()
        .unwrap()
        .contains("alias Mass for ISQBase::MassValue;"));
    assert!(read[2]["library"]
        .as_array()
        .unwrap()
        .contains(&json!("ScalarValues::Real")));
}

// ------------------------------------------------------------------ api

/// A stand-in for a model server that sends the headers it is told to
/// with each answer: `answer` says, for a request target, the extra
/// headers and the body.
fn serving(
    count: usize,
    answer: impl Fn(&str) -> (Vec<String>, String) + Send + 'static,
) -> String {
    use std::io::{BufRead, BufReader};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for _ in 0..count {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            let target = line.split(' ').nth(1).unwrap_or_default().to_string();
            loop {
                line.clear();
                let _ = reader.read_line(&mut line);
                if line.trim_end().is_empty() {
                    break;
                }
            }
            let (headers, body) = answer(&target);
            let mut stream = stream;
            let extra: String = headers.iter().map(|it| format!("{it}\r\n")).collect();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    base
}

#[test]
fn api_elements_asked_for_one_element_answers_that_one() {
    let base = serving(3, |target| match target {
        "/projects/p1/commits/c1/elements/e1" => (vec![], r#"{"@id":"e1","name":"One"}"#.into()),
        _ => (vec![], "[]".into()),
    });
    let answers = bare(&[
        call(
            "api_elements",
            json!({ "server": base, "project": "p1", "commit": "c1", "element": "e1" }),
        ),
        call(
            "api_elements",
            json!({ "server": base, "project": "p1", "commit": "c1", "sysml": "yes" }),
        ),
        call(
            "api_elements",
            json!({ "server": base, "project": "p1", "commit": "c1" }),
        ),
    ]);
    let one = answered(&answers[0]);
    assert_eq!(one["element"]["name"], "One", "{one}");
    assert_eq!(one["server"], base);
    assert_eq!(refused(&answers[1]), "`sysml` must be true or false");
    // asked for the commit without saying `sysml`, it is the elements alone
    let listed = answered(&answers[2]);
    assert_eq!(listed["total"], 0, "{listed}");
    assert!(listed["model"].is_null(), "{listed}");
}

#[test]
fn a_link_with_a_quoted_parameter_is_followed_to_the_next_page() {
    // the title says `a"b`, and what follows it is still the link's
    let base = serving(2, |target| match target {
        "/projects" => (
            vec![r#"Link: </projects?page=2>; title="a\"b"; rel="next""#.to_string()],
            r#"[{"@id":"p1","@type":"Project","name":"One"}]"#.into(),
        ),
        _ => (
            vec![],
            r#"[{"@id":"p2","@type":"Project","name":"Two"}]"#.into(),
        ),
    });
    let answers = bare(&[call("api_projects", json!({ "server": base }))]);
    let names: Vec<String> = answered(&answers[0])["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["One", "Two"]);
}

#[test]
fn a_server_named_by_the_environment_is_the_one_asked() {
    use std::process::{Command, Stdio};
    let base = serving(1, |_| {
        (
            vec![],
            r#"[{"@id":"p1","@type":"Project","name":"Env"}]"#.into(),
        )
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_sysml"))
        .args(["--no-library", "mcp"])
        .env("SYSML_API_URL", &base)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the binary runs");
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, "{}", call("api_projects", json!({}))).unwrap();
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    let line = String::from_utf8(out.stdout).unwrap();
    let said = answered(&serde_json::from_str(line.lines().next().unwrap()).unwrap());
    assert_eq!(said["projects"][0]["name"], "Env", "{said}");
    assert_eq!(said["server"], base);
}

// ------------------------------------------------------------ resources

#[test]
fn resources_are_named_as_the_library_and_project_files_they_are() {
    let library = scratch("sysml-mcp-edge-library");
    std::fs::write(
        library.join("Lib.sysml"),
        "standard library package Lib { part def Thing; }\n",
    )
    .unwrap();
    let project = scratch("sysml-mcp-edge-project");
    std::fs::write(project.join("empty.sysml"), "").unwrap();
    std::fs::write(project.join("loose.sysml"), "part def Loose;\n").unwrap();
    std::fs::write(project.join("anonymous.sysml"), "part { part q; }\n").unwrap();
    std::fs::write(project.join("two.sysml"), "part def One;\npart def Two;\n").unwrap();
    let answers = drive(
        sysml_cli::mcp::Server::with_project(Some(&library), Some(&project)),
        &[
            request(1, "resources/list", json!({})),
            request(
                2,
                "resources/read",
                json!({ "uri": "https://example.com/nothing" }),
            ),
            request(
                3,
                "resources/read",
                json!({ "uri": "sysml://library/Lib.sysml" }),
            ),
            request(4, "resources/list", json!({ "cursor": 7 })),
        ],
    );
    let listed = answers[0]["result"]["resources"].as_array().unwrap();
    let of = |name: &str| {
        listed
            .iter()
            .find(|it| it["name"] == name)
            .unwrap_or_else(|| panic!("{name} is listed: {listed:?}"))
            .clone()
    };
    // a file that declares nothing says only whose it is
    assert_eq!(of("empty.sysml")["description"], "Project file");
    // and one whose top level is not a package is named by what it holds
    assert_eq!(
        of("loose.sysml")["description"],
        "Project file; declares Loose"
    );
    assert_eq!(
        of("two.sysml")["description"],
        "Project file; declares One, Two"
    );
    assert_eq!(
        of("anonymous.sysml")["description"],
        "Project file; declares q"
    );
    let lib = of("Lib.sysml");
    assert_eq!(lib["uri"], "sysml://library/Lib.sysml", "{lib}");
    assert_eq!(lib["description"], "Standard library file; declares Lib");
    // a URI that is neither of ours nor a file names nothing
    assert_eq!(answers[1]["error"]["code"], -32002, "{}", answers[1]);
    let read = &answers[2]["result"]["contents"][0];
    assert!(read["text"].as_str().unwrap().contains("part def Thing;"));
    assert_eq!(answers[3]["error"]["code"], -32602, "{}", answers[3]);
    assert!(answers[3]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("`cursor` is the string a page before gave"));
    let _ = std::fs::remove_dir_all(&library);
    let _ = std::fs::remove_dir_all(&project);
}

// -------------------------------------------------------------- prompts

#[test]
fn a_prompt_argument_left_blank_reads_as_what_it_stands_for() {
    let answers = bare(&[
        request(
            1,
            "prompts/get",
            json!({ "name": "write_model", "arguments": { "requirement": "A car", "package": "  " } }),
        ),
        request(
            2,
            "prompts/get",
            json!({ "name": "model_from_code", "arguments": { "source": "src" } }),
        ),
        request(
            3,
            "prompts/get",
            json!({ "name": "write_model", "arguments": { "requirement": 5 } }),
        ),
    ]);
    let text = |answer: &Value| {
        answer["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let write = text(&answers[0]);
    assert!(
        write.contains("Put it in a package named for what the requirement is about, in a new `.sysml` file of your choosing."),
        "{write}"
    );
    let code = text(&answers[1]);
    assert!(code.contains("written in Rust"), "{code}");
    assert!(
        code.contains("as a SysML package named the name the code gives itself"),
        "{code}"
    );
    assert_eq!(answers[2]["error"]["code"], -32602, "{}", answers[2]);
    assert!(answers[2]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("`requirement` must be a string"));
}

// ----------------------------------------------------------- the process

#[cfg(target_os = "linux")]
#[test]
fn output_that_cannot_be_written_for_want_of_room_is_an_error_not_silence() {
    let dir = scratch("sysml-full-device");
    let model = dir.join("m.sysml");
    std::fs::write(&model, "package P { part def A; }\n").unwrap();
    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("/dev/full opens");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_sysml"))
        .args(["--no-library", "export", model.to_str().unwrap()])
        .stdout(full)
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("the binary runs");
    assert_eq!(out.status.code(), Some(1));
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("error: cannot write to stdout"), "{said}");
    let _ = std::fs::remove_dir_all(&dir);
}

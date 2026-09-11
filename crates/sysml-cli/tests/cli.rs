//! End-to-end tests running the `sysml` binary (every subcommand and its
//! error paths).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sysml_corpus::library;

fn sysml(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sysml"))
        .args(args)
        .output()
        .unwrap()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sysml-cli-test-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

const OK_MODEL: &str = "package P {\n    part def Vehicle;\n    part car : Vehicle;\n}\n";

#[test]
fn parse_reports_ok_and_dumps_trees() {
    let dir = temp_dir("parse");
    let ok = write(&dir, "ok.sysml", OK_MODEL);
    let out = sysml(&["parse", ok.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("ok (0 error(s))"));

    let out = sysml(&["parse", "--tree", ok.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("SOURCE_FILE"));

    // kerml dialect selection
    let kerml = write(&dir, "ok.kerml", "classifier A;\n");
    assert!(sysml(&["parse", kerml.to_str().unwrap()]).status.success());
}

#[test]
fn parse_reports_errors_with_positions() {
    let dir = temp_dir("parse-err");
    let bad = write(&dir, "bad.sysml", "part def {{{\n%%\n");
    let out = sysml(&["parse", bad.to_str().unwrap()]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error:"), "{stderr}");
    assert!(stderr.contains("bad.sysml:"), "{stderr}");

    let out = sysml(&["parse", dir.join("missing.sysml").to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot read"));
}

#[test]
fn stats_counts_elements() {
    let dir = temp_dir("stats");
    let ok = write(&dir, "ok.sysml", OK_MODEL);
    let out = sysml(&["stats", ok.to_str().unwrap()]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("PartDefinition"), "{stdout}");
    assert!(stdout.contains("total elements"), "{stdout}");

    let out = sysml(&["stats", dir.join("missing.sysml").to_str().unwrap()]);
    assert!(!out.status.success());
}

#[test]
fn export_writes_interchange_json() {
    let dir = temp_dir("export");
    let ok = write(&dir, "ok.sysml", OK_MODEL);
    let out = sysml(&["export", ok.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("\"@type\": \"Package\""));

    let json = dir.join("out.json");
    let out = sysml(&["export", ok.to_str().unwrap(), "-o", json.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(std::fs::read_to_string(&json).unwrap().contains("Vehicle"));

    // a file the parser could not follow is missing declarations, and
    // what would be exported is not the model that was written
    let bad = write(&dir, "bad.sysml", "part def {{{\n");
    let out = sysml(&["export", bad.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("error:"));
    assert!(out.stdout.is_empty(), "nothing partial is written out");

    let out = sysml(&["export", dir.join("missing.sysml").to_str().unwrap()]);
    assert!(!out.status.success());
    let unwritable = dir.join("no-such-dir").join("out.json");
    let out = sysml(&[
        "export",
        ok.to_str().unwrap(),
        "-o",
        unwritable.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot write"));
}

#[test]
fn export_resolves_and_marks_the_library() {
    let dir = temp_dir("export-library");
    let lib = write(&dir, "lib.sysml", "package L {\n\tpart def Base;\n}\n");
    let model = write(
        &dir,
        "model.sysml",
        "package M {\n\timport L::*;\n\tpart def Car :> Base;\n}\n",
    );
    // `--library` here names a library of this test's own making, and
    // what is asserted below is which elements it marked -- so the real
    // one, which would mark sixty thousand more, stays out
    let out = sysml(&[
        "--no-library",
        "export",
        model.to_str().unwrap(),
        "--library",
        lib.to_str().unwrap(),
        "--include-library",
    ]);
    assert!(out.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let objects = json.as_array().unwrap();
    let of = |name: &str| {
        objects
            .iter()
            .find(|object| object["declaredName"] == name)
            .unwrap()
    };
    // the library came along, marked as what it is
    assert_eq!(of("Base")["isLibraryElement"], true);
    assert_eq!(of("Car")["isLibraryElement"], false);
    // resolution ran: the specialization is reified and derived from
    let car = of("Car");
    assert_eq!(car["ownedSubclassification"].as_array().unwrap().len(), 1);
    // and the import reached M's member list
    let m = of("M");
    assert!(m["member"]
        .as_array()
        .unwrap()
        .iter()
        .any(|member| member["@id"] == of("Base")["@id"]));
    assert_eq!(of("M")["importedMembership"].as_array().unwrap().len(), 1);

    // a library that cannot be read fails the export like any input
    let out = sysml(&[
        "export",
        model.to_str().unwrap(),
        "--library",
        dir.join("absent.sysml").to_str().unwrap(),
    ]);
    assert!(!out.status.success());
}

#[test]
fn import_rust_writes_a_package_from_rustdoc_json() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../sysml-rust/tests/fixtures/inventory_store.rustdoc.json");
    let dir = temp_dir("import-rust");
    let out_path = dir.join("api.sysml");
    let out = sysml(&[
        "import-rust",
        fixture.to_str().unwrap(),
        "--package",
        "Warehouse",
        "-o",
        out_path.to_str().unwrap(),
    ]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("definition(s)"));
    let written = std::fs::read_to_string(&out_path).unwrap();
    assert!(written.contains("package Warehouse {"));
    assert!(written.contains("action def GetStock {"));

    // to stdout without -o
    let out = sysml(&["import-rust", fixture.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("package InventoryStoreApi {"));

    // a missing file and a broken one are named errors
    let out = sysml(&["import-rust", dir.join("absent.json").to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot read"));
    let bad = write(&dir, "bad.json", "not json");
    let out = sysml(&["import-rust", bad.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not JSON"));
}

#[test]
fn what_a_crate_imported_as_is_counted_off_the_tree() {
    // the count used to be occurrences of `" def "` in the text it wrote,
    // so a doc comment saying the words was read as another definition
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../sysml-rust/tests/fixtures/inventory_store.rustdoc.json");
    let dir = temp_dir("import-rust-count");
    let plain = dir.join("plain.sysml");
    let honest = sysml(&[
        "import-rust",
        fixture.to_str().unwrap(),
        "-o",
        plain.to_str().unwrap(),
    ]);
    let counted = String::from_utf8_lossy(&honest.stderr);
    let said = counted.split_once(" to ").unwrap().0.to_string();
    let n: usize = said
        .trim_start_matches("wrote ")
        .split_whitespace()
        .next()
        .and_then(|word| word.parse().ok())
        .expect("a count to compare against");
    assert!(n > 0, "{said}");

    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&fixture).unwrap()).unwrap();
    let index = doc["index"].as_object_mut().unwrap();
    let first = index.keys().next().unwrap().clone();
    index.get_mut(&first).unwrap()["docs"] =
        serde_json::json!("a part def in prose, and an item def too");
    let talkative = write(&dir, "talkative.json", &doc.to_string());
    let written = dir.join("talkative.sysml");
    let out = sysml(&[
        "import-rust",
        talkative.to_str().unwrap(),
        "-o",
        written.to_str().unwrap(),
    ]);
    assert!(out.status.success());
    // the prose really did reach the file, so the count had every chance
    // to be fooled by it
    let text = std::fs::read_to_string(&written).unwrap();
    assert!(text.contains("a part def in prose"), "{text}");
    let counted = String::from_utf8_lossy(&out.stderr);
    assert_eq!(counted.split_once(" to ").unwrap().0, said);
}

#[test]
fn rustgen_generates_and_says_what_stopped_it() {
    let dir = temp_dir("rustgen");
    let out_path = dir.join("generated.rs");
    let scalars_lib = write(
        &dir,
        "scalars0.kerml",
        "package ScalarValues {\n\tabstract datatype Real;\n}\n",
    );
    let model = write(
        &dir,
        "planner.sysml",
        "package Planner {\n\
         \tprivate import ScalarValues::*;\n\
         \tpart def OrderPlanner {\n\
         \t\tdoc /* Stands in for the pub struct Ghost it replaces. */\n\
         \t\tattribute threshold : Real;\n\t}\n\
         }\n",
    );
    let out = sysml(&[
        "rustgen",
        model.to_str().unwrap(),
        "--library",
        scalars_lib.to_str().unwrap(),
        "-o",
        out_path.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // one struct, not two: the `doc` that mentions another is prose
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("1 struct(s)"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(std::fs::read_to_string(&out_path)
        .unwrap()
        .contains("pub struct OrderPlanner"));

    // an unresolved model is refused: generated code would silently miss
    // whatever did not resolve
    let broken = write(
        &dir,
        "broken.sysml",
        "part def P {\n\tport x : Nowhere;\n}\n",
    );
    let out = sysml(&["rustgen", broken.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unresolved `Nowhere`"));

    // unreadable inputs fail like everywhere else
    let out = sysml(&["rustgen", dir.join("absent.sysml").to_str().unwrap()]);
    assert!(!out.status.success());
    let out = sysml(&[
        "rustgen",
        model.to_str().unwrap(),
        "--library",
        dir.join("absent.sysml").to_str().unwrap(),
    ]);
    assert!(!out.status.success());

    // a performed API no port provides is a named error
    let scalars = write(
        &dir,
        "scalars.kerml",
        "package ScalarValues {\n\tabstract datatype String;\n}\n",
    );
    let api = write(
        &dir,
        "api.sysml",
        "package Api {\n\
         \tprivate import ScalarValues::*;\n\
         \tmetadata def code { attribute writtenIn : String;\n\
    \t\tattribute path : String; attribute takesSelf : String; }\n\
         \taction def Orphan { @code { :>> writtenIn = \"rust\"; :>> path = \"elsewhere::Api::orphan\"; :>> takesSelf = \"&self\"; } }\n\
         }\n",
    );
    let system = write(
        &dir,
        "system.sysml",
        "package S {\n\
         \tprivate import Api::*;\n\
         \tpart def Node {\n\t\tperform action stray : Orphan;\n\t}\n}\n",
    );
    let out = sysml(&[
        "rustgen",
        system.to_str().unwrap(),
        "--library",
        api.to_str().unwrap(),
        "--library",
        scalars.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no port of the part"));
}

/// Re-spacing a file the parser could not follow can move where a quote
/// ends, and `--write` puts that in the modeller's file. Printing it is
/// safe -- every character is still there -- so only the rewrite is
/// refused.
#[test]
fn fmt_will_not_rewrite_a_file_it_could_not_parse() {
    let dir = temp_dir("fmt-broken");
    let text = "package 'Half Open {\n\tpart def A;\n";
    let broken = write(&dir, "broken.sysml", text);

    let out = sysml(&["fmt", "--write", broken.to_str().unwrap()]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("does not parse"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(&broken).unwrap(),
        text,
        "the file must be as it was"
    );

    // printing it is still allowed, and loses nothing
    let out = sysml(&["fmt", broken.to_str().unwrap()]);
    assert!(out.status.success());
    let printed = String::from_utf8_lossy(&out.stdout);
    let letters = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };
    assert_eq!(letters(&printed), letters(text), "{printed}");
}

#[test]
fn fmt_formats_checks_and_writes() {
    let dir = temp_dir("fmt");
    let messy = write(&dir, "messy.sysml", "package   P{part def A;}");
    let out = sysml(&["fmt", messy.to_str().unwrap()]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "package P {\n    part def A;\n}\n"
    );

    let out = sysml(&["fmt", "--check", messy.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not formatted"));

    let out = sysml(&["fmt", "--write", messy.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("formatted"));
    // now canonical: --check passes and --write is a no-op
    assert!(sysml(&["fmt", "--check", messy.to_str().unwrap()])
        .status
        .success());
    let out = sysml(&["fmt", "--write", messy.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(out.stderr.is_empty());

    let out = sysml(&["fmt", dir.join("missing.sysml").to_str().unwrap()]);
    assert!(!out.status.success());
}

#[cfg(unix)]
#[test]
fn fmt_write_reports_readonly_failures() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("fmt-ro");
    let messy = write(&dir, "messy.sysml", "package   P{}");
    std::fs::set_permissions(&messy, std::fs::Permissions::from_mode(0o444)).unwrap();
    let out = sysml(&["fmt", "--write", messy.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot write"));
}

#[test]
fn check_resolves_and_reports_unresolved() {
    let dir = temp_dir("check");
    write(&dir, "lib.sysml", OK_MODEL);
    let out = sysml(&["check", dir.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("(100.0%)"));

    // many unresolved references: default --show truncates with "and N more"
    let mut bad = String::from("package Q {\n");
    for i in 0..25 {
        bad.push_str(&format!("    part p{i} : Missing{i};\n"));
    }
    bad.push_str("}\n");
    let bad = write(&dir, "bad.sysml", &bad);
    let out = sysml(&["check", bad.to_str().unwrap()]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("`Missing0` resolves to nothing"),
        "{stderr}"
    );
    // and the line it is written on is quoted back, with the name
    // underlined -- a place alone is a place the reader has to go and
    // look at
    assert!(stderr.contains("part p0 : Missing0;"), "{stderr}");
    assert!(stderr.contains("^^^^^^^^"), "{stderr}");
    assert!(stderr.contains("and 5 more"), "{stderr}");

    // --show 0 lists everything
    let out = sysml(&["check", "--show", "0", bad.to_str().unwrap()]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("Missing24"));

    let out = sysml(&["check", dir.join("missing.sysml").to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot read"));

    // and a file that does not parse is a finding here too, in words
    let unclosed = write(&dir, "unclosed.sysml", "package Q {\n\tpart def A;\n");
    let out = sysml(&["check", unclosed.to_str().unwrap()]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("expected"), "{stderr}");
}

/// An export is the model, and the library is what it was resolved
/// against.
///
/// The two used to be one document. Since the library stopped having to
/// be named to be loaded, `sysml export model.sysml` wrote a hundred and
/// twenty-eight thousand elements where the model has three, and `sysml
/// api push` sent every one of them to somebody's server -- seven
/// hundred and ninety megabytes of standard library that nobody asked
/// for. What the model refers to across that line is written as the
/// `@id` it always was, and those are UUIDv5 over the ownership path:
/// anybody holding the same library computes the same ones, which is how
/// the standard refers to an element another project holds.
#[test]
fn an_export_is_the_model_and_not_the_library_it_resolved_against() {
    let dir = temp_dir("export-share");
    let model = write(
        &dir,
        "m.sysml",
        "package M {\n\tprivate import ISQ::*;\n\tpart def Car {\n\t\tattribute mass : MassValue;\n\t}\n}\n",
    );
    let out = sysml(&["export", model.to_str().unwrap()]);
    assert!(out.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let objects = json.as_array().unwrap();
    assert!(
        objects.len() < 50,
        "the model, not the library: {}",
        objects.len()
    );
    assert!(
        objects.iter().all(|it| it["isLibraryElement"] != true),
        "nothing of the library's is written"
    );

    // and what the model says about a library type is still said: the
    // typing is the model's own element, and what it points at is an id
    // the reader resolves against the library it already has
    let here: std::collections::HashSet<&str> =
        objects.iter().filter_map(|it| it["@id"].as_str()).collect();
    let typing = objects
        .iter()
        .find(|it| it["@type"] == "FeatureTyping")
        .expect("the model types its attribute");
    let target = typing["type"]["@id"].as_str().expect("by id");
    assert!(!here.contains(target), "and the library is not in here");

    // asked for, it comes along, and the answer says how much of it
    // there is
    let out = sysml(&[
        "export",
        model.to_str().unwrap(),
        "--include-library",
        "-o",
        "/dev/null",
    ]);
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("of them are the library's"), "{said:?}");
}

/// `check` answers for what the specification requires, and not only
/// for what resolves.
///
/// A model whose every name resolves can still be one the standard
/// rejects, and until this was asked only the MCP server ever asked it:
/// running `check` -- which is most of the reason the command exists --
/// called such a model sound.
///
/// Two things are asked first. Constraints come after names, as names
/// come after syntax: asked of a model with a dangling reference they
/// answer about the hole, and one undeclared type in a five-line file
/// drew four complaints of its own, none of them a second thing to fix.
/// And they are written against the standard library, so without it
/// they report what is missing rather than what is wrong -- `case def
/// Trip { objective placed; }` draws four on its own and none with the
/// library beside it.
#[test]
fn check_answers_for_the_constraints_the_specification_states() {
    let library = library();
    let library = library.to_str().unwrap();
    let dir = temp_dir("check-rules");

    // an objective belongs to a case, and this is a part
    let wrong = write(
        &dir,
        "wrong.sysml",
        "package P {\n\tpart def Engine {\n\t\tobjective misplaced;\n\t}\n}\n",
    );
    let out = sysml(&["check", wrong.to_str().unwrap(), library]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("wrong.sysml:3:13"),
        "placed where it is written: {stderr}"
    );
    assert!(stderr.contains("P::Engine::misplaced"), "{stderr}");
    assert!(stderr.contains("must be a CaseDefinition"), "{stderr}");
    // the constraint's own name, where rustc puts an error code: it is
    // what somebody looking the rule up will search for
    assert!(
        stderr.contains("error[validateObjectiveMembershipOwningType]"),
        "{stderr}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("1 violation(s)"), "{stdout}");

    // as JSON, with the rule that was broken and where it was broken
    let out = sysml(&[
        "--format",
        "json",
        "check",
        wrong.to_str().unwrap(),
        library,
    ]);
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["ok"], false);
    assert_eq!(
        json["violations"][0]["rule"],
        "validateObjectiveMembershipOwningType"
    );
    assert_eq!(json["violations"][0]["line"], 3);
    assert!(json["rules"]["held"].as_u64().unwrap() > 0, "{json}");

    // the same model, put where it belongs, holds
    let right = write(
        &dir,
        "right.sysml",
        "package P {\n\tcase def Trip {\n\t\tobjective placed;\n\t}\n}\n",
    );
    let out = sysml(&["check", right.to_str().unwrap(), library]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("0 violation(s)"), "{stdout}");

    // without the library the constraints are not asked at all, rather
    // than asked and answered about what is not there
    let out = sysml(&["--no-library", "check", right.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("constraint(s)"),
        "{:?}",
        String::from_utf8_lossy(&out.stdout)
    );

    // and a model that does not resolve is not also told what the
    // standard would have said about the hole
    let dangling = write(
        &dir,
        "dangling.sysml",
        "package P {\n\tpart engine : NoSuchThing;\n}\n",
    );
    let out = sysml(&["check", dangling.to_str().unwrap(), library]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("`NoSuchThing` resolves to nothing"),
        "{stderr}"
    );
    assert!(!stderr.contains("must be"), "no constraint noise: {stderr}");
}

#[test]
fn diagram_renders_definitions_as_svg() {
    let dir = temp_dir("diagram");
    let model = write(
        &dir,
        "model.sysml",
        "part def PowerSource;\npart def Engine :> PowerSource { attribute power; }\n",
    );

    let out = sysml(&["diagram", model.to_str().unwrap()]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("<svg xmlns="), "{stdout}");
    assert_eq!(stdout.matches("<rect class=\"box\"").count(), 2);
    assert_eq!(stdout.matches("marker-end").count(), 1);
    assert!(stdout.contains(">attribute power</text>"));

    let svg = dir.join("out.svg");
    let out = sysml(&[
        "diagram",
        model.to_str().unwrap(),
        "-o",
        svg.to_str().unwrap(),
    ]);
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(
            "2 box(es), 1 specialization(s), 0 composition(s), 0 reference(s), 0 subsetting(s), 0 connection(s), 0 flow(s), 0 allocation(s), 0 transition(s), 0 dependency(ies) and 0 satisfaction(s)"
        ),
        "{stderr}"
    );
    assert!(std::fs::read_to_string(&svg)
        .unwrap()
        .contains("PowerSource"));

    // directories load the same way `check` loads them
    assert!(sysml(&["diagram", dir.to_str().unwrap()]).status.success());
}

#[test]
fn diagram_resolves_against_a_library_without_drawing_it() {
    let dir = temp_dir("diagram-library");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    write(&lib, "types.sysml", "attribute def Real;\n");
    let model = write(
        &dir,
        "model.sysml",
        "part def Engine { attribute power : Real; }\n",
    );

    // on its own, `Real` does not resolve, so the feature stays untyped
    let bare = sysml(&["diagram", model.to_str().unwrap()]);
    let bare = String::from_utf8_lossy(&bare.stdout);
    assert!(bare.contains(">attribute power</text>"), "{bare}");

    let out = sysml(&[
        "diagram",
        model.to_str().unwrap(),
        "--library",
        lib.to_str().unwrap(),
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(">attribute power : Real</text>"),
        "{stdout}"
    );
    // the library supplies the name but never becomes a box of its own
    assert_eq!(stdout.matches("<rect class=\"box\"").count(), 1);

    let out = sysml(&[
        "diagram",
        model.to_str().unwrap(),
        "--library",
        dir.join("missing.sysml").to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot read"));
}

#[test]
fn diagram_draws_the_internal_structure_of_one_definition() {
    let dir = temp_dir("diagram-internal");
    let model = write(
        &dir,
        "car.sysml",
        "part def Wheel { port hub; }\n\
         part def Axle { port mount; }\n\
         part def Car {\n\
         \tpart w : Wheel;\n\
         \tpart a : Axle;\n\
         \tconnect w.hub to a.mount;\n\
         }\n",
    );

    let out = sysml(&["diagram", model.to_str().unwrap(), "--internal", "Car"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // the parts of Car, not the definitions themselves
    assert_eq!(stdout.matches("<rect class=\"box\"").count(), 2);
    assert!(stdout.contains(">w : Wheel</text>"), "{stdout}");
    assert_eq!(stdout.matches("<line class=\"edge\"").count(), 1);

    let out = sysml(&["diagram", model.to_str().unwrap(), "--internal", "NoSuch"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no element named `NoSuch`"));
}

#[test]
fn diagram_draws_an_interaction_as_a_sequence_view() {
    let dir = temp_dir("diagram-sequence");
    let model = write(
        &dir,
        "talk.sysml",
        "item def Ask;\n\
         part def A { event occurrence sent; }\n\
         part def B { event occurrence got; }\n\
         occurrence def Talk {\n\
         \tref part a : A;\n\
         \tref part b : B;\n\
         \tmessage ask of Ask from a.sent to b.got;\n\
         }\n",
    );

    let svg = dir.join("talk.svg");
    let out = sysml(&[
        "diagram",
        model.to_str().unwrap(),
        "--sequence",
        "Talk",
        "-o",
        svg.to_str().unwrap(),
    ]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("2 lifeline(s) and 1 message(s)"));
    let drawn = std::fs::read_to_string(&svg).unwrap();
    assert_eq!(drawn.matches("class=\"lifeline\"").count(), 2);
    assert!(drawn.contains(">ask of Ask</text>"), "{drawn}");

    // a definition that declares no interaction has none to draw
    let out = sysml(&["diagram", model.to_str().unwrap(), "--sequence", "A"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("declares no interaction"));

    // and a name nothing answers to is reported the way `--internal` is
    let out = sysml(&["diagram", model.to_str().unwrap(), "--sequence", "NoSuch"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no element named `NoSuch`"));
}

#[test]
fn diagram_draws_the_membership_tree() {
    let dir = temp_dir("diagram-browser");
    let model = write(
        &dir,
        "tree.sysml",
        "package P {\n\tpart def Wheel { port hub; }\n}\n",
    );

    let out = sysml(&["diagram", model.to_str().unwrap(), "--browser"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // one row per named element, no boxes at all
    assert_eq!(stdout.matches("<text class=\"keyword\"").count(), 3);
    assert!(!stdout.contains("<rect"));
    assert!(stdout.contains(">hub</tspan>"), "{stdout}");

    let svg = dir.join("tree.svg");
    let out = sysml(&[
        "diagram",
        model.to_str().unwrap(),
        "--browser",
        "-o",
        svg.to_str().unwrap(),
    ]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("3 row(s)"));

    // an empty model has no tree to draw
    let empty = write(&dir, "empty.sysml", "\n");
    let out = sysml(&["diagram", empty.to_str().unwrap(), "--browser"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("nothing to draw"));

    // the two modes ask for different drawings
    let out = sysml(&[
        "diagram",
        model.to_str().unwrap(),
        "--browser",
        "--internal",
        "Wheel",
    ]);
    assert!(!out.status.success());
}

#[test]
fn diagram_reports_empty_models_and_write_failures() {
    let dir = temp_dir("diagram-err");
    let empty = write(&dir, "empty.sysml", "package OnlyAPackage;\n");
    let out = sysml(&["diagram", empty.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("nothing to draw"));

    let model = write(&dir, "model.sysml", "part def A;\n");
    let unwritable = dir.join("no-such-dir").join("out.svg");
    let out = sysml(&[
        "diagram",
        model.to_str().unwrap(),
        "-o",
        unwritable.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot write"));

    let out = sysml(&["diagram", dir.join("missing.sysml").to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot read"));
}

#[test]
fn corpus_measures_parse_rates() {
    let dir = temp_dir("corpus");
    write(&dir, "ok.sysml", OK_MODEL);
    write(&dir, "bad.sysml", "part def {{{\n");
    let out = sysml(&["corpus", dir.to_str().unwrap()]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("1/2 files ok"), "{stdout}");
    assert!(stdout.contains("worst"), "{stdout}");

    let out = sysml(&["corpus", "--failures", dir.to_str().unwrap()]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("FAIL"));

    let empty = temp_dir("corpus-empty");
    let out = sysml(&["corpus", empty.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no .sysml"));
}

#[cfg(unix)]
#[test]
fn corpus_warns_on_unreadable_files() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("corpus-unreadable");
    write(&dir, "ok.sysml", OK_MODEL);
    let hidden = write(&dir, "hidden.sysml", OK_MODEL);
    std::fs::set_permissions(&hidden, std::fs::Permissions::from_mode(0o000)).unwrap();
    let out = sysml(&["corpus", dir.to_str().unwrap()]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot read"));
    std::fs::set_permissions(&hidden, std::fs::Permissions::from_mode(0o644)).unwrap();
}

#[cfg(unix)]
#[test]
fn check_reports_unreadable_directories() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("check-unreadable");
    let hidden = write(&dir, "hidden.sysml", OK_MODEL);
    std::fs::set_permissions(&hidden, std::fs::Permissions::from_mode(0o000)).unwrap();
    let out = sysml(&["check", dir.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot load"));
    std::fs::set_permissions(&hidden, std::fs::Permissions::from_mode(0o644)).unwrap();
}

#[test]
fn check_with_no_references_reports_100_percent() {
    let dir = temp_dir("check-empty");
    let ok = write(&dir, "empty.sysml", "package OnlyAPackage;\n");
    // `--no-library`, because a model read against the copy built in
    // has seventeen thousand references into it and this is about a
    // model that has none
    let out = sysml(&["--no-library", "check", ok.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("0/0"));
}

#[test]
fn corpus_handles_subdirectories_and_non_directories() {
    let dir = temp_dir("corpus-nested");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    write(&dir.join("sub"), "ok.sysml", OK_MODEL);
    let out = sysml(&["corpus", dir.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("1/1 files ok"));

    // passing a file: read_dir fails, so no corpus files are found
    let file = write(&dir, "notadir.sysml", OK_MODEL);
    let out = sysml(&["corpus", file.to_str().unwrap()]);
    assert!(!out.status.success());
}

#[test]
fn no_arguments_prints_usage() {
    let out = sysml(&[]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Usage"));
}

#[test]
fn diagram_can_let_elk_lay_out_the_boxes() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("diagram-elk");
    let model = write(
        &dir,
        "model.sysml",
        "part def PowerSource;\npart def Engine :> PowerSource;\n",
    );
    let fake = dir.join("fake-elk");
    std::fs::write(
        &fake,
        "#!/bin/sh\ncat >/dev/null\n\
         printf '{\"width\":400,\"height\":144,\"children\":\
[{\"id\":\"n0\",\"x\":0,\"y\":0},{\"id\":\"n1\",\"x\":0,\"y\":100}]}\\n'\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

    let out = sysml(&[
        "diagram",
        model.to_str().unwrap(),
        "--elk",
        "--elk-command",
        fake.to_str().unwrap(),
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // the fake's canvas plus two 16px margins, not the built-in one
    assert!(stdout.contains("height=\"176\""), "{stdout}");

    // a missing elkrs is an error that says what to install
    let out = sysml(&[
        "diagram",
        model.to_str().unwrap(),
        "--elk",
        "--elk-command",
        "/nonexistent/elk/elkrs",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cargo install elkrs"), "{stderr}");
}

#[test]
fn json_reports_what_a_program_works_from() {
    let dir = temp_dir("json");
    let ok = write(&dir, "ok.sysml", OK_MODEL);
    let broken = write(
        &dir,
        "broken.sysml",
        "package P {\n\tpart def A;\n\tpart b : Missing;\n",
    );
    // the same model, closed: a name nothing answers to, and nothing else
    let unresolved = write(
        &dir,
        "unresolved.sysml",
        "package P {\n\tpart def A;\n\tpart b : Missing;\n}\n",
    );
    let json = |out: &Output| -> serde_json::Value {
        serde_json::from_slice(&out.stdout).expect("stdout is one JSON document")
    };

    // a syntax error, placed where an editor counts from one
    let out = sysml(&["--format", "json", "parse", broken.to_str().unwrap()]);
    assert!(!out.status.success());
    let v = json(&out);
    assert_eq!(v["command"], "parse");
    assert_eq!(v["ok"], false);
    assert_eq!(v["files"][0]["errors"][0]["line"], 4);
    assert!(v["files"][0]["errors"][0]["message"].is_string());

    // a file that does not parse has no names worth resolving, and
    // `check` says so rather than reporting nothing wrong with it
    let out = sysml(&["--format", "json", "check", broken.to_str().unwrap()]);
    assert!(!out.status.success());
    let v = json(&out);
    assert_eq!(v["command"], "check");
    assert_eq!(v["ok"], false);
    assert_eq!(v["parseErrors"][0]["line"], 4);
    assert!(v["parseErrors"][0]["message"].is_string());
    assert!(v["unresolved"].is_null(), "syntax first, names after");

    // an unresolved reference, by name and place
    let out = sysml(&["--format", "json", "check", unresolved.to_str().unwrap()]);
    assert!(!out.status.success());
    let v = json(&out);
    assert_eq!(v["command"], "check");
    assert_eq!(v["parseErrors"].as_array().unwrap().len(), 0);
    assert_eq!(v["unresolved"][0]["name"], "Missing");
    assert_eq!(v["unresolved"][0]["line"], 3);
    assert_eq!(v["unresolved"][0]["column"], 11);

    // and nothing to say, said the same way
    let out = sysml(&["--format", "json", "check", ok.to_str().unwrap()]);
    assert!(out.status.success());
    let v = json(&out);
    assert_eq!(v["ok"], true);
    assert_eq!(v["unresolved"].as_array().unwrap().len(), 0);

    let out = sysml(&["--format", "json", "stats", ok.to_str().unwrap()]);
    assert!(out.status.success());
    let v = json(&out);
    assert_eq!(v["counts"]["PartDefinition"], 1);
    assert_eq!(v["parseErrors"], 0);

    // a file that cannot be read is a finding, not a stray line, and
    // every command says so in the same place
    for command in ["parse", "check", "stats", "fmt"] {
        let mut args = vec!["--format", "json", command, "no-such-file.sysml"];
        if command == "fmt" {
            args.push("--check");
        }
        let out = sysml(&args);
        assert!(!out.status.success(), "{command}");
        let v = json(&out);
        assert_eq!(v["command"], command);
        assert_eq!(v["ok"], false, "{command}");
        assert_eq!(
            v["unreadable"][0]["path"], "no-such-file.sysml",
            "{command}"
        );
        assert!(v["unreadable"][0]["error"].is_string(), "{command}");
    }

    // fmt --check names the files a program would rewrite
    let ugly = write(&dir, "ugly.sysml", "package  P {  }\n");
    let out = sysml(&["--format", "json", "fmt", "--check", ugly.to_str().unwrap()]);
    assert!(!out.status.success());
    let v = json(&out);
    assert_eq!(v["command"], "fmt");
    assert_eq!(v["unformatted"].as_array().unwrap().len(), 1);
    let out = sysml(&["--format", "json", "fmt", "--check", ok.to_str().unwrap()]);
    assert!(out.status.success());
    assert_eq!(json(&out)["ok"], true);
}

/// A one-shot stand-in for a model server: reads the whole request --
/// headers and any body -- then answers with `body`. Returns the base
/// URL to point the CLI at.
fn serve_once(body: &'static str) -> String {
    serve_with("200 OK", body)
}

/// The same, for a server that refuses.
fn serve_with(status: &'static str, body: &'static str) -> String {
    use std::io::{BufRead, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut length = 0usize;
        let mut line = String::new();
        loop {
            line.clear();
            let _ = reader.read_line(&mut line);
            let header = line.trim_end().to_lowercase();
            if header.is_empty() {
                break;
            }
            if let Some(value) = header.strip_prefix("content-length: ") {
                length = value.trim().parse().unwrap();
            }
        }
        let mut request_body = vec![0u8; length];
        let _ = reader.read_exact(&mut request_body);
        let mut stream = stream;
        let _ = stream.write_all(
            format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        );
    });
    format!("http://{addr}")
}

/// The model server commands, each against a server that answers once.
/// `push` is the one that matters: it exports the model the same way
/// `export` does and sends that, so the two cannot come to mean
/// different things by a model.
#[test]
fn api_talks_to_a_model_server() {
    let base = serve_once(r#"[{"@id":"p1","@type":"Project","name":"Demo"}]"#);
    let out = sysml(&["api", "--server", &base, "projects"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("p1 Demo"));

    let base = serve_once(r#"{"@id":"p1","@type":"Project","name":"Demo"}"#);
    let out = sysml(&[
        "--format", "json", "api", "--server", &base, "project", "p1",
    ]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["name"], "Demo");

    let base = serve_once(r#"{"@id":"p2","@type":"Project","name":"Fresh"}"#);
    let out = sysml(&["api", "--server", &base, "new-project", "Fresh"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("p2 Fresh"));

    let base = serve_once(r#"[{"@id":"c1","@type":"Commit","description":"first"}]"#);
    let out = sysml(&["api", "--server", &base, "commits", "p1"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("c1 first"));

    let base = serve_once(r#"[{"@id":"e1","@type":"PartDefinition","declaredName":"Vehicle"}]"#);
    let out = sysml(&["api", "--server", &base, "elements", "p1", "c1"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("e1 PartDefinition Vehicle"));

    let base = serve_once(r#"{"@id":"e1","@type":"PartDefinition","declaredName":"Vehicle"}"#);
    let out = sysml(&["api", "--server", &base, "element", "p1", "c1", "e1"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Vehicle"));

    // push exports the model and commits it
    let dir = temp_dir("api-push");
    let model = write(&dir, "ok.sysml", OK_MODEL);
    let base = serve_once(r#"{"@id":"c2","@type":"Commit","description":"sysml push"}"#);
    let out = sysml(&[
        "api",
        "--server",
        &base,
        "push",
        model.to_str().unwrap(),
        "--project",
        "p1",
    ]);
    assert!(
        out.status.success(),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains("element(s) as commit c2"), "{said}");
    // what was sent is the model. It used to be the model and the
    // standard library behind it -- ninety-six thousand elements to
    // somebody's server, for a model of three.
    assert!(!said.contains("library"), "{said}");

    // a model that cannot be read is reported before anything is sent
    let out = sysml(&[
        "api",
        "push",
        dir.join("nowhere.sysml").to_str().unwrap(),
        "--project",
        "p1",
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot read"));

    // a server that is not there is reported, not swallowed, and the
    // password it was reached with stays out of the message
    let out = sysml(&[
        "api",
        "--server",
        "http://user:secret@127.0.0.1:1",
        "projects",
    ]);
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("error:"), "{said}");
    assert!(!said.contains("secret"), "{said}");

    // a refusal says what the server said, which is where a model server
    // explains itself, and names the server once
    let base = serve_with("404 Not Found", r#"{"error":"no project p9"}"#);
    let out = sysml(&["api", "--server", &base, "project", "p9"]);
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("HTTP 404 Not Found"), "{said}");
    assert!(said.contains("no project p9"), "{said}");
    assert_eq!(said.matches(&base).count(), 1, "{said}");

    // and a server that accepts and then says nothing is given up on
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let held = format!("http://{}", listener.local_addr().unwrap());
    let (done, wait) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        let _ = wait.recv();
    });
    let out = sysml(&["api", "--server", &held, "--timeout", "1", "projects"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("timed out"),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    drop(done);
}

/// A diagnostic points at what it is about, and a column counted in
/// bytes puts the caret past it as soon as a name is not ASCII.
///
/// A terminal spends two columns on `あ` and one on `a`, and the model
/// this toolchain is for carries documentation in whatever language it
/// was written in.
#[test]
fn the_caret_lands_under_the_character_it_points_at() {
    /// How many columns a terminal spends on this text. Crude -- the
    /// wide ranges, and everything else one -- which is enough to tell a
    /// caret that is placed by display column from one placed by byte.
    fn columns(text: &str) -> usize {
        text.chars()
            .map(|c| match ('\u{1100}'..='\u{ffdc}').contains(&c) {
                true => 2,
                false => 1,
            })
            .sum()
    }

    let dir = temp_dir("caret");
    let bad = write(&dir, "wide.sysml", "part def 'あいう' {{{\n");
    let out = sysml(&["parse", bad.to_str().unwrap()]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    let (written, caret) = stderr
        .lines()
        .zip(stderr.lines().skip(1))
        .find(|(line, next)| line.contains("part def") && next.contains('^'))
        .expect("a caret under the line it is about");

    // Both lines carry the same gutter, so what is left of each is the
    // drawn source and the marks under it. Walking the source by the
    // width a terminal spends on each character reaches whatever the
    // caret is under -- a brace, since that is what the parser is
    // complaining about. Counted in bytes it lands three columns past
    // the line's end; counted in characters, on the quote before it.
    let gutter = |line: &str| line.find("| ").expect("a gutter") + 2;
    let source = &written[gutter(written)..];
    let at = caret.find('^').unwrap() - gutter(caret);
    let mut column = 0;
    let pointed = source.chars().find(|c| {
        let here = column;
        column += columns(&c.to_string());
        here == at
    });
    assert_eq!(pointed, Some('{'), "{stderr}");
}

/// A directory that is not there is unreadable, not empty: the walk
/// takes what it can reach, so the two used to read the same.
#[test]
fn corpus_tells_an_unreadable_directory_from_an_empty_one() {
    let out = sysml(&["corpus", "/nowhere/at/all"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("cannot read"),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );

    let empty = temp_dir("corpus-empty");
    let out = sysml(&["corpus", empty.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no .sysml/.kerml files"),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// One unreadable file in a directory refuses the directory, and the
/// message says which file: a corpus of hundreds is otherwise a refusal
/// with nothing to act on.
#[cfg(unix)]
#[test]
fn an_unreadable_file_in_a_directory_is_named() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("check-names-the-file");
    write(&dir, "ok.sysml", OK_MODEL);
    let hidden = write(&dir, "hidden.sysml", OK_MODEL);
    std::fs::set_permissions(&hidden, std::fs::Permissions::from_mode(0o000)).unwrap();
    let out = sysml(&["check", dir.to_str().unwrap()]);
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("hidden.sysml"), "{said}");
    std::fs::set_permissions(&hidden, std::fs::Permissions::from_mode(0o644)).unwrap();
}

/// The JSON is one document on stdout and the exit code agrees with it,
/// whichever subcommand a program is running.
#[test]
fn a_program_reads_one_document_and_an_exit_code_that_agrees() {
    let dir = temp_dir("json-shape");
    let broken = write(&dir, "broken.sysml", "part def {{{\n");
    let json = |out: &Output| -> serde_json::Value {
        serde_json::from_slice(&out.stdout).expect("stdout is one JSON document")
    };

    // --tree writes the tree where it cannot spoil the document
    let out = sysml(&[
        "--format",
        "json",
        "parse",
        "--tree",
        broken.to_str().unwrap(),
    ]);
    assert_eq!(json(&out)["command"], "parse");
    assert!(String::from_utf8_lossy(&out.stderr).contains("SOURCE_FILE"));

    // counting the elements of a file that did not parse is counting
    // what the parser guessed at
    let out = sysml(&["stats", broken.to_str().unwrap()]);
    assert!(!out.status.success());
    let out = sysml(&["--format", "json", "stats", broken.to_str().unwrap()]);
    assert!(!out.status.success());
    assert_eq!(json(&out)["ok"], false);

    // `fmt --write` refuses a file it could not parse, and says which
    let out = sysml(&[
        "--format",
        "json",
        "fmt",
        "--check",
        "--write",
        broken.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    let v = json(&out);
    assert_eq!(v["ok"], false);
    assert_eq!(v["broken"].as_array().unwrap().len(), 1);
    assert_eq!(v["unformatted"].as_array().unwrap().len(), 0);
}

/// A path is bytes, and `check` used to place its findings by re-reading
/// the file under a lossy rendering of that path -- which named no file
/// at all, so every finding landed at 1:1.
#[cfg(unix)]
#[test]
fn findings_are_placed_in_a_file_whose_name_is_not_utf8() {
    use std::os::unix::ffi::OsStrExt;
    let dir = temp_dir("check-not-utf8");
    let path = dir.join(std::ffi::OsStr::from_bytes(b"caf\xffe.sysml"));
    std::fs::write(
        &path,
        "package P {\n\tpart def A;\n\tpart b : Missing;\n}\n",
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_sysml"))
        .args(["--format", "json", "check"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["unresolved"][0]["name"], "Missing", "{v}");
    assert_eq!(v["unresolved"][0]["line"], 3, "{v}");
    assert_eq!(v["unresolved"][0]["column"], 11, "{v}");
}

/// Two definitions can share a declared name -- one in the library, one
/// in the model, or two in the model itself. Taking whichever came
/// first drew a picture of something the modeller had not asked about.
#[test]
fn a_name_that_two_definitions_share_is_settled_before_it_is_drawn() {
    let dir = temp_dir("internal-ambiguous");
    let lib = write(
        &dir,
        "lib.sysml",
        "package L {\n\tpart def Car {\n\t\tpart fromLibrary;\n\t}\n}\n",
    );
    let model = write(
        &dir,
        "car.sysml",
        "package M {\n\tpart def Car {\n\t\tpart fromModel;\n\t}\n}\n",
    );

    // the model's own `Car` is the one it meant
    let out = sysml(&[
        "diagram",
        model.to_str().unwrap(),
        "--library",
        lib.to_str().unwrap(),
        "--internal",
        "Car",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("fromModel"), "{stdout}");
    assert!(!stdout.contains("fromLibrary"), "{stdout}");

    // and a qualified name says which when the plain one will not
    let out = sysml(&[
        "diagram",
        model.to_str().unwrap(),
        "--library",
        lib.to_str().unwrap(),
        "--internal",
        "L::Car",
    ]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("fromLibrary"));

    // two of them in the model itself is a choice nobody can make, and
    // it is said rather than made quietly
    let both = write(
        &dir,
        "both.sysml",
        "package M {\n\tpart def Car {\n\t\tpart fromModel;\n\t}\n}\n\
         package N {\n\tpart def Car {\n\t\tpart fromOther;\n\t}\n}\n",
    );
    let out = sysml(&["diagram", both.to_str().unwrap(), "--internal", "Car"]);
    assert!(out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("names 2 elements"), "{said}");
    assert!(said.contains("qualified name"), "{said}");
}

/// A root package of one's own named after one of the standard
/// library's still resolves -- each side reads its own -- so `check`
/// says so rather than counting it among the unresolved names.
#[test]
fn check_reports_a_package_named_after_a_library_one() {
    let dir = temp_dir("check-collision");
    write(
        &dir,
        "lib.sysml",
        "standard library package Requirements {\n    part def RequirementCheck;\n}\n",
    );
    write(
        &dir,
        "mine.sysml",
        "package Requirements {\n    part def Safe;\n}\n",
    );
    // this writes its own `standard library package Requirements` to
    // collide with, so the real one must stay out of the way
    let out = sysml(&["--no-library", "check", dir.to_str().unwrap()]);
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("`Requirements` is also a root package of the standard library"),
        "{stderr}"
    );
    assert!(stderr.contains("mine.sysml:1:9"), "{stderr}");

    let out = sysml(&[
        "--no-library",
        "--format",
        "json",
        "check",
        dir.to_str().unwrap(),
    ]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["collisions"].as_array().unwrap().len(), 1);
    assert!(v["collisions"][0]["path"]
        .as_str()
        .unwrap()
        .ends_with("mine.sysml"));
}

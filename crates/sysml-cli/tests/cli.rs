//! End-to-end tests running the `sysml` binary (every subcommand and its
//! error paths).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

    // parse diagnostics still get printed while exporting
    let bad = write(&dir, "bad.sysml", "part def {{{\n");
    let out = sysml(&["export", bad.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("error:"));

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
    let out = sysml(&[
        "export",
        model.to_str().unwrap(),
        "--library",
        lib.to_str().unwrap(),
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
         \tpart def OrderPlanner {\n\t\tattribute threshold : Real;\n\t}\n\
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
    assert!(String::from_utf8_lossy(&out.stderr).contains("1 struct(s)"));
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
         \tmetadata def rust { attribute path : String; attribute takesSelf : String; }\n\
         \taction def Orphan { @rust { :>> path = \"elsewhere::Api::orphan\"; :>> takesSelf = \"&self\"; } }\n\
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
    assert!(stderr.contains("unresolved `Missing0`"), "{stderr}");
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
    let out = sysml(&["check", ok.to_str().unwrap()]);
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

    // a file that cannot be read is a finding, not a stray line
    let out = sysml(&["--format", "json", "parse", "no-such-file.sysml"]);
    assert!(!out.status.success());
    assert!(json(&out)["files"][0]["unreadable"].is_string());

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
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
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

    // a server that is not there is reported, not swallowed
    let out = sysml(&["api", "--server", "http://127.0.0.1:1", "projects"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("error:"));
}

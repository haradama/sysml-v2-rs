//! `import-code`: a package from a surface, or from the importer for a
//! language -- found the way Doxygen finds a filter, by name -- and
//! checked either way before it is written.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn sysml(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sysml"))
        .args(args)
        .output()
        .unwrap()
}

/// The tool, with a directory in front of `PATH` -- where an importer is
/// found by name.
fn sysml_finding_in(dir: &Path, args: &[&str]) -> Output {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![dir.to_path_buf()];
    paths.extend(std::env::split_paths(&path));
    Command::new(env!("CARGO_BIN_EXE_sysml"))
        .env("PATH", std::env::join_paths(paths).unwrap())
        .args(args)
        .output()
        .unwrap()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sysml-cli-import-code-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

/// The Rust importer's fixture: a crate's rustdoc JSON.
fn rustdoc() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../sysml-rust/tests/fixtures/inventory_store.rustdoc.json")
}

/// The Python importer, and the module and package beside it.
fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/import-python")
}

const SURFACE: &str = r#"{
    "language": "python",
    "module": "inventory",
    "items": [
        { "kind": "record", "name": "StockQuery", "path": "inventory.StockQuery",
          "fields": [ { "name": "sku", "type": "String" },
                      { "name": "filter", "type": "Boolean" } ] },
        { "kind": "function", "name": "count", "path": "inventory.count",
          "parameters": [ { "name": "of", "type": "StockQuery", "multiplicity": "many" } ],
          "result": { "type": "Integer" } },
        { "kind": "function", "name": "weigh", "path": "inventory.weigh",
          "parameters": [ { "name": "of", "type": "Basket" } ] }
    ]
}"#;

#[test]
fn a_surface_file_is_written_as_a_checked_package() {
    let dir = temp_dir("surface");
    let surface = write(&dir, "inventory.json", SURFACE);
    let out_path = dir.join("inventory.sysml");
    let out = sysml(&[
        "import-code",
        surface.to_str().unwrap(),
        "--package",
        "Warehouse",
        "-o",
        out_path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    // what was written -- the binding's metadata definition among them
    // -- what could not be, and that the rest resolves
    assert!(
        text(&out.stderr).contains("wrote 3 definition(s), 1 item(s) not imported"),
        "{}",
        text(&out.stderr)
    );
    let written = std::fs::read_to_string(&out_path).unwrap();
    assert!(written.contains("package Warehouse {"), "{written}");
    assert!(
        written.contains("attribute 'filter' : Boolean;"),
        "{written}"
    );
    assert!(written.contains("in 'of' : StockQuery[*];"), "{written}");
    assert!(
        written
            .contains("//   inventory.weigh -- names `Basket`, which the surface does not declare"),
        "{written}"
    );

    // to stdout without -o, named after the module
    let out = sysml(&["import-code", surface.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("package InventoryApi {"));

    // a missing file, and text that is not a surface, are named errors
    let out = sysml(&["import-code", dir.join("absent.json").to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("cannot read"),
        "{}",
        text(&out.stderr)
    );
    let bad = write(&dir, "bad.json", r#"{ "language": "python" }"#);
    let out = sysml(&["import-code", bad.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("not a surface") && text(&out.stderr).contains("module"),
        "{}",
        text(&out.stderr)
    );
    // one surface is one package
    let out = sysml(&[
        "import-code",
        surface.to_str().unwrap(),
        bad.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("one JSON file"),
        "{}",
        text(&out.stderr)
    );
}

/// The package is checked before it is written, whichever the importer
/// wrote: a name that resolves to nothing in a package somebody else's
/// tool wrote is what this toolchain is for catching.
#[test]
fn what_the_importer_wrote_is_checked_before_it_is_written() {
    let dir = temp_dir("checked");
    let surface = write(&dir, "inventory.json", SURFACE);
    // without the library nothing in the package resolves, and the
    // package is still written -- so that what is wrong can be looked at
    // -- with the exit code saying it is wrong
    let out_path = dir.join("bare.sysml");
    let out = sysml(&[
        "--no-library",
        "import-code",
        surface.to_str().unwrap(),
        "-o",
        out_path.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    let said = text(&out.stderr);
    assert!(
        said.contains("`ScalarValues` resolves to nothing at 3:17 of what was written"),
        "{said}"
    );
    assert!(
        said.contains("finding(s) in what the importer wrote"),
        "{said}"
    );
    assert!(std::fs::read_to_string(&out_path)
        .unwrap()
        .contains("package InventoryApi {"));

    // an importer may write SysML itself, as a Doxygen filter writes
    // what Doxygen reads; `sysml fmt` is one that writes the file it is
    // given, which is how a dangling name and a broken file reach here
    let via = format!("{} fmt", env!("CARGO_BIN_EXE_sysml"));
    let dangling = write(
        &dir,
        "dangling.sysml",
        "package P {\n\tpart def A :> Nowhere;\n}\n",
    );
    let out = sysml(&["import-code", "--via", &via, dangling.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("`Nowhere` resolves to nothing at 2:19 of what was written"),
        "{}",
        text(&out.stderr)
    );
    assert!(text(&out.stdout).contains("part def A :> Nowhere;"));
    let broken = write(&dir, "broken.sysml", "package P {\n\tpart def A;\n");
    let out = sysml(&["import-code", "--via", &via, broken.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("expected R_BRACE, found EOF at 3:1 of what was written"),
        "{}",
        text(&out.stderr)
    );
    // and one that is whole passes, as the Rust importer's output does
    let via = format!("{} import-rust", env!("CARGO_BIN_EXE_sysml"));
    let out = sysml(&["import-code", "--via", &via, rustdoc().to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("package InventoryStoreApi {"));
    // the name of such a package is in what the importer wrote
    let out = sysml(&[
        "import-code",
        "--via",
        &via,
        rustdoc().to_str().unwrap(),
        "--package",
        "Warehouse",
    ]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("wrote SysML itself"),
        "{}",
        text(&out.stderr)
    );

    // the library it is checked against is the one every front end reads
    let out = Command::new(env!("CARGO_BIN_EXE_sysml"))
        .env("SYSML_LIBRARY_PATH", dir.join("nowhere"))
        .args(["import-code", surface.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("cannot read"),
        "{}",
        text(&out.stderr)
    );
}

/// The importer is found the way `cargo` and `git` find a subcommand:
/// `sysml-import-<lang>` on `PATH`, and the language is the extension
/// unless `--lang` says otherwise.
#[cfg(unix)]
#[test]
fn an_importer_is_found_by_name_and_runs_on_the_inputs() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("found");
    // an importer for `.rustdoc` files, which writes the surface
    let importer = write(
        &dir,
        "sysml-import-rustdoc",
        &format!(
            "#!/bin/sh\necho 'reading rustdoc' >&2\nexec \"{}\" import-rust --surface \"$@\"\n",
            env!("CARGO_BIN_EXE_sysml")
        ),
    );
    std::fs::set_permissions(&importer, std::fs::Permissions::from_mode(0o755)).unwrap();
    let input = dir.join("inventory.rustdoc");
    std::fs::copy(rustdoc(), &input).unwrap();

    // by the extension alone
    let out = sysml_finding_in(&dir, &["import-code", input.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("package InventoryStoreApi {"));
    // what the importer said for itself is said here too
    assert!(
        text(&out.stderr).contains("reading rustdoc"),
        "{}",
        text(&out.stderr)
    );
    // by name
    let out = sysml_finding_in(
        &dir,
        &["import-code", "--lang", "rustdoc", input.to_str().unwrap()],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    // an importer nothing provides
    let out = sysml_finding_in(
        &dir,
        &["import-code", "--lang", "klingon", input.to_str().unwrap()],
    );
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("cannot run `sysml-import-klingon`"),
        "{}",
        text(&out.stderr)
    );
    // one that wrote something that is not text
    let binary = write(&dir, "sysml-import-bin", "#!/bin/sh\nprintf '\\377'\n");
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = sysml_finding_in(
        &dir,
        &["import-code", "--lang", "bin", input.to_str().unwrap()],
    );
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("not text"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn what_cannot_name_an_importer_is_refused_with_the_way_to() {
    let dir = temp_dir("unnamed");
    let py = write(&dir, "a.py", "");
    let rs = write(&dir, "b.rs", "");
    let bare = write(&dir, "Makefile", "");
    for (inputs, why) in [
        (vec![py.as_path(), rs.as_path()], "different extensions"),
        (vec![bare.as_path()], "no extension"),
    ] {
        let mut args = vec!["import-code"];
        args.extend(inputs.iter().map(|path| path.to_str().unwrap()));
        let out = sysml(&args);
        assert!(!out.status.success());
        assert!(text(&out.stderr).contains(why), "{}", text(&out.stderr));
        assert!(
            text(&out.stderr).contains("--lang or --via"),
            "{}",
            text(&out.stderr)
        );
    }
    // a command with nothing in it
    let out = sysml(&["import-code", "--via", "", py.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("names no command"),
        "{}",
        text(&out.stderr)
    );
    // and one that fails is reported as the failure it was
    let via = format!("{} check", env!("CARGO_BIN_EXE_sysml"));
    let out = sysml(&[
        "import-code",
        "--via",
        &via,
        dir.join("absent.sysml").to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("failed (exit status: 1)"),
        "{}",
        text(&out.stderr)
    );
}

/// The worked example, held to the file beside it: what the Python
/// importer writes, through `import-code`, is what `inventory.sysml`
/// says it is -- so a change to the writer or to the importer shows.
#[test]
fn the_python_example_imports_as_the_file_beside_it_says() {
    let example = example();
    let dir = temp_dir("python");
    let out_path = dir.join("inventory.sysml");
    let via = format!("python3 {}", example.join("sysml-import-python").display());
    let out = sysml(&[
        "import-code",
        "--via",
        &via,
        example.join("inventory.py").to_str().unwrap(),
        "-o",
        out_path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let written = std::fs::read_to_string(&out_path).unwrap();
    let expected = std::fs::read_to_string(example.join("inventory.sysml")).unwrap();
    assert_eq!(
        written, expected,
        "regenerate examples/import-python/inventory.sysml"
    );
    // and the README's claims about it hold
    assert!(written.contains("attribute 'filter' : Boolean;"));
    assert!(written.contains("attribute def PyBytes :> String"));
    assert!(written.contains(":>> isAsync = true;"));
    assert!(written.contains(":>> isFallible = true;"));
    assert!(written.contains("//   StockQuery.history -- no SysML shape for `list[list[int]]`"));
    assert!(written.contains("//   inventory.pick -- `index` has no annotation"));
    assert!(
        text(&out.stderr).contains("wrote 10 definition(s), 5 item(s) not imported"),
        "{}",
        text(&out.stderr)
    );
    // found on PATH under its own name, the same package comes out
    #[cfg(unix)]
    {
        let out = sysml_finding_in(
            &example,
            &[
                "import-code",
                "--lang",
                "python",
                example.join("inventory.py").to_str().unwrap(),
            ],
        );
        assert!(out.status.success(), "{}", text(&out.stderr));
        assert_eq!(text(&out.stdout), expected);
    }
}

#[test]
fn the_schema_is_printed_and_import_rust_shows_its_surface() {
    let dir = temp_dir("schema");
    let out = sysml(&["import-code", "--schema"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let schema: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(schema["properties"]["language"].is_object(), "{schema}");
    assert!(schema["$defs"]["Callable"].is_object(), "{schema}");
    let at = dir.join("surface.schema.json");
    let out = sysml(&["import-code", "--schema", "-o", at.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(text(&out.stderr).contains("the schema of a surface"));
    assert!(at.is_file());

    // the surface the Rust importer reads off a crate is the JSON
    // `import-code` reads, so the two are the same package
    let out = sysml(&["import-rust", "--surface", rustdoc().to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let surface: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(surface["language"], "rust");
    assert_eq!(surface["unit"], "crate");
    assert_eq!(surface["items"][0]["kind"], "record");
    let written = write(&dir, "inventory_store.json", &text(&out.stdout));
    let through = sysml(&["import-code", written.to_str().unwrap()]);
    assert!(through.status.success(), "{}", text(&through.stderr));
    let direct = sysml(&["import-rust", rustdoc().to_str().unwrap()]);
    assert_eq!(text(&through.stdout), text(&direct.stdout));

    // to a file, and refused where the rustdoc is not rustdoc
    let at = dir.join("inventory_store.surface.json");
    let out = sysml(&[
        "import-rust",
        "--surface",
        rustdoc().to_str().unwrap(),
        "-o",
        at.to_str().unwrap(),
    ]);
    assert!(out.status.success());
    assert!(at.is_file());
    assert!(
        text(&out.stderr).contains("wrote 11 item(s), 7 not imported"),
        "{}",
        text(&out.stderr)
    );
    let bad = write(&dir, "bad.json", "{}");
    let out = sysml(&["import-rust", "--surface", bad.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("missing its item index"),
        "{}",
        text(&out.stderr)
    );
}

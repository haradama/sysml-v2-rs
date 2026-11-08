//! `codegen`: code in any language from a model, through the generator
//! for that language -- found the way Doxygen finds a filter, by name --
//! handed the plan on standard input.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn sysml(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sysml"))
        .args(args)
        .output()
        .unwrap()
}

/// The tool, with a directory in front of `PATH` -- where a generator
/// is found by name.
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
    let dir = std::env::temp_dir().join(format!("sysml-cli-codegen-{name}"));
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

/// The Python generator, and the model and module beside it.
fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/generate-python")
}

/// The Arduino example, whose generated Rust is checked in.
fn arduino() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/arduino-uno")
}

const MODEL: &str = "package P {\n\
    \tprivate import ScalarValues::*;\n\
    \tpart def Wheel;\n\
    \tpart def Car {\n\
    \t\tattribute mass : Real = 1200.0;\n\
    \t\tpart wheels : Wheel[4];\n\
    \t}\n\
    }\n";

/// A generator is any program that reads the plan and writes the code:
/// one that writes its input back shows what it was handed.
#[test]
fn the_generator_is_handed_the_plan_and_answers_with_the_code() {
    let dir = temp_dir("plan");
    let model = write(&dir, "car.sysml", MODEL);
    let echo = "python3 -c \"import sys; sys.stdout.write(sys.stdin.read())\"";
    let out = sysml(&["codegen", "--via", echo, model.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let plan: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(plan["checked"]["ok"], true);
    assert_eq!(plan["definitions"][1]["of"], "P::Car");
    assert_eq!(
        plan["definitions"][1]["features"][1]["multiplicity"]["lower"],
        4
    );

    // to a file, with a count of what came back
    let out_path = dir.join("car.txt");
    let out = sysml(&[
        "codegen",
        "--via",
        echo,
        model.to_str().unwrap(),
        "-o",
        out_path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(
        text(&out.stderr).contains("wrote 1 line(s) from 2 definition(s)"),
        "{}",
        text(&out.stderr)
    );
    assert!(out_path.is_file());

    // what the generator says for itself is said here
    let talkative = "python3 -c \"import sys; print('reading', file=sys.stderr); print('code')\"";
    let out = sysml(&["codegen", "--via", talkative, model.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "code\n");
    assert!(
        text(&out.stderr).contains("reading"),
        "{}",
        text(&out.stderr)
    );
}

/// What does not resolve is refused before any generator runs, since
/// code generated from it would quietly leave out whatever the model
/// misspelt.
#[test]
fn a_model_that_does_not_resolve_is_refused_before_the_generator_runs() {
    let dir = temp_dir("refused");
    let model = write(
        &dir,
        "car.sysml",
        "package P {\n\tpart def Car { part wheels : Wheeel[4]; }\n}\n",
    );
    let out = sysml(&["codegen", "--via", "python3 -c 1", model.to_str().unwrap()]);
    assert!(!out.status.success());
    let said = text(&out.stderr);
    assert!(said.contains("unresolved `Wheeel`"), "{said}");
    assert!(said.contains("`sysml check` says where"), "{said}");
    assert!(out.stdout.is_empty());
    let broken = write(&dir, "broken.sysml", "package P {\n\tpart def Car;\n");
    let out = sysml(&["codegen", "--via", "python3 -c 1", broken.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("does not parse"),
        "{}",
        text(&out.stderr)
    );
    // and files that cannot be read, on either side of `--library`
    let out = sysml(&[
        "codegen",
        "--lang",
        "python",
        dir.join("absent.sysml").to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("cannot read"),
        "{}",
        text(&out.stderr)
    );
    let model = write(&dir, "ok.sysml", MODEL);
    let out = sysml(&[
        "codegen",
        "--lang",
        "python",
        model.to_str().unwrap(),
        "--library",
        dir.join("absent.sysml").to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("cannot read"),
        "{}",
        text(&out.stderr)
    );
}

/// A generator is found the way `cargo` and `git` find a subcommand:
/// `sysml-gen-<lang>` on `PATH`; and one that fails, or writes what is
/// not text, is reported as that.
#[cfg(unix)]
#[test]
fn a_generator_is_found_by_name_and_its_failures_are_named() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("found");
    let model = write(&dir, "car.sysml", MODEL);
    let executable = |name: &str, script: &str| {
        let at = write(&dir, name, script);
        std::fs::set_permissions(&at, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    executable(
        "sysml-gen-count",
        "#!/bin/sh\npython3 -c \"import json, sys; print(len(json.load(sys.stdin)['definitions']), 'definitions')\"\n",
    );
    executable(
        "sysml-gen-sulky",
        "#!/bin/sh\necho 'not today' >&2\nexit 3\n",
    );
    executable("sysml-gen-bin", "#!/bin/sh\nprintf '\\377'\n");

    let out = sysml_finding_in(
        &dir,
        &["codegen", "--lang", "count", model.to_str().unwrap()],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "2 definitions\n");

    let out = sysml_finding_in(
        &dir,
        &["codegen", "--lang", "sulky", model.to_str().unwrap()],
    );
    assert!(!out.status.success());
    let said = text(&out.stderr);
    assert!(said.contains("not today"), "{said}");
    assert!(
        said.contains("`sysml-gen-sulky` failed (exit status: 3)"),
        "{said}"
    );

    let out = sysml_finding_in(&dir, &["codegen", "--lang", "bin", model.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("not text"),
        "{}",
        text(&out.stderr)
    );

    let out = sysml_finding_in(
        &dir,
        &["codegen", "--lang", "klingon", model.to_str().unwrap()],
    );
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("cannot run `sysml-gen-klingon`"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn a_command_is_split_as_a_shell_splits_one() {
    let dir = temp_dir("split");
    let model = write(&dir, "car.sysml", MODEL);
    // a quote that never closes is not a command
    let out = sysml(&["codegen", "--via", "python3 'x", model.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("never closes"),
        "{}",
        text(&out.stderr)
    );
    // and one with nothing in it names nothing
    let out = sysml(&["codegen", "--via", "", model.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("names no command"),
        "{}",
        text(&out.stderr)
    );
    // the same splitting reads an importer's command, so a path with a
    // space in it can be quoted there too
    let spaced = dir.join("with space");
    std::fs::create_dir_all(&spaced).unwrap();
    let importer = spaced.join("sysml-import-python");
    std::fs::copy(
        example().join("../import-python/sysml-import-python"),
        &importer,
    )
    .unwrap();
    let via = format!("python3 '{}'", importer.display());
    let out = sysml(&[
        "import-code",
        "--via",
        &via,
        example()
            .join("../import-python/inventory.py")
            .to_str()
            .unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("package InventoryApi {"));
}

/// `--lang rust` is the generator built in, and writes what `rustgen`
/// writes -- held to the Arduino example's checked-in Rust, as the
/// example's own CI job holds `rustgen`.
#[test]
fn rust_is_the_generator_built_in() {
    let arduino = arduino();
    let model = |name: &str| arduino.join("model").join(name);
    let out = sysml(&[
        "codegen",
        "--lang",
        "rust",
        model("blink.sysml").to_str().unwrap(),
        model("requirements.sysml").to_str().unwrap(),
        model("verification.sysml").to_str().unwrap(),
        "--library",
        model("hardware.sysml").to_str().unwrap(),
        "--library",
        model("middleware.sysml").to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let expected = std::fs::read_to_string(arduino.join("src/generated.rs")).unwrap();
    assert_eq!(text(&out.stdout), expected);
}

/// The worked example, held to the file beside it: what the Python
/// generator writes for the thermostat, through `codegen`, is what
/// `thermostat.py` says it is -- so a change to the plan or to the
/// generator shows.
#[test]
fn the_python_example_generates_the_file_beside_it() {
    let example = example();
    let dir = temp_dir("python");
    let out_path = dir.join("thermostat.py");
    let via = format!("python3 {}", example.join("sysml-gen-python").display());
    let out = sysml(&[
        "codegen",
        "--via",
        &via,
        example.join("model").to_str().unwrap(),
        "-o",
        out_path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let written = std::fs::read_to_string(&out_path).unwrap();
    let expected = std::fs::read_to_string(example.join("thermostat.py")).unwrap();
    assert_eq!(
        written, expected,
        "regenerate examples/generate-python/thermostat.py"
    );
    // and the README's claims about it hold
    assert!(written.contains("class Celsius(float):"));
    assert!(written.contains("class Mode(Enum):"));
    assert!(written.contains("from sensors import Thermometer"));
    assert!(written.contains("class Room(Space):"));
    assert!(written.contains("target: Celsius = 21.0"));
    assert!(written.contains("log: list[str] = field(default_factory=list)"));
    assert!(written.contains("class RegulationState(Enum):"));
    assert!(written.contains("def check_comfort(room: Room) -> bool:"));
    assert!(written.contains("# left open"));
    assert!(
        text(&out.stderr).contains("thing(s) left open"),
        "{}",
        text(&out.stderr)
    );
    // found on PATH under its own name, the same module comes out
    #[cfg(unix)]
    {
        let out = sysml_finding_in(
            &example,
            &[
                "codegen",
                "--lang",
                "python",
                example.join("model").to_str().unwrap(),
            ],
        );
        assert!(out.status.success(), "{}", text(&out.stderr));
        assert_eq!(text(&out.stdout), expected);
    }
}

#[test]
fn the_plan_has_a_schema_a_generator_is_written_against() {
    let out = sysml(&["plan", "--schema"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let schema: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(schema["properties"]["definitions"].is_object(), "{schema}");
    assert!(schema["$defs"]["Transition"].is_object(), "{schema}");
}

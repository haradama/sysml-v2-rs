//! The standard library, without being handed one.
//!
//! Almost nothing in a SysML model resolves without it. Until there was a
//! copy built into the binary, getting one meant cloning a repository
//! whose history is two gigabytes for one and a third megabytes of model
//! -- so `cargo install sysml-cli` gave you a tool that reported every
//! name in every model as unresolved until you did.
//!
//! The rest of the suite checks the other half: `--no-library`, which is
//! what a model looks like to a tool that cannot find one.

use std::path::PathBuf;
use std::process::{Command, Output};

fn sysml(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sysml"))
        .args(args)
        .output()
        .unwrap()
}

/// The same, with the environment variable a launcher sets when it has
/// nowhere to put a flag.
fn sysml_with_library(library: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sysml"))
        .args(args)
        .env("SYSML_LIBRARY_PATH", library)
        .output()
        .unwrap()
}

fn written(name: &str, text: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("sysml-cli-bundled-library");
    std::fs::create_dir_all(&dir).unwrap();
    let at = dir.join(name);
    std::fs::write(&at, text).unwrap();
    at
}

/// A model that reaches into the library, checked with no path given.
///
/// `ISQ::MassValue` is in the Quantities and Units domain library and is
/// reached through two levels of import, so a copy that is there but
/// truncated fails this as surely as no copy at all.
#[test]
fn a_model_resolves_against_the_copy_built_in() {
    let model = written(
        "reaches.sysml",
        "package P {\n\
         \tprivate import ISQ::*;\n\
         \tpart def Tank {\n\
         \t\tattribute fuel : MassValue;\n\
         \t}\n\
         }\n",
    );
    let out = sysml(&["--format", "json", "check", model.to_str().unwrap()]);
    let said: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(said["ok"], true, "{said}");
    assert_eq!(said["unresolved"].as_array().unwrap().len(), 0, "{said}");
    assert!(
        said["library"]
            .as_str()
            .is_some_and(|it| it.starts_with("built in")),
        "{said}"
    );
    assert!(out.status.success());
}

/// And the constraints the specification states are put to it, which
/// they are not for a model with no library to be asked against.
///
/// This is the half that fails quietly: every name resolves either way,
/// and what is missing is that nothing was asked.
#[test]
fn the_constraints_are_put_to_it_too() {
    let model = written("asked.sysml", "package P {\n\tpart def Engine;\n}\n");
    let out = sysml(&["--format", "json", "check", model.to_str().unwrap()]);
    let said: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        said["rules"]["held"].as_u64().unwrap() > 0,
        "the constraints were not asked: {said}"
    );
}

/// `--no-library` is what says otherwise, and it says it for every
/// subcommand rather than for the one that happened to need it.
#[test]
fn no_library_is_the_way_to_say_otherwise() {
    let model = written(
        "bare.sysml",
        "package P {\n\
         \tprivate import ISQ::*;\n\
         \tpart def Tank {\n\
         \t\tattribute fuel : MassValue;\n\
         \t}\n\
         }\n",
    );
    let out = sysml(&[
        "--no-library",
        "--format",
        "json",
        "check",
        model.to_str().unwrap(),
    ]);
    let said: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(said["ok"], false, "{said}");
    assert!(!said["unresolved"].as_array().unwrap().is_empty(), "{said}");
    assert!(said["library"].is_null(), "none answered: {said}");
}

/// A library named on the command line wins, and is not loaded twice
/// over the copy built in.
///
/// `sysml check model/ path/to/sysml.library` has named it as a plain
/// path since before there was anything to fall back on. A second copy
/// added over it would make every name in it answer twice, so what is
/// already loaded is what settles it.
#[test]
fn what_was_named_wins_and_is_not_doubled() {
    let library = sysml_corpus::library();
    let model = written("named.sysml", "package P {\n\tpart def Engine;\n}\n");
    let out = sysml(&[
        "--format",
        "json",
        "check",
        model.to_str().unwrap(),
        library.to_str().unwrap(),
    ]);
    let said: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(said["ok"], true, "{said}");
    assert_eq!(said["library"], "given", "{said}");
    assert_eq!(
        said["collisions"].as_array().unwrap().len(),
        0,
        "a second copy would collide with the first: {said}"
    );
}

/// And `--version` says which release it is carrying, because the
/// library moves with the specification and an answer nobody can
/// reproduce is worth less than one they can.
#[test]
fn the_version_names_the_release_it_carries() {
    let out = sysml(&["--version"]);
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        said.contains(sysml_stdlib::RELEASE),
        "`--version` says {said:?}, which does not name {}",
        sysml_stdlib::RELEASE
    );
}

/// `SYSML_LIBRARY_PATH` names one too, which is how a launcher with
/// nowhere to put a flag says it -- the language server and the MCP
/// server read the same variable, and a shell that exports it once
/// means it for all three.
#[test]
fn the_environment_names_a_library_as_the_command_line_does() {
    let dir = std::env::temp_dir().join("sysml-cli-library-by-environment");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("tiny.sysml"),
        "package Tiny {\n\tpart def Widget;\n}\n",
    )
    .unwrap();
    let model = written(
        "borrowed.sysml",
        "package P {\n\tprivate import Tiny::*;\n\tpart w : Widget;\n}\n",
    );
    let out = sysml_with_library(
        &dir,
        &["--format", "json", "check", model.to_str().unwrap()],
    );
    let said: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(said["ok"], true, "{said}");
    assert_eq!(said["unresolved"].as_array().unwrap().len(), 0, "{said}");
    // and the answer says which library it was, since a program cannot
    // tell one that was found from one that was built in
    assert_eq!(said["library"], dir.to_str().unwrap(), "{said}");
}

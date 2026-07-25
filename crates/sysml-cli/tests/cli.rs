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

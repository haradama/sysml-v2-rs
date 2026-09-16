//! Standard interchange, read rather than written.
//!
//! `sysml export` writes the JSON the SysML v2 API & Services standard
//! defines, and `sysml_interchange::from_json` reads it back -- round-trip
//! tested over the whole standard library since it was written, and
//! called from no front end at all. A document off a model server had
//! nowhere to go, and `sysml api` could fetch one. `sysml import` is
//! where it goes.
//!
//! Two things it can do with what it reads, and the boundary between them
//! is what a model on its own is enough for: counting the elements, and
//! drawing them. Checking the specification's constraints is not among
//! them -- that wants names resolved against files, and a document is not
//! files.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Mutex, OnceLock};

fn sysml(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sysml"))
        .args(args)
        .output()
        .unwrap()
}

fn dir() -> PathBuf {
    let at = std::env::temp_dir().join("sysml-cli-interchange-read");
    std::fs::create_dir_all(&at).unwrap();
    at
}

fn written(name: &str, text: &str) -> PathBuf {
    let at = dir().join(name);
    std::fs::write(&at, text).unwrap();
    at
}

/// The four tests that drive a whole document take it in turns.
///
/// A document that carries the library is not cheap to drive: `export`
/// peaks around 6.7 GB writing the 754 MB of it, and `import` around
/// 6.9 GB reading it back. The harness runs tests on as many threads as
/// the machine has cores, so four at once asked for some 27 GB -- more
/// than a CI runner has, and what a runner does about that is kill the
/// process. No test failed; the job came back as signalled, twice, and
/// said nothing about why.
///
/// One at a time is about 7 GB, which fits. What this costs is that the
/// four run one after another, and they are the slow ones.
fn in_turn<T>(what: impl FnOnce() -> T) -> T {
    static TURN: Mutex<()> = Mutex::new(());
    // A test that panicked while holding this poisoned it, and the next
    // one failing on the poison would report the first one's mistake
    // under its own name.
    let _held = TURN.lock().unwrap_or_else(|held| held.into_inner());
    what()
}

/// The document those four share, written once.
///
/// Every one of them needs the library carried -- `import` refuses a
/// document that points outside itself, which is what the test below is
/// about -- and the library is all but the whole of the 754 MB. So they
/// ask the same model of it: two part definitions, one specializing the
/// other, and an `Engine` for the drawing to show.
fn whole_document() -> PathBuf {
    static AT: OnceLock<PathBuf> = OnceLock::new();
    AT.get_or_init(|| {
        exported(
            "whole",
            "package Vehicles {\n  part def PowerSource;\n  part def Engine :> PowerSource;\n}\n",
        )
    })
    .clone()
}

/// A model, and the document `export` writes for it.
///
/// `--include-library` because a document without it names the library
/// elements its definitions specialize and does not carry them; the test
/// below is about exactly that.
fn exported(name: &str, model: &str) -> PathBuf {
    let source = written(&format!("{name}.sysml"), model);
    let json = dir().join(format!("{name}.json"));
    let out = sysml(&[
        "export",
        source.to_str().unwrap(),
        "--include-library",
        "-o",
        json.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    json
}

#[test]
fn a_document_this_tool_wrote_is_a_document_it_reads() {
    let out = in_turn(|| {
        let json = whole_document();
        sysml(&["import", json.to_str().unwrap()])
    });
    assert!(out.status.success());
    let said = String::from_utf8_lossy(&out.stdout);
    // the counts are the model's, the last number the document's: a
    // membership that only carries ownership is an object there and an
    // edge here, so the two never agree and saying both is what keeps a
    // reader from thinking one of them wrong
    assert!(said.contains("PartDefinition"), "{said}");
    assert!(said.contains("total elements"), "{said}");
    assert!(said.contains("object(s)"), "{said}");
}

/// The counts a program reads, under the same key names every other
/// command writes.
#[test]
fn it_says_the_same_to_a_program() {
    let out = in_turn(|| {
        let json = whole_document();
        sysml(&["--format", "json", "import", json.to_str().unwrap()])
    });
    assert!(out.status.success());
    let said: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(said["command"], "import");
    assert_eq!(said["ok"], true);
    assert!(said["elements"].as_u64().unwrap() > 2);
    // every element of the library plus the model's own hangs off one
    // root namespace
    assert!(said["roots"].as_u64().unwrap() >= 1);
    assert!(said["objects"].as_u64().unwrap() > said["elements"].as_u64().unwrap());
    assert!(said["counts"]["PartDefinition"].as_u64().unwrap() >= 2);
}

/// A model drawn without ever having been text here.
///
/// This is the whole point of reading a document back: a diagram asks for
/// elements and the relationships between them, which is exactly what the
/// interchange carries, so a model fetched from a server can be looked at
/// without anyone writing it out as notation first.
#[test]
fn what_it_read_can_be_drawn() {
    let svg = dir().join("drawn.svg");
    let out = in_turn(|| {
        let json = whole_document();
        sysml(&[
            "import",
            json.to_str().unwrap(),
            "--diagram",
            svg.to_str().unwrap(),
        ])
    });
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let drawing = std::fs::read_to_string(&svg).unwrap();
    assert!(drawing.starts_with("<svg xmlns="));
    assert!(
        drawing.contains("Engine"),
        "the model is not in the drawing"
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("box(es)"));
}

/// A skin reaches the drawing, and a skin that will not read stops it
/// before anything is written.
#[test]
fn it_is_painted_the_way_the_rest_of_the_tool_paints() {
    let svg = dir().join("painted.svg");
    let (ok, refused) = in_turn(|| {
        let json = whole_document();
        let painted = |skin: &str| {
            sysml(&[
                "import",
                json.to_str().unwrap(),
                "--diagram",
                svg.to_str().unwrap(),
                "--skin",
                skin,
            ])
        };
        (painted("contrast"), painted("chartreuse"))
    });
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("chartreuse"));
}

/// A document that is part of a model rather than one whole.
///
/// Every definition implicitly specializes something in the library, so
/// `export` without `--include-library` writes a reference to an element
/// the document does not carry -- which is also the shape a model server
/// returns for one page of elements. The refusal says which element and
/// what writes a document that stands on its own, because an unexplained
/// UUID is the one thing a reader here cannot act on.
#[test]
fn a_document_that_points_outside_itself_is_refused_in_words() {
    let source = written("partial.sysml", "package P {\n  part def A;\n}\n");
    let json = dir().join("partial.json");
    assert!(sysml(&[
        "export",
        source.to_str().unwrap(),
        "-o",
        json.to_str().unwrap()
    ])
    .status
    .success());

    let out = sysml(&["import", json.to_str().unwrap()]);
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("does not carry it"), "{said}");
    assert!(said.contains("--include-library"), "{said}");

    let json_out = sysml(&["--format", "json", "import", json.to_str().unwrap()]);
    let said: serde_json::Value = serde_json::from_slice(&json_out.stdout).unwrap();
    assert_eq!(said["ok"], false);
    assert!(said["unreadable"][0]["error"]
        .as_str()
        .unwrap()
        .contains("--include-library"));
}

/// The two ways a file fails before any of it is a model: it is not
/// there, and it is there and is not JSON.
#[test]
fn what_is_not_a_document_is_said_to_be_one_rather_than_panicked_over() {
    let missing = dir().join("nothing-here.json");
    let _ = std::fs::remove_file(&missing);
    let out = sysml(&["import", missing.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(!String::from_utf8_lossy(&out.stderr).is_empty());

    let garbage = written("not.json", "part def A;\n");
    let out = sysml(&["import", garbage.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not JSON"));

    // JSON, and not a document: the standard writes an array of elements
    let wrong = written("object.json", "{\"@type\": \"Package\"}\n");
    let out = sysml(&["import", wrong.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("array"));
}

/// A document holding no definition draws nothing, and says so rather
/// than writing an empty picture.
#[test]
fn a_document_with_nothing_in_it_to_draw_is_refused() {
    let empty = written("empty.json", "[]\n");
    let svg = dir().join("empty.svg");
    let out = sysml(&[
        "import",
        empty.to_str().unwrap(),
        "--diagram",
        svg.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("nothing to draw"));

    // and counting it is not an error: a document may legitimately be empty
    let counted = sysml(&["import", empty.to_str().unwrap()]);
    assert!(counted.status.success());
    assert!(String::from_utf8_lossy(&counted.stdout).contains("total elements"));
}

/// `--skin` is about the drawing, so asking for one without asking for a
/// drawing is a mistake the argument parser catches rather than a setting
/// that quietly does nothing.
#[test]
fn a_skin_without_a_drawing_is_refused_by_the_parser() {
    let out = sysml(&[
        "import",
        Path::new("x.json").to_str().unwrap(),
        "--skin",
        "mono",
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--diagram"));
}

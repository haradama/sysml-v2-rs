//! Every example of the notation is real SysML.
//!
//! The point of `notation` is that an agent asking how a construct is
//! written gets something it can copy rather than something plausible.
//! An example that no longer parses -- or that resolves to nothing, or
//! that the specification's own constraints reject -- is worse than no
//! example at all: it is a wrong answer given confidently, to a caller
//! that asked precisely because it did not know.
//!
//! So each is put through the same `check` an agent would put its own
//! model through.

use sysml_cli::notation::{notation, NOTATION};
use sysml_semantics::Workspace;

/// The standard library, loaded and resolved once.
///
/// Each example is checked against a copy of this rather than against a
/// library of its own: cloning a resolved workspace costs a tenth of
/// what loading one does, and there are twenty examples.
fn library() -> Workspace {
    let mut ws = Workspace::new();
    for (name, text) in sysml_stdlib::FILES {
        ws.add_file(*name, text);
    }
    ws.resolve_all();
    ws
}

/// The example, checked the way the tool checks a model: it parses,
/// every name in it resolves against the standard library, and the
/// constraints the specification states hold of it.
fn checked(base: &Workspace, sysml: &str) -> String {
    let mut ws = base.clone();
    let file = ws.add_file("example.sysml", sysml);
    ws.resolve_files(&[file]);

    let mut wrong = String::new();
    let found = ws.diagnose(&[file]);
    for it in &found.found.syntax {
        wrong += &format!("\n  does not parse: {}", it.what);
    }
    for it in &found.found.names {
        wrong += &format!("\n  resolves to nothing: `{}`", it.what);
    }
    for it in &found.rules.violations {
        wrong += &format!(
            "\n  the specification rejects it: {} of `{}`",
            it.rule,
            ws.qualified_name_of(it.element)
        );
    }
    if !wrong.is_empty() && found.rules.violations.is_empty() && !found.asked {
        wrong += "\n  (the constraints were not put to it)";
    }
    wrong
}

#[test]
fn every_example_is_checked() {
    let base = library();
    let mut wrong = String::new();
    for it in NOTATION {
        let said = checked(&base, it.sysml);
        if !said.is_empty() {
            wrong += &format!("\n=== `{}`{said}\n{}", it.of, it.sysml);
        }
    }
    assert!(wrong.is_empty(), "{wrong}");
}

/// And the constraints really were put to them, so that a clean answer
/// above is a clean bill rather than a question nobody asked.
#[test]
fn the_constraints_were_put_to_them() {
    let it = &NOTATION[0];
    let mut ws = library();
    let file = ws.add_file("example.sysml", it.sysml);
    ws.resolve_files(&[file]);
    let found = ws.diagnose(&[file]);
    assert!(found.asked, "`{}` was not asked the constraints", it.of);
    assert!(!found.rules.held.is_empty());
}

/// What one example points at is an example there is, or the pointer is
/// a dead end for whoever follows it.
#[test]
fn what_they_point_at_is_there() {
    for it in NOTATION {
        for &also in it.see_also {
            assert!(
                notation(also).is_some(),
                "`{}` points at `{also}`, which nothing answers for",
                it.of
            );
        }
        assert!(
            !it.see_also.contains(&it.of),
            "`{}` points at itself",
            it.of
        );
    }
}

/// Each is asked for by one name, and asking is not case-sensitive --
/// a caller writing `Requirement` means the same thing.
#[test]
fn each_is_named_once_and_found_either_way() {
    let mut seen: Vec<&str> = NOTATION.iter().map(|it| it.of).collect();
    let given = seen.len();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), given, "two of them answer to one name");

    assert!(notation("requirement").is_some());
    assert!(notation("REQUIREMENT").is_some());
    assert!(notation("no such construct").is_none());
}

/// Every one of them says when to reach for it, since that is the
/// question an agent transcribing a specification actually has: not
/// "what is the grammar of a requirement" but "which of these is the
/// sentence in front of me".
#[test]
fn every_one_says_when_it_is_wanted() {
    for it in NOTATION {
        assert!(it.when.len() > 40, "`{}` says too little", it.of);
        assert!(!it.sysml.is_empty(), "`{}` shows nothing", it.of);
    }
}

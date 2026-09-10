//! Finding a definition by what it says rather than by what it is called.
//!
//! Somebody transcribing a specification has the words the specification
//! used. The library has its own words, and the two meet only in the
//! documentation: a sentence about "the resistance a fluid offers to
//! flow" is asking for `DynamicViscosityValue`, and shares not one word
//! with it.

use sysml_semantics::Workspace;

fn resolved(text: &str) -> Workspace {
    let mut ws = Workspace::default();
    ws.add_file("m.sysml", text);
    ws.resolve_all();
    ws
}

const LIBRARY: &str = "package Q {\n\
                       \tattribute def ViscosityValue {\n\
                       \t\tdoc /* the resistance a fluid offers to flow */\n\
                       \t}\n\
                       \tattribute def PressureValue {\n\
                       \t\tdoc /* force per unit area, as a fluid on the wall of its vessel */\n\
                       \t}\n\
                       \tattribute def Undocumented;\n\
                       }\n";

fn names(ws: &Workspace, query: &str, limit: usize) -> Vec<String> {
    ws.search_documentation(query, limit)
        .into_iter()
        .map(|id| ws.qualified_name_of(id))
        .collect()
}

/// The whole query as a phrase is what it is looking for first.
#[test]
fn a_phrase_out_of_the_documentation_finds_what_said_it() {
    let ws = resolved(LIBRARY);
    assert_eq!(names(&ws, "offers to flow", 20), ["Q::ViscosityValue"]);
}

/// And where nothing says the whole of it, the words of it found apart
/// still say which one is meant -- after any that said it whole.
#[test]
fn the_words_apart_come_after_the_phrase_whole() {
    let ws = resolved(LIBRARY);
    assert_eq!(
        names(&ws, "fluid resistance", 20),
        ["Q::ViscosityValue"],
        "one says both words; the other says only `fluid`"
    );

    // `PressureValue` says `fluid` and `wall`, in that order and apart
    assert_eq!(names(&ws, "wall fluid", 20), ["Q::PressureValue"]);
}

/// Asking is not case-sensitive, since a specification's prose is
/// written in sentences and the library's is not.
#[test]
fn asking_is_not_case_sensitive() {
    let ws = resolved(LIBRARY);
    assert_eq!(names(&ws, "Force Per Unit Area", 20), ["Q::PressureValue"]);
}

/// The shorter documentation comes first: a paragraph largely about what
/// was asked for, before one that mentions it in passing.
#[test]
fn the_documentation_that_is_mostly_about_it_comes_first() {
    let ws = resolved(
        "package Q {\n\
         \tattribute def Mass { doc /* how heavy a thing is */ }\n\
         \tattribute def Weight {\n\
         \t\tdoc /* the force gravity puts on a thing, which depends on how heavy it is and on where it is */\n\
         \t}\n\
         }\n",
    );
    assert_eq!(names(&ws, "how heavy", 20), ["Q::Mass", "Q::Weight"]);
}

/// The limit is a limit, and it is applied after the ordering rather
/// than before it.
#[test]
fn no_more_than_was_asked_for() {
    let ws = resolved(
        "package Q {\n\
         \tattribute def Mass { doc /* how heavy a thing is */ }\n\
         \tattribute def Weight { doc /* also about how heavy a thing is */ }\n\
         }\n",
    );
    assert_eq!(names(&ws, "how heavy", 1), ["Q::Mass"]);
}

/// An empty query finds nothing rather than everything: every documented
/// element in the library is not an answer to a question nobody asked.
#[test]
fn nothing_asked_is_nothing_answered() {
    let ws = resolved(LIBRARY);
    assert!(names(&ws, "   ", 20).is_empty());
}

/// What says nothing about itself is not offered, and neither is what
/// says something no one asked about.
#[test]
fn what_does_not_say_it_is_not_offered() {
    let ws = resolved(LIBRARY);
    assert!(names(&ws, "undocumented", 20).is_empty(), "by its name");
    assert!(names(&ws, "torque", 20).is_empty());
}

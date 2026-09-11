//! A name that answered to nothing answered against the files there were
//! then.
//!
//! Every cache in a workspace is a memory of what the model said, and a
//! file added afterwards can make any of them wrong. The ones that
//! remember a *failure* are the dangerous half: a supertype list that came
//! out short is recomputed the moment anything asks again, but a name
//! remembered as resolving to nothing stays that way unless something
//! forgets it -- which is what `Workspace::forget_failures` is for, and
//! why `add_file` calls it.

use sysml_semantics::Workspace;

/// The one that got away: `has_standard_library` asks whether
/// `Base::Anything` is there, and the answer was remembered from the root
/// namespace.
///
/// A tool that asks before loading the library -- which is what a tool
/// with a copy built in does -- went on being told there was none after
/// loading one. Every name resolved, so nothing looked wrong; what was
/// missing is that the specification's constraints are only put to a model
/// that has the library, so they were silently never put.
#[test]
fn a_library_that_arrives_after_the_question_still_answers_it() {
    let mut ws = Workspace::new();
    ws.add_file("model.sysml", "package P { part def Car; }\n");

    assert!(
        !ws.has_standard_library(),
        "nothing has declared `Base::Anything` yet"
    );

    // the smallest thing that makes the answer yes
    ws.add_file(
        "Base.kerml",
        "standard library package Base {\n\tabstract classifier Anything;\n}\n",
    );

    assert!(
        ws.has_standard_library(),
        "the file that declares it has arrived"
    );
}

/// The same shape without the library in it: any name looked up from the
/// root and not found is looked for again once a file arrives.
#[test]
fn a_name_asked_for_from_the_root_is_asked_again_when_a_file_arrives() {
    let mut ws = Workspace::new();
    ws.add_file("one.sysml", "package P { part def Car; }\n");
    assert_eq!(ws.named_globally("Q::Wheel"), None);

    ws.add_file("two.sysml", "package Q { part def Wheel; }\n");
    assert!(
        ws.named_globally("Q::Wheel").is_some(),
        "`Q` is declared now"
    );
}

/// And a name that was found stays found: forgetting the failures must
/// not throw away the answers, or every lookup pays for every file.
#[test]
fn what_was_found_is_not_forgotten_with_it() {
    let mut ws = Workspace::new();
    ws.add_file("one.sysml", "package P { part def Car; }\n");
    let car = ws.named_globally("P::Car").expect("declared here");

    ws.add_file("two.sysml", "package Q { part def Wheel; }\n");
    assert_eq!(ws.named_globally("P::Car"), Some(car));
}

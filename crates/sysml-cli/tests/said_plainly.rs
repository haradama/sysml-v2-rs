//! What a finding says, to a person and to a program.
//!
//! Both faces of the same thing: the text quotes the line and underlines
//! what it means, and the JSON hands over the span it drew from, so a
//! program need not lex the line again to find out what the tool was
//! pointing at.

use std::path::PathBuf;
use std::process::{Command, Output};

use sysml_cli::report::{draw, Said, Severity};

const MODEL: &str = "package Car {\n\
                     \tpart def Wheel;\n\
                     \tpart def Vehicle {\n\
                     \t\tpart w : Wheeel;\n\
                     \t}\n\
                     }\n";

fn said(span: std::ops::Range<usize>) -> Said<'static> {
    Said {
        severity: Severity::Error,
        title: "`Wheeel` resolves to nothing".into(),
        id: None,
        path: "model/car.sysml",
        text: MODEL,
        span,
        label: None,
        helps: Vec::new(),
    }
}

#[test]
fn it_quotes_the_line_and_underlines_what_it_means() {
    let at = MODEL.find("Wheeel").unwrap();
    let mut about = said(at..at + "Wheeel".len());
    about.helps = vec!["did you mean `Car::Wheel`?".into()];
    let drawn = draw(&about, false);
    // the two tabs are drawn as the terminal draws them, and the caret
    // is placed under what it means rather than by byte offset
    assert!(drawn.contains("model/car.sysml:4:12"), "{drawn}");
    assert!(drawn.contains("part w : Wheeel;"), "{drawn}");
    assert!(drawn.contains("^^^^^^"), "{drawn}");
    assert!(drawn.contains("did you mean `Car::Wheel`?"), "{drawn}");
}

/// A constraint's own name goes where rustc puts an error code, and a
/// warning says warning: a root package that shadows a library one is
/// not a model that fails to work.
#[test]
fn what_it_is_called_and_how_loudly_it_is_said() {
    let at = MODEL.find("Wheel").unwrap();
    let mut about = said(at..at + 5);
    about.id = Some("validateControlNodeIsComposite");
    about.label = Some("`Car::Wheel`".into());
    let drawn = draw(&about, false);
    assert!(
        drawn.starts_with("error[validateControlNodeIsComposite]"),
        "{drawn}"
    );
    assert!(drawn.contains("`Car::Wheel`"), "{drawn}");

    about.severity = Severity::Warning;
    about.id = None;
    assert!(draw(&about, false).starts_with("warning:"), "not a failure");
}

/// A span that is nowhere is still drawn: the parser complaining that
/// the file stopped points past the end of it, and a finding that cannot
/// be drawn is a finding the reader never sees.
#[test]
fn a_span_off_the_end_is_drawn_at_the_end() {
    let drawn = draw(&said(MODEL.len()..MODEL.len() + 40), false);
    assert!(drawn.contains('^'), "{drawn}");

    // and one that starts inside a character is drawn from the start of
    // it rather than panicking
    let text = "part def 'あいう';\n";
    let inside = Said {
        text,
        span: 11..12,
        ..said(0..1)
    };
    assert!(draw(&inside, false).contains('^'), "a caret inside `あ`");
}

/// Colour is escape codes or it is not, and which of the two is asked
/// for rather than guessed at here.
#[test]
fn colour_is_what_was_asked_for() {
    let at = MODEL.find("Wheeel").unwrap();
    let about = said(at..at + 6);
    assert!(draw(&about, true).contains('\u{1b}'), "coloured");
    assert!(!draw(&about, false).contains('\u{1b}'), "plain");
}

fn sysml(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sysml"))
        .args(args)
        .output()
        .unwrap()
}

fn written(name: &str, text: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("sysml-cli-said-plainly");
    std::fs::create_dir_all(&dir).unwrap();
    let at = dir.join(name);
    std::fs::write(&at, text).unwrap();
    at
}

/// The JSON says where a finding ends as well as where it starts.
///
/// A program given a point has to lex the line again to know what the
/// tool meant, and gets it wrong wherever its idea of a name differs
/// from this one's; given the span it underlines exactly what was
/// underlined in the terminal.
#[test]
fn a_program_is_given_the_span_and_not_only_the_point() {
    let model = written(
        "spans.sysml",
        "package Q { part def Wheel; }\n\
         package P { part def Car { part w : Wheel; } }\n",
    );
    let out = sysml(&[
        "--no-library",
        "--format",
        "json",
        "check",
        model.to_str().unwrap(),
    ]);
    let said: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let found = &said["unresolved"][0];
    assert_eq!(found["name"], "Wheel", "{said}");
    assert_eq!(found["line"], 2, "{said}");
    assert_eq!(found["endLine"], 2, "{said}");
    assert_eq!(
        found["endOffset"].as_u64().unwrap() - found["offset"].as_u64().unwrap(),
        "Wheel".len() as u64,
        "{said}"
    );
    assert_eq!(
        found["endColumn"].as_u64().unwrap() - found["column"].as_u64().unwrap(),
        "Wheel".len() as u64,
        "{said}"
    );
}

/// And it says what the name might have meant, which the command line
/// has known since the MCP server was taught to answer it and has been
/// keeping to itself.
#[test]
fn a_program_is_told_what_the_name_might_have_meant() {
    let declared = written(
        "declared.sysml",
        "package Q { part def Wheel; }\n\
         package P { part def Car { part w : Wheel; } }\n",
    );
    let out = sysml(&[
        "--no-library",
        "--format",
        "json",
        "check",
        declared.to_str().unwrap(),
    ]);
    let said: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(said["unresolved"][0]["declaredAs"][0], "Q::Wheel", "{said}");

    let misspelt = written(
        "misspelt.sysml",
        "package P { part def Wheel; part def Car { part w : Wheeel; } }\n",
    );
    let out = sysml(&[
        "--no-library",
        "--format",
        "json",
        "check",
        misspelt.to_str().unwrap(),
    ]);
    let said: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(said["unresolved"][0]["didYouMean"][0], "P::Wheel", "{said}");
    assert!(said["unresolved"][0]["declaredAs"].is_null(), "{said}");
}

/// A name that is declared somewhere is answered with the import it
/// wants and nothing else: the names merely near it make that answer
/// worse, not better.
#[test]
fn a_person_is_told_the_one_mistake_that_applies() {
    let model = written(
        "one.sysml",
        "package Q { part def Wheel; }\n\
         package P { part def Car { part w : Wheel; } }\n",
    );
    let out = sysml(&["--no-library", "check", model.to_str().unwrap()]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("it is declared as `Q::Wheel`"), "{stderr}");
    assert!(!stderr.contains("did you mean"), "{stderr}");
}

/// A name nothing declares is answered with the names near it, said as
/// a sentence -- and stopped before the reader stops reading, since a
/// list of everything vaguely like it is a question rather than an
/// answer.
#[test]
fn a_person_is_told_what_the_name_was_nearly() {
    let model = written(
        "near.sysml",
        "package P {\n\
         \tpart def Wheel;\n\
         \tpart def Wheels;\n\
         \tpart def Wheelz;\n\
         \tpart def Wheela;\n\
         \tpart def Car { part w : Wheeel; }\n\
         }\n",
    );
    let out = sysml(&["--no-library", "check", model.to_str().unwrap()]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // nearest first -- `Wheel` is one letter out and the rest are two,
    // which leaves them in the order the model declares them -- said as
    // a sentence, and the rest counted rather than listed
    assert!(
        stderr.contains("did you mean `P::Wheel`, `P::Wheels`, `P::Wheelz` or 1 other(s)?"),
        "{stderr}"
    );
}

/// Colour is asked for on the command line, and a terminal is what
/// decides it when nobody asks. A test is not a terminal, so `auto`
/// here is plain and the other two are what they say.
#[test]
fn the_command_line_says_whether_to_colour() {
    let model = written("coloured.sysml", "package P { part w : Wheel; }\n");
    let escapes = |args: &[&str]| {
        let out = sysml(args);
        String::from_utf8_lossy(&out.stderr).contains('\u{1b}')
    };
    let path = model.to_str().unwrap();
    assert!(escapes(&[
        "--no-library",
        "--color",
        "always",
        "check",
        path
    ]));
    assert!(!escapes(&[
        "--no-library",
        "--color",
        "never",
        "check",
        path
    ]));
    assert!(!escapes(&["--no-library", "check", path]), "not a terminal");
}

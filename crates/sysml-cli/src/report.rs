//! What a finding looks like to a person.
//!
//! Every finding goes out twice: as JSON for a program, placed by offset
//! and span, and as text for whoever is reading the terminal. The text
//! used to be one line --
//!
//! ```text
//! model/car.sysml:4:20: unresolved `Wheel`
//! ```
//!
//! -- which says where to look but not what is there, so the reader
//! opens the file to find out, and what the tool already knew about the
//! name went unsaid. Quoting the line, underlining the span and putting
//! what is known underneath is a solved problem: `annotate-snippets` is
//! rustc's own renderer, taken out of it and published. It counts a
//! column the way a terminal draws one, which is the part that is easy
//! to get wrong -- a caret placed by byte offset lands half a line away
//! from what it means on any file with a wide character in it, and this
//! toolchain's models carry documentation in whatever language they were
//! written in.

use std::ops::Range;

use annotate_snippets::{AnnotationKind, Group, Level, Renderer, Snippet};

/// Whether what was found stops the model working, or only wants
/// looking at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// The model is wrong.
    Error,
    /// The model works and something about it is worth knowing.
    Warning,
}

/// One finding, ready to be drawn.
pub struct Said<'a> {
    /// How loudly to say it.
    pub severity: Severity,
    /// The sentence at the top.
    pub title: String,
    /// What rustc puts an error code in. Ours is the constraint's own
    /// name -- what the specification calls it, and so what somebody
    /// looking it up will search for.
    pub id: Option<&'a str>,
    /// The file it is in, as the reader named it.
    pub path: &'a str,
    /// The whole file. Only the annotated lines are drawn.
    pub text: &'a str,
    /// The bytes it is about.
    pub span: Range<usize>,
    /// What to write under the caret.
    pub label: Option<String>,
    /// What is known about it, a line each, under the snippet.
    pub helps: Vec<String>,
}

/// Draw it, in colour or without.
///
/// A span is made to fit the text rather than trusted. A parser
/// complaining that the file stopped points past the end of it, and one
/// clipped by somebody's arithmetic can land inside a character; a
/// renderer handed either panics, so a model that merely ends badly
/// would take the report with it. It is clamped to the text, widened out
/// to whole characters, and widened again to one character where it is
/// empty, since nothing is drawn under a caret that is nowhere.
pub fn draw(said: &Said<'_>, colour: bool) -> String {
    let start = floor(said.text, said.span.start.min(said.text.len()));
    let end = ceiling(said.text, said.span.end.clamp(start, said.text.len()));
    let end = match end == start {
        true => ceiling(said.text, (start + 1).min(said.text.len())),
        false => end,
    };

    let level = match said.severity {
        Severity::Error => Level::ERROR,
        Severity::Warning => Level::WARNING,
    };
    let mut annotation = AnnotationKind::Primary.span(start..end);
    if let Some(label) = &said.label {
        annotation = annotation.label(label.as_str());
    }
    let mut title = level.primary_title(said.title.as_str());
    if let Some(id) = said.id {
        title = title.id(id);
    }
    let mut groups = vec![title.element(
        Snippet::source(said.text)
            .path(said.path)
            .annotation(annotation),
    )];
    for help in &said.helps {
        groups.push(Group::with_title(
            Level::HELP.secondary_title(help.as_str()),
        ));
    }

    let renderer = match colour {
        true => Renderer::styled(),
        false => Renderer::plain(),
    };
    renderer.render(&groups).to_string()
}

/// The character boundary at or before `offset`, and the one at or
/// after it.
///
/// Offsets come from the lexer and land on boundaries, but one that has
/// been clipped -- to the end of a file, or by a caller's arithmetic --
/// need not, and slicing a string anywhere else is a panic. A span
/// widened to the character it started inside is what the reader wanted
/// underlined anyway.
fn floor(text: &str, offset: usize) -> usize {
    let mut at = offset.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn ceiling(text: &str, offset: usize) -> usize {
    let mut at = offset.min(text.len());
    while at < text.len() && !text.is_char_boundary(at) {
        at += 1;
    }
    at
}

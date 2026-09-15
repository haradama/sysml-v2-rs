//! Lexer, parser and lossless syntax tree for the SysML v2 / KerML textual
//! notation.
//!
//! The design follows rust-analyzer: a [`logos`]-based lexer feeds a
//! hand-written recursive-descent parser that builds a lossless
//! [`rowan`]-based concrete syntax tree (CST). Typed AST views over the CST
//! live in [`ast`]. Parsing never fails — bad input yields a tree that still
//! reproduces the source text exactly, plus [`Diagnostic`]s.
//!
//! ```
//! let parse = sysml_syntax::parse("part def Vehicle { attribute mass : Real; }");
//! assert!(parse.ok());
//!
//! use sysml_syntax::ast::{self, AstNode};
//! let file = ast::SourceFile::cast(parse.syntax()).unwrap();
//! let def = file.members().find_map(|m| match m {
//!     ast::Member::Definition(d) => Some(d),
//!     _ => None,
//! }).unwrap();
//! assert_eq!(def.name().unwrap().text(), "Vehicle");
//! ```

pub mod ast;
pub mod fmt;
mod kind;
mod lexer;
mod parser;

pub use ast::is_name_chain;
pub use kind::{
    Reserved, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken, SysMLLanguage, KEYWORDS,
};
pub use lexer::{lex, lex_dialect, Token};
pub use parser::{parse, parse_dialect, Parse};
pub use rowan::{TextRange, TextSize, TokenAtOffset};

/// The two textual notations sharing this syntax tree. Keywords of one
/// dialect are ordinary identifiers in the other (`frame` is a name in
/// KerML; `step` is a name in SysML).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Dialect {
    /// `.sysml` files
    #[default]
    SysML,
    /// `.kerml` files
    KerML,
}

impl Dialect {
    /// Pick a dialect from a file extension (defaults to SysML).
    pub fn from_extension(ext: &str) -> Dialect {
        if ext.eq_ignore_ascii_case("kerml") {
            Dialect::KerML
        } else {
            Dialect::SysML
        }
    }

    /// Pick a dialect from a file name or path, by its extension.
    ///
    /// The one place the rule lives: the formatter, the workspace and the
    /// corpus tests each used to spell it, and not all of them ignored
    /// the case of the extension.
    pub fn from_path(path: impl AsRef<std::path::Path>) -> Dialect {
        path.as_ref()
            .extension()
            .and_then(|ext| ext.to_str())
            .map_or(Dialect::SysML, Dialect::from_extension)
    }

    /// Whether this dialect reads that kind as a keyword.
    ///
    /// The two disagree: `frame` is a name in KerML and a keyword in
    /// SysML, `step` the other way round.
    pub fn is_keyword(self, kind: SyntaxKind) -> bool {
        match self {
            Dialect::SysML => kind.is_sysml_keyword(),
            Dialect::KerML => kind.is_kerml_keyword(),
        }
    }
}

/// The name a name token denotes.
///
/// A basic name is itself. An unrestricted name is what stands between
/// its quotes with the escapes inside resolved, so `'it\'s'` is the
/// four characters `it's` and not six -- KerML says the escape sequences
/// "shall be replaced" by what they stand for. The syntax tree, the
/// model builder and the resolver each kept a copy of the quote
/// stripping that left the backslash in, and the three agreed with each
/// other only because none of them resolved anything.
pub fn unquote(text: &str) -> String {
    unquote_with(text, '\'')
}

/// What a comment says, with the margin it is drawn down taken off.
///
/// A body written across lines carries whatever runs down its left --
/// a column of `*`, or the indentation of whatever it sits inside --
/// and neither is part of what it says. Reading it this way is what
/// lets the formatter bring a comment's interior under the column its
/// `/*` ends up in without changing the model: the same paragraph one
/// level further in is the same paragraph.
///
/// Takes the token's own text, delimiters and all.
pub fn comment_text(raw: &str) -> String {
    // each end on its own: a file being typed into holds a comment that
    // has been opened and not yet closed, and what it says so far is
    // still what it says
    let text = raw.strip_prefix("/*").unwrap_or(raw);
    let text = text.strip_suffix("*/").unwrap_or(text);
    let mut lines: Vec<&str> = text.lines().collect();
    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    // What runs down the left of the lines under the first one is what
    // decides the margin. The first line sits after `/*` and shares
    // neither, so it is taken as it comes.
    let under: Vec<&str> = lines
        .iter()
        .skip(1)
        .copied()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let starred = !under.is_empty() && under.iter().all(|line| line.trim_start().starts_with('*'));
    let margin = if starred {
        String::new()
    } else {
        shared_indent(&under)
    };
    lines
        .iter()
        .map(|line| {
            if starred {
                let rest = line.trim_start();
                match rest.strip_prefix('*') {
                    Some(said) => said.strip_prefix(' ').unwrap_or(said).to_string(),
                    None => rest.to_string(),
                }
            } else {
                line.strip_prefix(&margin)
                    .unwrap_or_else(|| line.trim_start())
                    .to_string()
            }
        })
        .collect::<Vec<String>>()
        .join("\n")
        .trim()
        .to_string()
}

/// The whitespace every one of `lines` begins with.
pub(crate) fn shared_indent(lines: &[&str]) -> String {
    let indent = |line: &str| line.len() - line.trim_start().len();
    let shortest = lines.iter().map(|line| indent(line)).min().unwrap_or(0);
    lines
        .first()
        .map(|line| line[..shortest].to_string())
        .unwrap_or_default()
}

/// The text a string literal denotes.
///
/// The same rule as [`unquote`], with double quotes: `"say \"hi\""` is
/// the eight characters `say "hi"`. Stripping the quotes by trimming
/// them off eats the escaped one at the end besides.
pub fn unquote_string(text: &str) -> String {
    unquote_with(text, '"')
}

fn unquote_with(text: &str, quote: char) -> String {
    let Some(inner) = text
        .strip_prefix(quote)
        .and_then(|inner| inner.strip_suffix(quote))
    else {
        return text.to_string();
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            // `\'`, `\"`, `\\`: the character itself
            Some(other) => out.push(other),
            // the lexer never ends a name on a lone backslash; a caller
            // handing over arbitrary text gets it back as written
            None => out.push('\\'),
        }
    }
    out
}

/// A parse or lex error with its byte range in the source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Where in the source text it is about.
    pub range: TextRange,
    /// What is wrong, said in one sentence.
    pub message: String,
}

impl Diagnostic {
    pub(crate) fn new(range: &std::ops::Range<usize>, message: impl Into<String>) -> Self {
        Diagnostic {
            range: TextRange::new(
                TextSize::from(range.start as u32),
                TextSize::from(range.end as u32),
            ),
            message: message.into(),
        }
    }
}

/// Where a byte offset falls, counting lines and columns from one and
/// measuring the column in bytes.
///
/// The command line prints this and the MCP server puts it in its
/// answers; they had a copy each, identical but for whether they counted
/// from zero. A language server needs a different measure -- the
/// protocol counts a column in UTF-16 code units -- and keeps its own.
///
/// The counting is over bytes rather than characters because an offset
/// handed in from elsewhere -- a diagnostic that has been clipped, a
/// column a user typed -- need not land on a character boundary, and
/// slicing the text at one that does not is a panic.
pub fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let prefix = &text.as_bytes()[..offset.min(text.len())];
    let line = prefix.iter().filter(|&&byte| byte == b'\n').count() + 1;
    let column = prefix
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(prefix.len(), |at| prefix.len() - at - 1)
        + 1;
    (line, column)
}

#[cfg(test)]
mod line_col_tests {
    #[test]
    fn a_byte_offset_is_a_line_and_a_column_counting_from_one() {
        let text = "ab\ncd\n";
        assert_eq!(super::line_col(text, 0), (1, 1));
        assert_eq!(super::line_col(text, 2), (1, 3));
        assert_eq!(super::line_col(text, 3), (2, 1));
        // a column is bytes, so a wide character is more than one
        assert_eq!(super::line_col("あb", 3), (1, 4));
        // past the end is the end
        assert_eq!(super::line_col(text, 99), (3, 1));
    }

    #[test]
    fn an_offset_inside_a_character_still_answers() {
        assert_eq!(super::line_col("あい", 1), (1, 2));
        assert_eq!(super::line_col("a\nあ", 3), (2, 2));
    }
}

#[cfg(test)]
mod comment_tests {
    use super::comment_text;

    #[test]
    fn a_margin_is_not_what_the_comment_says() {
        // the same paragraph, drawn at two indentations
        let close = "/*\n * first\n * second\n */";
        let far = "/*\n         * first\n         * second\n         */";
        assert_eq!(comment_text(close), "first\nsecond");
        assert_eq!(comment_text(far), comment_text(close));
    }

    #[test]
    fn a_body_with_no_stars_keeps_the_shape_under_its_own_indentation() {
        assert_eq!(
            comment_text("/*\n    first\n      indented\n    last\n*/"),
            "first\n  indented\nlast"
        );
    }

    #[test]
    fn what_follows_the_opening_delimiter_carries_no_margin() {
        // `first` sits after `/*` and shares nothing with the lines
        // under it, whether those are starred or merely indented
        assert_eq!(comment_text("/* first\n * second */"), "first\nsecond");
        assert_eq!(comment_text("/* first\n    second */"), "first\nsecond");
    }

    #[test]
    fn one_line_and_nothing_at_all() {
        assert_eq!(comment_text("/* said */"), "said");
        assert_eq!(comment_text("/**/"), "");
        assert_eq!(comment_text("/*\n\n*/"), "");
        // an unterminated body is what a half-written file holds
        assert_eq!(comment_text("/* open"), "open");
    }

    #[test]
    fn a_blank_line_inside_is_a_paragraph_break() {
        assert_eq!(
            comment_text("/*\n * first\n *\n * second\n */"),
            "first\n\nsecond"
        );
    }
}

#[cfg(test)]
mod name_tests {
    use super::{unquote, Dialect};

    #[test]
    fn an_unrestricted_name_is_what_its_quotes_hold_with_escapes_resolved() {
        assert_eq!(unquote("plain"), "plain");
        assert_eq!(unquote("'two words'"), "two words");
        assert_eq!(unquote("'it\\'s'"), "it's");
        assert_eq!(unquote("'a\\\\b'"), "a\\b");
        assert_eq!(unquote("'tab\\there'"), "tab\there");
        assert_eq!(unquote("'\\b\\f\\n\\r'"), "\u{8}\u{c}\n\r");
        // an escape the language does not define stands for its character
        assert_eq!(unquote("'\\q'"), "q");
        // text that is not a name token comes back as it was
        assert_eq!(unquote("'unterminated"), "'unterminated");
        assert_eq!(unquote("'ends in a backslash\\'"), "ends in a backslash\\");
    }

    #[test]
    fn a_dialect_is_read_off_the_extension_whatever_its_case() {
        assert_eq!(Dialect::from_path("Model.kerml"), Dialect::KerML);
        assert_eq!(Dialect::from_path("Model.KERML"), Dialect::KerML);
        assert_eq!(Dialect::from_path("dir.kerml/Model.sysml"), Dialect::SysML);
        assert_eq!(Dialect::from_path("noext"), Dialect::SysML);
        assert_eq!(
            Dialect::from_path(std::path::Path::new("/tmp/x.kerml")),
            Dialect::KerML
        );
    }
}

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

pub use kind::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken, SysMLLanguage};
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

    pub fn is_keyword(self, kind: SyntaxKind) -> bool {
        match self {
            Dialect::SysML => kind.is_sysml_keyword(),
            Dialect::KerML => kind.is_kerml_keyword(),
        }
    }
}

/// A parse or lex error with its byte range in the source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub range: TextRange,
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

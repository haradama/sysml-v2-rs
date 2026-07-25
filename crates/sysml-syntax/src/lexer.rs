//! Lexer for the KerML / SysML v2 textual notation, built on `logos`.
//!
//! Notes (`//`, `//* ... */`) are trivia; `/* ... */` is *not* — it is the
//! body of a `doc` or `comment` element and participates in the grammar.

use logos::Logos;

use crate::{Diagnostic, SyntaxKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Note {
    Line,
    Block { terminated: bool },
}

fn lex_note(lex: &mut logos::Lexer<'_, Tok>) -> Note {
    let rem = lex.remainder();
    if let Some(rest) = rem.strip_prefix('*') {
        // `//* ... */`
        match rest.find("*/") {
            Some(i) => {
                lex.bump(1 + i + 2);
                Note::Block { terminated: true }
            }
            None => {
                lex.bump(rem.len());
                Note::Block { terminated: false }
            }
        }
    } else {
        let i = rem.find('\n').unwrap_or(rem.len());
        lex.bump(i);
        Note::Line
    }
}

fn lex_comment_body(lex: &mut logos::Lexer<'_, Tok>) -> bool {
    let rem = lex.remainder();
    match rem.find("*/") {
        Some(i) => {
            lex.bump(i + 2);
            true
        }
        None => {
            lex.bump(rem.len());
            false
        }
    }
}

#[derive(Logos, Clone, Copy, Debug, PartialEq)]
pub(crate) enum Tok {
    #[regex(r"[ \t\r\n]+")]
    Whitespace,
    #[token("//", lex_note)]
    Note(Note),
    #[token("/*", lex_comment_body)]
    CommentBody(bool),

    #[regex(r"[A-Za-z_][A-Za-z0-9_]*")]
    Ident,
    #[regex(r"'([^'\\\n]|\\.)*'")]
    UnrestrictedName,
    #[regex(r#""([^"\\\n]|\\.)*""#)]
    Str,
    #[regex(
        r"([0-9]+((\.[0-9]+([eE][+-]?[0-9]+)?)|([eE][+-]?[0-9]+)))|(\.[0-9]+([eE][+-]?[0-9]+)?)"
    )]
    Real,
    #[regex(r"[0-9]+")]
    Decimal,

    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token(";")]
    Semicolon,
    #[token(",")]
    Comma,
    #[token(".")]
    Dot,
    #[token("..")]
    DotDot,
    #[token(".?")]
    DotQuestion,
    #[token(":")]
    Colon,
    #[token("::")]
    ColonColon,
    #[token(":>")]
    ColonGt,
    #[token(":>>")]
    ColonGtGt,
    #[token("::>")]
    ColonColonGt,
    #[token(":=")]
    ColonEq,
    #[token("=")]
    Eq,
    #[token("==")]
    EqEq,
    #[token("===")]
    EqEqEq,
    #[token("!=")]
    NotEq,
    #[token("!==")]
    NotEqEq,
    #[token("=>")]
    FatArrow,
    #[token("->")]
    Arrow,
    #[token("*")]
    Star,
    #[token("**")]
    StarStar,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,
    #[token("^")]
    Caret,
    #[token("<")]
    Lt,
    #[token("<=")]
    LtEq,
    #[token(">")]
    Gt,
    #[token(">=")]
    GtEq,
    #[token("&")]
    Amp,
    #[token("|")]
    Pipe,
    #[token("~")]
    Tilde,
    #[token("?")]
    Question,
    #[token("??")]
    QuestionQuestion,
    #[token("@")]
    At,
    #[token("@@")]
    AtAt,
    #[token("#")]
    Hash,
    #[token("$")]
    Dollar,
}

/// A lexed token: kind plus byte range into the source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: SyntaxKind,
    pub range: std::ops::Range<usize>,
}

/// Tokenize `text` as SysML (see [`lex_dialect`]).
pub fn lex(text: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    lex_dialect(text, crate::Dialect::SysML)
}

/// Tokenize `text`, returning every token (including trivia) plus lexing
/// diagnostics. The concatenation of all token texts equals the input.
/// Identifiers are classified as keywords per `dialect`.
pub fn lex_dialect(text: &str, dialect: crate::Dialect) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut tokens = Vec::new();
    let mut diagnostics = Vec::new();

    for (result, range) in Tok::lexer(text).spanned() {
        let kind = match result {
            Ok(Tok::Whitespace) => SyntaxKind::WHITESPACE,
            Ok(Tok::Note(Note::Line)) => SyntaxKind::LINE_NOTE,
            Ok(Tok::Note(Note::Block { terminated })) => {
                if !terminated {
                    diagnostics.push(Diagnostic::new(&range, "unterminated note"));
                }
                SyntaxKind::BLOCK_NOTE
            }
            Ok(Tok::CommentBody(terminated)) => {
                if !terminated {
                    diagnostics.push(Diagnostic::new(&range, "unterminated comment body"));
                }
                SyntaxKind::COMMENT_BODY
            }
            Ok(Tok::Ident) => SyntaxKind::from_keyword(&text[range.clone()])
                .filter(|kw| dialect.is_keyword(*kw))
                .unwrap_or(SyntaxKind::IDENT),
            Ok(Tok::UnrestrictedName) => SyntaxKind::UNRESTRICTED_NAME,
            Ok(Tok::Str) => SyntaxKind::STRING,
            Ok(Tok::Real) => SyntaxKind::REAL,
            Ok(Tok::Decimal) => SyntaxKind::DECIMAL,
            Ok(Tok::LBrace) => SyntaxKind::L_BRACE,
            Ok(Tok::RBrace) => SyntaxKind::R_BRACE,
            Ok(Tok::LParen) => SyntaxKind::L_PAREN,
            Ok(Tok::RParen) => SyntaxKind::R_PAREN,
            Ok(Tok::LBracket) => SyntaxKind::L_BRACKET,
            Ok(Tok::RBracket) => SyntaxKind::R_BRACKET,
            Ok(Tok::Semicolon) => SyntaxKind::SEMICOLON,
            Ok(Tok::Comma) => SyntaxKind::COMMA,
            Ok(Tok::Dot) => SyntaxKind::DOT,
            Ok(Tok::DotDot) => SyntaxKind::DOT_DOT,
            Ok(Tok::DotQuestion) => SyntaxKind::DOT_QUESTION,
            Ok(Tok::Colon) => SyntaxKind::COLON,
            Ok(Tok::ColonColon) => SyntaxKind::COLON_COLON,
            Ok(Tok::ColonGt) => SyntaxKind::COLON_GT,
            Ok(Tok::ColonGtGt) => SyntaxKind::COLON_GT_GT,
            Ok(Tok::ColonColonGt) => SyntaxKind::COLON_COLON_GT,
            Ok(Tok::ColonEq) => SyntaxKind::COLON_EQ,
            Ok(Tok::Eq) => SyntaxKind::EQ,
            Ok(Tok::EqEq) => SyntaxKind::EQ_EQ,
            Ok(Tok::EqEqEq) => SyntaxKind::EQ_EQ_EQ,
            Ok(Tok::NotEq) => SyntaxKind::NOT_EQ,
            Ok(Tok::NotEqEq) => SyntaxKind::NOT_EQ_EQ,
            Ok(Tok::FatArrow) => SyntaxKind::FAT_ARROW,
            Ok(Tok::Arrow) => SyntaxKind::ARROW,
            Ok(Tok::Star) => SyntaxKind::STAR,
            Ok(Tok::StarStar) => SyntaxKind::STAR_STAR,
            Ok(Tok::Plus) => SyntaxKind::PLUS,
            Ok(Tok::Minus) => SyntaxKind::MINUS,
            Ok(Tok::Slash) => SyntaxKind::SLASH,
            Ok(Tok::Percent) => SyntaxKind::PERCENT,
            Ok(Tok::Caret) => SyntaxKind::CARET,
            Ok(Tok::Lt) => SyntaxKind::LT,
            Ok(Tok::LtEq) => SyntaxKind::LT_EQ,
            Ok(Tok::Gt) => SyntaxKind::GT,
            Ok(Tok::GtEq) => SyntaxKind::GT_EQ,
            Ok(Tok::Amp) => SyntaxKind::AMP,
            Ok(Tok::Pipe) => SyntaxKind::PIPE,
            Ok(Tok::Tilde) => SyntaxKind::TILDE,
            Ok(Tok::Question) => SyntaxKind::QUESTION,
            Ok(Tok::QuestionQuestion) => SyntaxKind::QUESTION_QUESTION,
            Ok(Tok::At) => SyntaxKind::AT,
            Ok(Tok::AtAt) => SyntaxKind::AT_AT,
            Ok(Tok::Hash) => SyntaxKind::HASH,
            Ok(Tok::Dollar) => SyntaxKind::DOLLAR,
            Err(()) => {
                diagnostics.push(Diagnostic::new(
                    &range,
                    format!("unexpected character `{}`", &text[range.clone()]),
                ));
                SyntaxKind::ERROR_TOKEN
            }
        };
        tokens.push(Token { kind, range });
    }

    (tokens, diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SyntaxKind::*;

    fn kinds(text: &str) -> Vec<SyntaxKind> {
        lex(text).0.into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn punctuation_longest_match() {
        assert_eq!(
            kinds("::A:>B:>>C:D"),
            vec![
                COLON_COLON,
                IDENT,
                COLON_GT,
                IDENT,
                COLON_GT_GT,
                IDENT,
                COLON,
                IDENT
            ]
        );
        assert_eq!(kinds("0..2"), vec![DECIMAL, DOT_DOT, DECIMAL]);
        assert_eq!(kinds("1.5"), vec![REAL]);
        assert_eq!(kinds("::**"), vec![COLON_COLON, STAR_STAR]);
    }

    #[test]
    fn notes_and_comment_bodies() {
        assert_eq!(kinds("// line note\nx"), vec![LINE_NOTE, WHITESPACE, IDENT]);
        assert_eq!(kinds("//* block\nnote */x"), vec![BLOCK_NOTE, IDENT]);
        assert_eq!(kinds("/* comment body */"), vec![COMMENT_BODY]);
    }

    #[test]
    fn keywords_and_names() {
        assert_eq!(
            kinds("part def Vehicle"),
            vec![PART_KW, WHITESPACE, DEF_KW, WHITESPACE, IDENT]
        );
        assert_eq!(kinds("'unrestricted name'"), vec![UNRESTRICTED_NAME]);
    }

    #[test]
    fn rare_operators_lex() {
        assert_eq!(kinds("=> @@"), vec![FAT_ARROW, WHITESPACE, AT_AT]);
    }

    #[test]
    fn unterminated_tokens_are_reported() {
        let (tokens, diags) = lex("//* never closed");
        assert_eq!(tokens[0].kind, BLOCK_NOTE);
        assert_eq!(diags.len(), 1);
        let (tokens, diags) = lex("/* never closed  ");
        assert_eq!(tokens[0].kind, COMMENT_BODY);
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn unexpected_characters_are_error_tokens() {
        let (tokens, diags) = lex("part ` def");
        assert!(tokens.iter().any(|t| t.kind == ERROR_TOKEN));
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn lossless() {
        let text = "part def Vehicle { attribute mass : Real = 1200.0; } // ok\n";
        let (tokens, diags) = lex(text);
        assert!(diags.is_empty());
        let rebuilt: String = tokens.iter().map(|t| &text[t.range.clone()]).collect();
        assert_eq!(rebuilt, text);
    }
}

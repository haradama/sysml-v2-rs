//! Canonical formatter for the KerML / SysML v2 textual notation.
//!
//! Token-stream based, driven by the lossless CST: indentation follows brace
//! depth, one member per line, single blank lines are preserved, comments
//! keep their own-line/trailing position, and spacing is decided from token
//! kinds plus the parent node (so `a < b` gets spaces while `<shortName>`
//! does not). Comment and note interiors are emitted verbatim.
//!
//! Guarantees (regression-tested against the whole official corpus):
//! formatting never changes the non-trivia token stream (reparse
//! equivalence) and is idempotent.

use crate::{parse_dialect, Dialect, SyntaxKind, SyntaxKind::*, SyntaxToken};

const INDENT: &str = "    ";

/// Format a whole source file.
pub fn format(text: &str, dialect: Dialect) -> String {
    let parse = parse_dialect(text, dialect);
    let all_tokens: Vec<SyntaxToken> = parse
        .syntax()
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .collect();
    Formatter {
        out: String::new(),
        depth: 0,
        line_empty: true,
        pending_newlines: 0,
        prev: None,
    }
    .run(&all_tokens)
}

/// Format using the dialect implied by a file name.
pub fn format_file(name: &str, text: &str) -> String {
    let dialect = if name.ends_with(".kerml") {
        Dialect::KerML
    } else {
        Dialect::SysML
    };
    format(text, dialect)
}

struct Formatter {
    out: String,
    depth: usize,
    /// nothing emitted on the current line yet
    line_empty: bool,
    /// newlines to emit before the next token
    pending_newlines: usize,
    prev: Option<SyntaxToken>,
}

impl Formatter {
    fn run(mut self, all_tokens: &[SyntaxToken]) -> String {
        let visible: Vec<&SyntaxToken> = all_tokens
            .iter()
            .filter(|t| !t.kind().is_trivia() || is_note(t.kind()))
            .collect();

        let mut skip_next = false;
        for (index, token) in visible.iter().enumerate() {
            if skip_next {
                skip_next = false;
                continue;
            }
            let kind = token.kind();
            let gap = self.original_gap_lines(token);
            let next_kind = visible
                .iter()
                .skip(index + 1)
                .find(|t| !is_note(t.kind()))
                .map(|t| t.kind());

            if is_note(kind) {
                if gap == 0 && !self.line_empty && self.prev.is_some() {
                    // trailing note stays on its line: `part def A; // note`
                    self.out.push(' ');
                    self.out.push_str(token.text());
                } else {
                    self.break_line(gap);
                    self.out.push_str(token.text());
                }
                self.line_empty = false;
                self.pending_newlines = self.pending_newlines.max(1);
                self.prev = Some((*token).clone());
                continue;
            }

            if kind == R_BRACE {
                self.depth = self.depth.saturating_sub(1);
                if !self.line_empty {
                    self.pending_newlines = self.pending_newlines.max(1);
                }
            }
            if self.pending_newlines > 0 {
                self.break_line(gap);
            } else if !self.line_empty {
                let prev = self.prev.clone().expect("non-empty line has a token");
                if space_between(&prev, token) {
                    self.out.push(' ');
                }
            }
            self.out.push_str(token.text());
            self.line_empty = false;

            match kind {
                L_BRACE => {
                    // keep `{}` on one line by emitting the pair atomically
                    if visible.get(index + 1).map(|t| t.kind()) == Some(R_BRACE) {
                        self.out.push('}');
                        self.pending_newlines = 1;
                        self.prev = Some(visible[index + 1].clone());
                        skip_next = true;
                        continue;
                    }
                    self.depth += 1;
                    self.pending_newlines = 1;
                }
                R_BRACE => {
                    if next_kind != Some(SEMICOLON) {
                        self.pending_newlines = 1;
                    }
                }
                SEMICOLON | COMMENT_BODY => self.pending_newlines = 1,
                _ => {}
            }
            self.prev = Some((*token).clone());
        }

        while self.out.ends_with(['\n', ' ', '\t']) {
            self.out.pop();
        }
        if !self.out.is_empty() {
            self.out.push('\n');
        }
        self.out
    }

    /// Emit the pending newline(s) — preserving at most one original blank
    /// line — and the indentation for the new line.
    fn break_line(&mut self, original_gap: usize) {
        if self.prev.is_none() {
            self.pending_newlines = 0;
            return; // start of file
        }
        let newlines = if original_gap >= 2 {
            2
        } else {
            self.pending_newlines.max(1)
        };
        for _ in 0..newlines {
            self.out.push('\n');
        }
        for _ in 0..self.depth {
            self.out.push_str(INDENT);
        }
        self.pending_newlines = 0;
        self.line_empty = true;
    }

    /// Newlines in the original text between the previous visible token and
    /// `next` (whitespace trivia only).
    fn original_gap_lines(&self, next: &SyntaxToken) -> usize {
        let Some(prev) = &self.prev else { return 0 };
        let mut lines = 0;
        let mut cursor = prev.next_token();
        while let Some(t) = cursor {
            if t.text_range() == next.text_range() {
                break;
            }
            if t.kind() == WHITESPACE {
                lines += t.text().matches('\n').count();
            }
            cursor = t.next_token();
        }
        lines
    }
}

fn is_note(kind: SyntaxKind) -> bool {
    matches!(kind, LINE_NOTE | BLOCK_NOTE)
}

/// Should a single space separate `prev` and `next`?
fn space_between(prev: &SyntaxToken, next: &SyntaxToken) -> bool {
    let p = prev.kind();
    let n = next.kind();

    // never before closers / separators
    if matches!(n, SEMICOLON | COMMA | R_PAREN | R_BRACKET) {
        return false;
    }
    // never after openers and prefix markers
    if matches!(p, L_PAREN | L_BRACKET | HASH | AT | AT_AT | TILDE | DOLLAR) {
        return false;
    }
    // tight path/scope operators
    if matches!(p, COLON_COLON | DOT | DOT_QUESTION | DOT_DOT)
        || matches!(n, COLON_COLON | DOT | DOT_QUESTION | DOT_DOT)
    {
        return false;
    }
    // `<shortName>` is tight inside the brackets; `a < b` is not
    if (p == LT && in_short_name(prev)) || (n == GT && in_short_name(next)) {
        return false;
    }
    // unary sign binds to its operand
    if matches!(p, MINUS | PLUS)
        && prev
            .parent()
            .is_some_and(|parent| parent.kind() == UNARY_EXPR)
    {
        return false;
    }
    // calls and multiplicities attach to names: `f(x)`, `Wheel[4]`
    if n == L_PAREN {
        return !matches!(p, IDENT | UNRESTRICTED_NAME | R_PAREN | R_BRACKET);
    }
    if n == L_BRACKET {
        return !matches!(p, IDENT | UNRESTRICTED_NAME | R_PAREN);
    }
    true
}

fn in_short_name(token: &SyntaxToken) -> bool {
    matches!(token.kind(), LT | GT) && token.parent().is_some_and(|p| p.kind() == SHORT_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(text: &str) -> String {
        format(text, Dialect::SysML)
    }

    #[test]
    fn normalizes_spacing_and_indentation() {
        let input =
            "package   P{part def Vehicle{attribute mass:Real=1200.0;part wheels:Wheel[4];}}";
        let expected = "package P {\n    part def Vehicle {\n        attribute mass : Real = 1200.0;\n        part wheels : Wheel[4];\n    }\n}\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn preserves_single_blank_lines() {
        let input = "package P {\n    part def A;\n\n\n\n    part def B;\n}\n";
        let expected = "package P {\n    part def A;\n\n    part def B;\n}\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn keeps_trailing_and_own_line_notes() {
        let input =
            "package P {\n    part def A; // trailing\n    // own line\n    part def B;\n}\n";
        let expected =
            "package P {\n    part def A; // trailing\n    // own line\n    part def B;\n}\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn short_names_and_operators() {
        let input = "attribute def <kg> Kilogram;\nattribute ok = a<b and x>=y;";
        let expected = "attribute def <kg> Kilogram;\nattribute ok = a < b and x >= y;\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn empty_bodies_stay_on_one_line() {
        assert_eq!(fmt("part def A {}"), "part def A {}\n");
        assert_eq!(fmt("part def A {  }"), "part def A {}\n");
    }

    #[test]
    fn docs_end_their_line() {
        let input = "package P { doc /* about P */ part def A; }";
        let expected = "package P {\n    doc /* about P */\n    part def A;\n}\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn kerml_dialect_empty_input_and_unterminated_comments() {
        assert_eq!(
            crate::fmt::format_file("m.kerml", "classifier   A;"),
            "classifier A;\n"
        );
        assert_eq!(fmt(""), "");
        // an unterminated comment body ends with whitespace that must be
        // trimmed from the output
        let out = fmt("doc /* open  ");
        assert!(out.ends_with("open\n"), "{out:?}");
    }

    #[test]
    fn idempotent() {
        let input = "package P { /* doc-ish */ part x : X = f(1, 2) + -3; }";
        let once = fmt(input);
        assert_eq!(fmt(&once), once);
    }

    #[test]
    fn reparse_equivalence() {
        let input = "package P{part def V{attribute m:Real;}}";
        let formatted = fmt(input);
        let a = crate::parse(input);
        let b = crate::parse(&formatted);
        assert!(b.ok());
        let toks = |p: &crate::Parse| {
            p.syntax()
                .descendants_with_tokens()
                .filter_map(|e| e.into_token())
                .filter(|t| !t.kind().is_trivia())
                .map(|t| (t.kind(), t.text().to_string()))
                .collect::<Vec<_>>()
        };
        assert_eq!(toks(&a), toks(&b));
    }
}

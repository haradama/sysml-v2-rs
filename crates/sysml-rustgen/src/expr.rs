//! SysML expression text as a Rust expression, for the simple subset.
//!
//! What translates: literals, references to what the caller can name
//! (parameters, fields), feature chains (`line.amount`), the arithmetic,
//! comparison and logical operators, parentheses, the conditional
//! `if c ? a else b`, and a call of a calculation the generator wrote a
//! function for. Numeric literals keep their written form, so a
//! model mixing `2` into Real arithmetic surfaces as a Rust type error
//! rather than a silent coercion. Anything beyond the subset -- `**`, a
//! quoted name, a named argument, a call of something abstract or of
//! nothing at all, an unresolvable reference -- makes
//! the whole expression untranslatable, and the caller says so instead
//! of approximating.

/// A translated expression and what the caller may want to know of it.
pub(crate) struct Translated {
    pub rust: String,
    /// The top of the expression yields a boolean.
    pub boolean: bool,
    /// The leading name of every reference chain, in order of first use.
    pub references: Vec<String>,
}

/// `text` as a Rust expression, or `None` where any part of it falls
/// outside the simple subset. `resolve` says how Rust spells the leading
/// name of a reference chain (`self.price`, a parameter).
pub(crate) fn translate(
    text: &str,
    resolve: &dyn Fn(&str) -> Option<String>,
    resolve_call: &dyn Fn(&str) -> Option<String>,
) -> Option<Translated> {
    let tokens = lex(text)?;
    let mut parser = Parser {
        tokens: &tokens,
        at: 0,
        references: Vec::new(),
        resolve,
        resolve_call,
    };
    let top = parser.expression()?;
    (parser.at == tokens.len()).then_some(Translated {
        rust: top.rust,
        boolean: top.boolean,
        references: parser.references,
    })
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Num(String),
    Str(String),
    Name(String),
    True,
    False,
    Not,
    And,
    Or,
    Xor,
    Implies,
    If,
    Else,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Lt,
    Le,
    Gt,
    Ge,
    EqEq,
    Ne,
    LParen,
    RParen,
    Comma,
    Dot,
    Question,
}

fn lex(text: &str) -> Option<Vec<Token>> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let ch = chars[at];
        if ch.is_whitespace() {
            at += 1;
            continue;
        }
        if ch.is_ascii_digit() {
            let start = at;
            while at < chars.len() && chars[at].is_ascii_digit() {
                at += 1;
            }
            if at + 1 < chars.len() && chars[at] == '.' && chars[at + 1].is_ascii_digit() {
                at += 1;
                while at < chars.len() && chars[at].is_ascii_digit() {
                    at += 1;
                }
            }
            if at < chars.len() && matches!(chars[at], 'e' | 'E') {
                let mut peek = at + 1;
                if peek < chars.len() && matches!(chars[peek], '+' | '-') {
                    peek += 1;
                }
                if peek < chars.len() && chars[peek].is_ascii_digit() {
                    at = peek;
                    while at < chars.len() && chars[at].is_ascii_digit() {
                        at += 1;
                    }
                }
            }
            tokens.push(Token::Num(chars[start..at].iter().collect()));
            continue;
        }
        if ch.is_alphabetic() || ch == '_' {
            let start = at;
            while at < chars.len() && (chars[at].is_alphanumeric() || chars[at] == '_') {
                at += 1;
            }
            let word: String = chars[start..at].iter().collect();
            tokens.push(match word.as_str() {
                "true" => Token::True,
                "false" => Token::False,
                "not" => Token::Not,
                "and" => Token::And,
                "or" => Token::Or,
                "xor" => Token::Xor,
                "implies" => Token::Implies,
                "if" => Token::If,
                "else" => Token::Else,
                _ => Token::Name(word),
            });
            continue;
        }
        if ch == '"' {
            let start = at;
            at += 1;
            while at < chars.len() && chars[at] != '"' {
                if chars[at] == '\\' {
                    at += 1;
                }
                at += 1;
            }
            if at >= chars.len() {
                return None;
            }
            at += 1;
            tokens.push(Token::Str(chars[start..at].iter().collect()));
            continue;
        }
        let two = (at + 1 < chars.len()).then(|| chars[at + 1]);
        let (token, width) = match (ch, two) {
            ('<', Some('=')) => (Token::Le, 2),
            ('>', Some('=')) => (Token::Ge, 2),
            ('=', Some('=')) => (Token::EqEq, 2),
            ('!', Some('=')) => (Token::Ne, 2),
            ('<', _) => (Token::Lt, 1),
            ('>', _) => (Token::Gt, 1),
            ('+', _) => (Token::Plus, 1),
            ('-', _) => (Token::Minus, 1),
            ('*', _) => (Token::Star, 1),
            ('/', _) => (Token::Slash, 1),
            ('%', _) => (Token::Percent, 1),
            ('(', _) => (Token::LParen, 1),
            (')', _) => (Token::RParen, 1),
            (',', _) => (Token::Comma, 1),
            ('.', _) => (Token::Dot, 1),
            ('?', _) => (Token::Question, 1),
            ('&', _) => (Token::And, 1),
            ('|', _) => (Token::Or, 1),
            _ => return None,
        };
        tokens.push(token);
        at += width;
    }
    Some(tokens)
}

/// One built subexpression: whether it needs parentheses when embedded,
/// and whether it yields a boolean.
struct Node {
    rust: String,
    atomic: bool,
    boolean: bool,
}

/// Embedded as an operand: composites keep their own parentheses.
fn wrap(node: &Node) -> String {
    if node.atomic {
        node.rust.clone()
    } else {
        format!("({})", node.rust)
    }
}

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
    references: Vec<String>,
    resolve: &'a dyn Fn(&str) -> Option<String>,
    resolve_call: &'a dyn Fn(&str) -> Option<String>,
}

impl Parser<'_> {
    fn eat(&mut self, token: &Token) -> bool {
        if self.tokens.get(self.at) == Some(token) {
            self.at += 1;
            return true;
        }
        false
    }

    /// Precedence low to high, after KerML: implies, or, xor, and,
    /// equality, relational, additive, multiplicative, unary, primary.
    fn expression(&mut self) -> Option<Node> {
        let lhs = self.or_expression()?;
        if !self.eat(&Token::Implies) {
            return Some(lhs);
        }
        // right-associative, spelled out: `a implies b` is `!a || b`
        let rhs = self.expression()?;
        Some(Node {
            rust: format!("!{} || {}", wrap(&lhs), wrap(&rhs)),
            atomic: false,
            boolean: true,
        })
    }

    fn or_expression(&mut self) -> Option<Node> {
        self.binary(&[(Token::Or, " || ")], Self::xor_expression, true)
    }

    fn xor_expression(&mut self) -> Option<Node> {
        self.binary(&[(Token::Xor, " ^ ")], Self::and_expression, true)
    }

    fn and_expression(&mut self) -> Option<Node> {
        self.binary(&[(Token::And, " && ")], Self::equality, true)
    }

    fn equality(&mut self) -> Option<Node> {
        self.binary(
            &[(Token::EqEq, " == "), (Token::Ne, " != ")],
            Self::relational,
            true,
        )
    }

    fn relational(&mut self) -> Option<Node> {
        self.binary(
            &[
                (Token::Le, " <= "),
                (Token::Ge, " >= "),
                (Token::Lt, " < "),
                (Token::Gt, " > "),
            ],
            Self::additive,
            true,
        )
    }

    fn additive(&mut self) -> Option<Node> {
        self.binary(
            &[(Token::Plus, " + "), (Token::Minus, " - ")],
            Self::multiplicative,
            false,
        )
    }

    fn multiplicative(&mut self) -> Option<Node> {
        self.binary(
            &[
                (Token::Star, " * "),
                (Token::Slash, " / "),
                (Token::Percent, " % "),
            ],
            Self::unary,
            false,
        )
    }

    fn binary(
        &mut self,
        operators: &[(Token, &str)],
        next: fn(&mut Self) -> Option<Node>,
        boolean: bool,
    ) -> Option<Node> {
        let mut lhs = next(self)?;
        'joining: loop {
            for (token, spelled) in operators {
                if self.eat(token) {
                    let rhs = next(self)?;
                    lhs = Node {
                        rust: format!("{}{spelled}{}", wrap(&lhs), wrap(&rhs)),
                        atomic: false,
                        boolean,
                    };
                    continue 'joining;
                }
            }
            return Some(lhs);
        }
    }

    fn unary(&mut self) -> Option<Node> {
        if self.eat(&Token::Minus) {
            let operand = self.unary()?;
            return Some(Node {
                rust: format!("-{}", wrap(&operand)),
                atomic: false,
                boolean: false,
            });
        }
        if self.eat(&Token::Not) {
            let operand = self.unary()?;
            return Some(Node {
                rust: format!("!{}", wrap(&operand)),
                atomic: false,
                boolean: true,
            });
        }
        self.primary()
    }

    fn primary(&mut self) -> Option<Node> {
        let token = self.tokens.get(self.at)?.clone();
        self.at += 1;
        match token {
            Token::Num(text) => Some(Node {
                rust: text,
                atomic: true,
                boolean: false,
            }),
            Token::Str(text) => Some(Node {
                rust: format!("{text}.to_string()"),
                atomic: true,
                boolean: false,
            }),
            Token::True => Some(Node {
                rust: "true".to_string(),
                atomic: true,
                boolean: true,
            }),
            Token::False => Some(Node {
                rust: "false".to_string(),
                atomic: true,
                boolean: true,
            }),
            // a call, where the name is one the caller can spell as a
            // function -- an abstract definition became a trait, and a
            // trait method is not callable out of nowhere
            Token::Name(leading) if self.tokens.get(self.at) == Some(&Token::LParen) => {
                let callee = (self.resolve_call)(&leading)?;
                self.at += 1;
                let mut arguments = Vec::new();
                if !self.eat(&Token::RParen) {
                    loop {
                        arguments.push(self.expression()?.rust);
                        if self.eat(&Token::RParen) {
                            break;
                        }
                        if !self.eat(&Token::Comma) {
                            return None;
                        }
                    }
                }
                Some(Node {
                    rust: format!("{callee}({})", arguments.join(", ")),
                    atomic: true,
                    boolean: false,
                })
            }
            Token::Name(leading) => {
                let mut rust = (self.resolve)(&leading)?;
                if !self.references.contains(&leading) {
                    self.references.push(leading);
                }
                while self.eat(&Token::Dot) {
                    let Some(Token::Name(segment)) = self.tokens.get(self.at).cloned() else {
                        return None;
                    };
                    self.at += 1;
                    rust = format!("{rust}.{}", super::ident(&segment));
                }
                Some(Node {
                    rust,
                    atomic: true,
                    boolean: false,
                })
            }
            Token::LParen => {
                let inner = self.expression()?;
                self.eat(&Token::RParen).then_some(Node {
                    rust: format!("({})", inner.rust),
                    atomic: true,
                    boolean: inner.boolean,
                })
            }
            // `if c ? a else b` -- braces keep it one Rust expression
            Token::If => {
                let condition = self.expression()?;
                if !self.eat(&Token::Question) {
                    return None;
                }
                let then = self.expression()?;
                if !self.eat(&Token::Else) {
                    return None;
                }
                let otherwise = self.expression()?;
                Some(Node {
                    rust: format!(
                        "if {} {{ {} }} else {{ {} }}",
                        condition.rust, then.rust, otherwise.rust
                    ),
                    atomic: false,
                    boolean: then.boolean,
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::translate;

    fn plain(text: &str) -> Option<String> {
        translate(text, &|name| Some(name.to_string()), &|name| {
            Some(name.to_lowercase())
        })
        .map(|t| t.rust)
    }

    #[test]
    fn arithmetic_keeps_kerml_precedence() {
        assert_eq!(plain("mass * speed").as_deref(), Some("mass * speed"));
        assert_eq!(
            plain("-x + (a + b) * 2.0e3").as_deref(),
            Some("(-x) + ((a + b) * 2.0e3)")
        );
        assert_eq!(plain("a / b % c - d").as_deref(), Some("((a / b) % c) - d"));
        assert_eq!(plain("- -a").as_deref(), Some("-(-a)"));
        assert_eq!(plain("2.5E-2 + 1e3").as_deref(), Some("2.5E-2 + 1e3"));
        // an exponent letter with no digits ends the number before it
        assert!(plain("2e").is_none());
    }

    #[test]
    fn logic_spells_rust_operators() {
        assert_eq!(
            plain("not a and b or c xor d").as_deref(),
            Some("((!a) && b) || (c ^ d)")
        );
        assert_eq!(plain("a & b | c").as_deref(), Some("(a && b) || c"));
        assert_eq!(plain("a implies b").as_deref(), Some("!a || b"));
        assert_eq!(
            plain("a implies b implies c").as_deref(),
            Some("!a || (!b || c)")
        );
        let compared = translate("a <= b != c >= d", &|n| Some(n.to_string()), &|_| None).unwrap();
        assert_eq!(compared.rust, "(a <= b) != (c >= d)");
        assert!(compared.boolean);
        assert_eq!(plain("x < y").as_deref(), Some("x < y"));
        assert_eq!(plain("x > y == true").as_deref(), Some("(x > y) == true"));
    }

    #[test]
    fn primaries_and_chains() {
        let translated = translate(
            "line.amount > 0 or empty",
            &|n| (n != "missing").then(|| n.to_string()),
            &|_| None,
        )
        .unwrap();
        assert_eq!(translated.rust, "(line.amount > 0) || empty");
        assert_eq!(translated.references, ["line", "empty"]);
        assert!(plain("a.r#in").is_none());
        assert_eq!(
            plain("self_.type").as_deref(),
            Some("self_.r#type"),
            "reserved chain segments become raw identifiers"
        );
        assert_eq!(
            plain("\"a\\\"b\" != name").as_deref(),
            Some("\"a\\\"b\".to_string() != name")
        );
        assert_eq!(plain("false or true").as_deref(), Some("false || true"));
        assert_eq!(
            plain("if a < b ? a else b").as_deref(),
            Some("if a < b { a } else { b }")
        );
        assert_eq!(
            plain("(if a < b ? a else b) + 1").as_deref(),
            Some("(if a < b { a } else { b }) + 1")
        );
    }

    #[test]
    fn what_falls_outside_stays_untranslated() {
        for text in [
            "",
            "a ** b",
            "a ^ b",
            "'quoted name' + 1",
            "a..b",
            "a b",
            "\"unterminated",
            "x + ",
            "(a",
            "a.",
            "a.1",
            "if a ? b",
            "if a ? b else",
            "if a b else c",
            "not",
            "[x]",
            "a := b",
            "a == ==",
            "2 .. 3",
            // a named argument names a parameter, and the caller has no
            // way to know which position that is
            "f(x = 1)",
            "f(1,)",
            "f(1",
        ] {
            assert!(plain(text).is_none(), "expected no translation: {text:?}");
        }
        // an unresolved leading name refuses the whole expression
        assert!(translate(
            "known + missing",
            &|n| (n == "known").then(|| n.to_string()),
            &|_| None
        )
        .is_none());
        // and so does a call of something the caller cannot spell
        assert!(plain("Widen(1, 2) > 0").is_some());
        assert!(translate("Widen(1) > 0", &|n| Some(n.to_string()), &|_| None).is_none());
    }
}

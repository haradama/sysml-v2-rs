//! SysML expression text as a Rust expression, for the simple subset.
//!
//! What translates: literals, references to what the caller can name,
//! feature chains (`line.amount`), the arithmetic, comparison and logical
//! operators, `**` as `powf`, escaped names, parentheses, `if c ? a else
//! b`, and a call of a calculation the generator wrote a function for. A
//! whole number keeps its written form unless the caller says the
//! expression is over reals, so a model mixing types Rust will not mix
//! surfaces as a type error rather than a silent coercion.
//!
//! A quantity written with its unit -- `2500 [g]`, `100 [km/h]` -- is
//! the number of SI coherent units it comes to, where the caller knows
//! what the unit is worth: `2.5`, `27.77777777777778`.
//!
//! Anything beyond the subset -- a call of something abstract or of
//! nothing at all, a call naming only some of its arguments, an
//! unresolvable reference -- makes the whole expression untranslatable,
//! and the caller says so instead of approximating.

use std::collections::HashMap;

/// What a call resolves to: the function Rust spells it as, and the
/// parameters it takes, in order, so that a call written with named
/// arguments can be put back into that order.
pub(crate) struct Callee {
    pub function: String,
    pub parameters: Vec<String>,
}

/// A translated expression and what the caller may want to know of it.
#[derive(Debug)]
pub(crate) struct Translated {
    pub rust: String,
    /// The top of the expression yields a boolean.
    pub boolean: bool,
    /// The leading name of every reference chain, in order of first use.
    pub references: Vec<String>,
}

impl Translated {
    /// Rust the caller worked out for itself, with nothing to say about
    /// it: no references gathered, no claim that it is a boolean.
    pub(crate) fn plain(rust: String) -> Translated {
        Translated {
            rust,
            boolean: false,
            references: Vec::new(),
        }
    }
}

/// How a whole number written in the model is spelled in Rust.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Numbers {
    /// As the model wrote it, so that a model mixing `2` into `Integer`
    /// arithmetic keeps the spelling it chose.
    Written,
    /// With a point on it. SysML has one numeric tower and Rust has
    /// several: `x * 2` against a `Real` means 2.0, and Rust will not
    /// multiply an `f64` by an integer to find that out.
    Reals,
    /// Whole, and within an `i64`: what an `Integer` holds. `1.5` and
    /// `99999999999999999999` are no such number, and written as they
    /// were they did not compile against the field.
    Integers,
}

/// `text` as a Rust expression, or `None` where any part of it falls
/// outside the simple subset. `resolve` says how Rust spells the leading
/// name of a reference chain (`self.price`, a parameter).
pub(crate) fn translate(
    text: &str,
    resolve: &dyn Fn(&str) -> Option<String>,
    resolve_call: &dyn Fn(&str) -> Option<Callee>,
) -> Option<Translated> {
    translate_as(
        text,
        Numbers::Written,
        &HashMap::new(),
        resolve,
        resolve_call,
    )
}

/// [`translate`], saying how whole numbers should be spelled and what
/// each unit written in brackets is worth, keyed by the unit as written
/// with its spaces taken out.
pub(crate) fn translate_as(
    text: &str,
    numbers: Numbers,
    units: &HashMap<String, f64>,
    resolve: &dyn Fn(&str) -> Option<String>,
    resolve_call: &dyn Fn(&str) -> Option<Callee>,
) -> Option<Translated> {
    let tokens = lex(text)?;
    let mut parser = Parser {
        tokens: &tokens,
        at: 0,
        numbers,
        units,
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
    StarStar,
    Slash,
    Percent,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    EqEq,
    Ne,
    LParen,
    RParen,
    Comma,
    Dot,
    Question,
    /// `[km/h]`, as written with its spaces taken out.
    Unit(String),
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
        // an escaped name -- `'provide transportation'` -- is a name
        // like any other; what Rust can spell it as is the caller's
        // business, not the lexer's
        if ch == '\'' {
            let start = at + 1;
            at += 1;
            while at < chars.len() && chars[at] != '\'' {
                at += 1;
            }
            if at >= chars.len() {
                return None;
            }
            tokens.push(Token::Name(chars[start..at].iter().collect()));
            at += 1;
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
        // what is in brackets is a unit, which the caller puts a number
        // to; this keeps the text it will be asked by
        if ch == '[' {
            let start = at + 1;
            while at < chars.len() && chars[at] != ']' {
                at += 1;
            }
            if at >= chars.len() {
                return None;
            }
            let written: String = chars[start..at].iter().collect();
            tokens.push(Token::Unit(crate::quantity::spaceless(&written)));
            at += 1;
            continue;
        }
        let two = (at + 1 < chars.len()).then(|| chars[at + 1]);
        let (token, width) = match (ch, two) {
            ('<', Some('=')) => (Token::Le, 2),
            ('>', Some('=')) => (Token::Ge, 2),
            ('=', Some('=')) => (Token::EqEq, 2),
            ('=', _) => (Token::Eq, 1),
            ('!', Some('=')) => (Token::Ne, 2),
            ('<', _) => (Token::Lt, 1),
            ('>', _) => (Token::Gt, 1),
            ('+', _) => (Token::Plus, 1),
            ('-', _) => (Token::Minus, 1),
            ('*', Some('*')) => (Token::StarStar, 2),
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
    /// A `String`: `"x".to_string() + "y".to_string()` does not compile,
    /// since Rust adds a `&str` to a `String` and not another `String`.
    text: bool,
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
    numbers: Numbers,
    units: &'a HashMap<String, f64>,
    references: Vec<String>,
    resolve: &'a dyn Fn(&str) -> Option<String>,
    resolve_call: &'a dyn Fn(&str) -> Option<Callee>,
}

impl Parser<'_> {
    fn eat(&mut self, token: &Token) -> bool {
        if self.tokens.get(self.at) == Some(token) {
            self.at += 1;
            return true;
        }
        false
    }

    /// The arguments of a call, in the order the callee declares its
    /// parameters. SysML lets them be named -- `Widen(size = 1)` -- and
    /// a name says which parameter it is for, not where it goes, so the
    /// callee's own order is what puts them back in place. Naming one
    /// argument means naming them all, since a call that mixes the two
    /// says nothing about where the unnamed ones belong.
    fn arguments(&mut self, callee: &Callee) -> Option<Vec<String>> {
        let mut positional = Vec::new();
        let mut named: Vec<(String, String)> = Vec::new();
        if !self.eat(&Token::RParen) {
            loop {
                match (self.tokens.get(self.at), self.tokens.get(self.at + 1)) {
                    (Some(Token::Name(parameter)), Some(Token::Eq)) => {
                        let parameter = parameter.clone();
                        self.at += 2;
                        named.push((parameter, self.expression()?.rust));
                    }
                    _ => positional.push(self.expression()?.rust),
                }
                if self.eat(&Token::RParen) {
                    break;
                }
                if !self.eat(&Token::Comma) {
                    return None;
                }
            }
        }
        if named.is_empty() {
            return Some(positional);
        }
        if !positional.is_empty() || named.len() != callee.parameters.len() {
            return None;
        }
        callee
            .parameters
            .iter()
            .map(|parameter| {
                named
                    .iter()
                    .find(|(named, _)| named == parameter)
                    .map(|(_, argument)| argument.clone())
            })
            .collect()
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
            text: false,
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
            Self::power,
            false,
        )
    }

    /// `a ** b`, which Rust spells as a method. Both sides have to be `Real`;
    /// an exponent written as a whole number is spelled as one anyway, since
    /// `x ** 2` means the same real number as `x ** 2.0` and only one of the
    /// two compiles.
    ///
    /// Written `f64::powf(a, b)` rather than `a.powf(b)` because a literal
    /// base has no settled type yet, and Rust will not pick one of its float
    /// types to look a method up on.
    fn power(&mut self) -> Option<Node> {
        let lhs = self.unary()?;
        if !self.eat(&Token::StarStar) {
            return Some(lhs);
        }
        // right-associative, as exponentiation is
        let rhs = self.power()?;
        // a power held by an integer is no `f64`, and `i64::pow` takes a
        // `u32` exponent the model does not promise
        if self.numbers == Numbers::Integers {
            return None;
        }
        let exponent = match rhs.rust.parse::<i64>() {
            Ok(whole) => format!("{whole}.0"),
            Err(_) => rhs.rust,
        };
        Some(Node {
            rust: format!("f64::powf({}, {exponent})", lhs.rust),
            atomic: true,
            boolean: false,
            text: false,
        })
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
                    // `String + &str` is how Rust joins text
                    let borrowed = if lhs.text && rhs.text && spelled.trim() == "+" {
                        "&"
                    } else {
                        ""
                    };
                    lhs = Node {
                        rust: format!("{}{spelled}{borrowed}{}", wrap(&lhs), wrap(&rhs)),
                        atomic: false,
                        boolean,
                        text: lhs.text && rhs.text,
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
                text: false,
            });
        }
        if self.eat(&Token::Not) {
            let operand = self.unary()?;
            return Some(Node {
                rust: format!("!{}", wrap(&operand)),
                atomic: false,
                boolean: true,
                text: false,
            });
        }
        let operand = self.primary()?;
        self.measured(operand)
    }

    /// `operand [unit]` -- the bracket binds tighter than any operator,
    /// as KerML's grammar has it. A number is multiplied out here, so
    /// `2500 [g]` is written `2.5`; anything else is multiplied in the
    /// generated code, and by nothing at all where the unit is one the
    /// SI counts in already.
    fn measured(&mut self, operand: Node) -> Option<Node> {
        let Some(Token::Unit(unit)) = self.tokens.get(self.at) else {
            return Some(operand);
        };
        let factor = *self.units.get(unit)?;
        self.at += 1;
        // by its first character, since `inf` and `NaN` parse as numbers
        // too and are names here
        let number = operand
            .rust
            .starts_with(|ch: char| ch.is_ascii_digit())
            .then(|| operand.rust.parse::<f64>().ok())
            .flatten();
        if let Some(number) = number {
            // to fifteen significant digits, which is all a product of
            // two decimals in a double has: `3 [ft]` is `0.9144` and not
            // the `0.9144000000000001` the multiplication leaves behind
            let product = format!("{:.14e}", number * factor)
                .parse::<f64>()
                .expect("a number Rust wrote reads back");
            return Some(Node {
                rust: crate::generate::real_literal(product),
                atomic: true,
                boolean: false,
                text: false,
            });
        }
        if factor == 1.0 {
            return Some(operand);
        }
        Some(Node {
            rust: format!(
                "{} * {}",
                wrap(&operand),
                crate::generate::real_literal(factor)
            ),
            atomic: false,
            boolean: false,
            text: false,
        })
    }

    fn primary(&mut self) -> Option<Node> {
        let token = self.tokens.get(self.at)?.clone();
        self.at += 1;
        match token {
            Token::Num(text) if self.numbers == Numbers::Integers => Some(Node {
                rust: text.parse::<i64>().ok()?.to_string(),
                atomic: true,
                boolean: false,
                text: false,
            }),
            Token::Num(text) => Some(Node {
                rust: match self.numbers == Numbers::Reals && !text.contains(['.', 'e', 'E']) {
                    true => format!("{text}.0"),
                    false => text,
                },
                atomic: true,
                boolean: false,
                text: false,
            }),
            Token::Str(text) => Some(Node {
                rust: format!("{text}.to_string()"),
                atomic: true,
                boolean: false,
                text: true,
            }),
            Token::True => Some(Node {
                rust: "true".to_string(),
                atomic: true,
                boolean: true,
                text: false,
            }),
            Token::False => Some(Node {
                rust: "false".to_string(),
                atomic: true,
                boolean: true,
                text: false,
            }),
            // a call, where the name is one the caller can spell as a
            // function -- an abstract definition became a trait, and a
            // trait method is not callable out of nowhere
            Token::Name(leading) if self.tokens.get(self.at) == Some(&Token::LParen) => {
                let callee = (self.resolve_call)(&leading)?;
                self.at += 1;
                let arguments = self.arguments(&callee)?;
                Some(Node {
                    rust: format!("{}({})", callee.function, arguments.join(", ")),
                    atomic: true,
                    boolean: false,
                    text: false,
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
                    rust = format!("{rust}.{}", crate::generate::ident(&segment));
                }
                Some(Node {
                    rust,
                    atomic: true,
                    boolean: false,
                    text: false,
                })
            }
            Token::LParen => {
                let inner = self.expression()?;
                self.eat(&Token::RParen).then_some(Node {
                    rust: format!("({})", inner.rust),
                    atomic: true,
                    boolean: inner.boolean,
                    text: false,
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
                    text: false,
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{translate, translate_as, Callee, Numbers};

    /// Every name resolves to itself, and every call to a lowercase
    /// function of two parameters -- enough to exercise the shapes.
    fn plain(text: &str) -> Option<String> {
        translate(text, &|name| Some(name.to_string()), &|name| {
            Some(Callee {
                function: name.to_lowercase(),
                parameters: vec!["a".to_string(), "b".to_string()],
            })
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
            "a ^ b",
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
            // naming only some of the arguments says nothing about where
            // the unnamed ones belong
            "f(a = 1, 2)",
            // and a name that is no parameter of the callee places nothing
            "f(a = 1, x = 2)",
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
        // `**` is a method in Rust, and a whole-number exponent is
        // spelled as the real number it stands for
        assert_eq!(plain("s ** 2").as_deref(), Some("f64::powf(s, 2.0)"));
        assert_eq!(plain("s ** 2.5").as_deref(), Some("f64::powf(s, 2.5)"));
        assert_eq!(
            plain("a ** b ** c").as_deref(),
            Some("f64::powf(a, f64::powf(b, c))")
        );
        // a literal base has no type to look a method up on
        assert_eq!(plain("3.0 ** 2").as_deref(), Some("f64::powf(3.0, 2.0)"));
        // an escaped name is a name
        assert_eq!(plain("'a name' + 1").as_deref(), Some("a name + 1"));
        assert!(plain("'unterminated").is_none());
        // named arguments go back into the order the callee declares
        assert_eq!(plain("F(b = 1, a = 2)").as_deref(), Some("f(2, 1)"));
        assert_eq!(plain("F(1, 2)").as_deref(), Some("f(1, 2)"));
        assert!(translate("Widen(1) > 0", &|n| Some(n.to_string()), &|_| None).is_none());
    }
    /// A unit in brackets is what the caller says it is worth, looked up
    /// by its text with the spaces taken out.
    #[test]
    fn a_quantity_comes_to_its_number_of_si_units() {
        let units: std::collections::HashMap<String, f64> = [
            ("g".to_string(), 1e-3),
            ("km/h".to_string(), 1000.0 / 3600.0),
            ("ft".to_string(), 0.3048),
            ("kg".to_string(), 1.0),
        ]
        .into();
        let measured = |text: &str| {
            translate_as(
                text,
                Numbers::Written,
                &units,
                &|name| Some(name.to_string()),
                &|_| None,
            )
            .map(|t| t.rust)
        };
        // a number is multiplied out, to the digits a double has
        assert_eq!(measured("2500 [g]").as_deref(), Some("2.5"));
        assert_eq!(measured("3 [ft]").as_deref(), Some("0.9144"));
        assert_eq!(measured("2.5E3 [ g ]").as_deref(), Some("2.5"));
        // and the bracket binds tighter than any operator
        assert_eq!(measured("m + 500 [g]").as_deref(), Some("m + 0.5"));
        // anything else is multiplied in the code, or not at all
        assert_eq!(
            measured("s [km / h]").as_deref(),
            Some("s * 0.2777777777777778")
        );
        assert_eq!(measured("(a + b) [g]").as_deref(), Some("(a + b) * 0.001"));
        assert_eq!(measured("m [kg]").as_deref(), Some("m"));
        // `inf` is a name here, whatever `f64::from_str` makes of it
        assert_eq!(measured("inf [g]").as_deref(), Some("inf * 0.001"));
        // a unit the caller has no number for is no translation at all
        assert!(measured("20 ['°C']").is_none());
        assert!(measured("2 [kg").is_none());
        // and with no units given, a bracket is beyond the subset
        assert!(plain("2500 [g]").is_none());
    }
}

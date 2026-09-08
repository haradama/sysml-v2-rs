//! The OCL the specification writes its well-formedness constraints in.
//!
//! [`sysml_model::RULES`] carries those constraints as the metamodel
//! states them, in OCL, verbatim. This parses the subset they use into a
//! tree that [`crate::rules`] evaluates. The subset is what the 180
//! constraints actually contain and no more -- navigation, the
//! collection operations, `let`, `if`, and the logical and comparison
//! operators -- because a parser that accepts what the corpus never
//! writes is a parser nothing checks.

/// One OCL expression.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Expr {
    Null,
    Bool(bool),
    Int(i64),
    /// `'select'`, and the library names `specializesFromLibrary` takes.
    Str(String),
    /// `*`, the unbounded end of a multiplicity. `MultiplicityRange::
    /// valueOf` answers it for an infinite literal and
    /// `hasBounds` compares against it.
    Unlimited,
    /// A bare name: `self`, a `let` or lambda variable, or a property of
    /// the element the rule is about. Which it is, is settled when it is
    /// evaluated and not before.
    Name(String),
    /// `TransitionFeatureKind::guard`.
    Enum(String, String),
    /// `Set{...}`, `Sequence{...}`.
    Collection(Vec<Expr>),
    Not(Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
    /// `if c then a else b endif`.
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `let x : T = value in body` -- the type is written but says
    /// nothing the value does not, so it is not kept.
    Let(String, Box<Expr>, Box<Expr>),
    /// `a.b`, where `b` names no operation.
    Nav(Box<Expr>, String),
    /// `a.f(x)`, `a->f(x | y)`, or a bare `f()` on the element the rule
    /// is about. `arrow` tells a collection operation from a call on
    /// what the target is.
    Call {
        target: Option<Box<Expr>>,
        arrow: bool,
        name: String,
        args: Vec<Expr>,
        /// `->forAll(b | ...)`: the name bound over each element, and
        /// the body it is bound in.
        lambda: Option<(String, Box<Expr>)>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Sub,
    /// `owningNamespace.qualifiedName + '::' + escapedName()`, which is
    /// how a qualified name is put together and how a path is written.
    Add,
    /// `2..n`, which the one rule that counts a chain writes.
    Range,
    And,
    Or,
    Xor,
    Implies,
}

/// Parse one constraint. The error says where, because the only reader
/// of it is whoever is adding a rule the parser has not met.
pub(crate) fn parse(source: &str) -> Result<Expr, String> {
    let tokens = lex(source)?;
    let mut parser = Parser { tokens, at: 0 };
    let expr = parser.expression()?;
    if parser.at < parser.tokens.len() {
        return Err(format!(
            "expected the end of the expression, found `{}`",
            parser.tokens[parser.at].text
        ));
    }
    Ok(expr)
}

#[derive(Clone, Debug, PartialEq)]
struct Token {
    text: String,
    kind: Tok,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tok {
    Name,
    Number,
    Str,
    /// Anything else: `(`, `->`, `=`, and the rest.
    Punct,
}

fn lex(source: &str) -> Result<Vec<Token>, String> {
    // the two-character operators first, so `<>` is not read as `<`
    const LONG: [&str; 6] = ["->", "<>", "<=", ">=", "::", ".."];
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut at = 0usize;
    while at < chars.len() {
        let ch = chars[at];
        if ch.is_whitespace() {
            at += 1;
            continue;
        }
        if ch == '\'' {
            let mut text = String::new();
            at += 1;
            while at < chars.len() && chars[at] != '\'' {
                text.push(chars[at]);
                at += 1;
            }
            if at == chars.len() {
                return Err("a string that is never closed".to_string());
            }
            at += 1;
            tokens.push(Token {
                text,
                kind: Tok::Str,
            });
            continue;
        }
        if ch.is_ascii_digit() {
            let mut text = String::new();
            while at < chars.len() && chars[at].is_ascii_digit() {
                text.push(chars[at]);
                at += 1;
            }
            tokens.push(Token {
                text,
                kind: Tok::Number,
            });
            continue;
        }
        if ch.is_alphabetic() || ch == '_' {
            let mut text = String::new();
            while at < chars.len() && (chars[at].is_alphanumeric() || chars[at] == '_') {
                text.push(chars[at]);
                at += 1;
            }
            tokens.push(Token {
                text,
                kind: Tok::Name,
            });
            continue;
        }
        let rest: String = chars[at..].iter().take(2).collect();
        // `--` runs to the end of the line; `->` and a subtraction both
        // begin with the same character, so the three are told apart here
        if rest == "--" {
            while at < chars.len() && chars[at] != '\n' {
                at += 1;
            }
            continue;
        }
        let long = LONG.iter().find(|op| rest.starts_with(**op));
        let text = match long {
            Some(op) => op.to_string(),
            None => ch.to_string(),
        };
        at += text.chars().count();
        tokens.push(Token {
            text,
            kind: Tok::Punct,
        });
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    /// Take the next token where it is the punctuation or keyword given.
    fn eat(&mut self, text: &str) -> bool {
        let matched = self.peek().is_some_and(|token| token.text == text);
        if matched {
            self.at += 1;
        }
        matched
    }

    /// Take the `{` a collection literal opens with, and the element
    /// type written before it where one is.
    fn opens_collection(&mut self) -> bool {
        if self.eat("{") {
            return true;
        }
        let from = self.at;
        let named = self.eat("(") && self.peek().is_some_and(|it| it.kind == Tok::Name) && {
            self.at += 1;
            self.eat(")") && self.eat("{")
        };
        if !named {
            self.at = from;
        }
        named
    }

    fn expect(&mut self, text: &str) -> Result<(), String> {
        if self.eat(text) {
            return Ok(());
        }
        Err(match self.peek() {
            Some(token) => format!("expected `{text}`, found `{}`", token.text),
            None => format!("expected `{text}`, found the end of the expression"),
        })
    }

    fn expression(&mut self) -> Result<Expr, String> {
        self.implies()
    }

    /// `implies` binds loosest of all, and the metamodel writes it
    /// left to right.
    fn implies(&mut self) -> Result<Expr, String> {
        let mut left = self.or()?;
        while self.eat("implies") {
            let right = self.or()?;
            left = Expr::Binary(Op::Implies, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn or(&mut self) -> Result<Expr, String> {
        let mut left = self.and()?;
        loop {
            let op = if self.eat("or") {
                Op::Or
            } else if self.eat("xor") {
                Op::Xor
            } else {
                return Ok(left);
            };
            let right = self.and()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut left = self.comparison()?;
        while self.eat("and") {
            let right = self.comparison()?;
            left = Expr::Binary(Op::And, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn comparison(&mut self) -> Result<Expr, String> {
        let mut left = self.range()?;
        loop {
            let op = if self.eat("=") {
                Op::Eq
            } else if self.eat("<>") {
                Op::Ne
            } else if self.eat("<=") {
                Op::Le
            } else if self.eat(">=") {
                Op::Ge
            } else if self.eat("<") {
                Op::Lt
            } else if self.eat(">") {
                Op::Gt
            } else {
                return Ok(left);
            };
            let right = self.range()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    /// `2..n`: every whole number from one to the other.
    fn range(&mut self) -> Result<Expr, String> {
        let left = self.additive()?;
        if self.eat("..") {
            let right = self.additive()?;
            return Ok(Expr::Binary(Op::Range, Box::new(left), Box::new(right)));
        }
        Ok(left)
    }

    fn additive(&mut self) -> Result<Expr, String> {
        let mut left = self.unary()?;
        loop {
            let op = if self.eat("-") {
                Op::Sub
            } else if self.eat("+") {
                Op::Add
            } else {
                break;
            };
            let right = self.unary()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat("not") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        self.postfix()
    }

    /// What follows a value: navigation, a call on it, or a collection
    /// operation.
    fn postfix(&mut self) -> Result<Expr, String> {
        let mut left = self.primary()?;
        loop {
            let arrow = if self.eat("->") {
                true
            } else if self.eat(".") {
                false
            } else {
                return Ok(left);
            };
            let name = self.name()?;
            left = match self.arguments()? {
                Some((args, lambda)) => Expr::Call {
                    target: Some(Box::new(left)),
                    arrow,
                    name,
                    args,
                    lambda,
                },
                // `a->size` is written without them as often as with
                None if arrow => Expr::Call {
                    target: Some(Box::new(left)),
                    arrow,
                    name,
                    args: Vec::new(),
                    lambda: None,
                },
                None => Expr::Nav(Box::new(left), name),
            };
        }
    }

    /// The argument list of a call, where there is one. A lambda is one
    /// argument written `name | body`, with the type it is sometimes
    /// declared with passed over.
    #[allow(clippy::type_complexity)]
    fn arguments(&mut self) -> Result<Option<(Vec<Expr>, Option<(String, Box<Expr>)>)>, String> {
        if !self.eat("(") {
            return Ok(None);
        }
        if self.eat(")") {
            return Ok(Some((Vec::new(), None)));
        }
        // `x | body` and `x : T | body` both bind `x`; anything else is
        // an ordinary argument
        if let Some(bound) = self.lambda_binder() {
            let body = self.expression()?;
            self.expect(")")?;
            return Ok(Some((Vec::new(), Some((bound, Box::new(body))))));
        }
        let mut args = vec![self.expression()?];
        while self.eat(",") {
            args.push(self.expression()?);
        }
        self.expect(")")?;
        Ok(Some((args, None)))
    }

    /// `x |` or `x : T |` at the head of an argument list, consumed.
    fn lambda_binder(&mut self) -> Option<String> {
        let start = self.at;
        let name = match self.peek() {
            Some(token) if token.kind == Tok::Name => token.text.clone(),
            _ => return None,
        };
        self.at += 1;
        if self.eat(":") {
            // the declared type, which the binder does not need
            if self.type_name().is_err() {
                self.at = start;
                return None;
            }
        }
        if self.eat("|") {
            return Some(name);
        }
        self.at = start;
        None
    }

    fn name(&mut self) -> Result<String, String> {
        match self.peek() {
            Some(token) if token.kind == Tok::Name => {
                let name = token.text.clone();
                self.at += 1;
                Ok(name)
            }
            Some(token) => Err(format!("expected a name, found `{}`", token.text)),
            None => Err("expected a name, found the end of the expression".to_string()),
        }
    }

    fn primary(&mut self) -> Result<Expr, String> {
        if self.eat("(") {
            let inner = self.expression()?;
            self.expect(")")?;
            return Ok(inner);
        }
        if self.eat("if") {
            let condition = self.expression()?;
            self.expect("then")?;
            let then = self.expression()?;
            self.expect("else")?;
            let otherwise = self.expression()?;
            self.expect("endif")?;
            return Ok(Expr::If(
                Box::new(condition),
                Box::new(then),
                Box::new(otherwise),
            ));
        }
        if self.eat("let") {
            let bound = self.name()?;
            if self.eat(":") {
                // the declared type again: what the value is says it
                self.type_name()?;
            }
            self.expect("=")?;
            let value = self.expression()?;
            self.expect("in")?;
            let body = self.expression()?;
            return Ok(Expr::Let(bound, Box::new(value), Box::new(body)));
        }
        let token = self
            .peek()
            .ok_or_else(|| "expected a value, found the end of the expression".to_string())?
            .clone();
        match token.kind {
            Tok::Number => {
                self.at += 1;
                token
                    .text
                    .parse()
                    .map(Expr::Int)
                    .map_err(|_| format!("`{}` is not a number this can hold", token.text))
            }
            Tok::Str => {
                self.at += 1;
                Ok(Expr::Str(token.text))
            }
            Tok::Name => {
                self.at += 1;
                match token.text.as_str() {
                    "null" => Ok(Expr::Null),
                    "true" => Ok(Expr::Bool(true)),
                    "false" => Ok(Expr::Bool(false)),
                    // `Set{a, b}` and its kin: what is in it is what
                    // matters, and which kind of collection it is does
                    // not, since nothing here counts duplicates. The
                    // metamodel writes one of them as `Set(Element){}`,
                    // naming the type its emptiness is empty of, which
                    // says nothing the values do not.
                    "Set" | "OrderedSet" | "Sequence" | "Bag" if self.opens_collection() => {
                        let mut items = Vec::new();
                        if !self.eat("}") {
                            items.push(self.expression()?);
                            while self.eat(",") {
                                items.push(self.expression()?);
                            }
                            self.expect("}")?;
                        }
                        Ok(Expr::Collection(items))
                    }
                    // `FeatureDirectionKind::_'in'`: a literal whose
                    // name is a keyword is escaped the way the notation
                    // escapes any other
                    _ if self.eat("::") => Ok(Expr::Enum(token.text, self.enum_literal()?)),
                    _ => match self.arguments()? {
                        Some((args, lambda)) => Ok(Expr::Call {
                            target: None,
                            arrow: false,
                            name: token.text,
                            args,
                            lambda,
                        }),
                        None => Ok(Expr::Name(token.text)),
                    },
                }
            }
            // `*` is a value here and nothing else: this subset has no
            // multiplication for it to be
            Tok::Punct if token.text == "*" => {
                self.at += 1;
                Ok(Expr::Unlimited)
            }
            Tok::Punct => Err(format!("expected a value, found `{}`", token.text)),
        }
    }

    /// The name of an enumeration literal, escaped where it is a
    /// keyword.
    fn enum_literal(&mut self) -> Result<String, String> {
        if self.eat("_") {
            return match self.peek() {
                Some(token) if token.kind == Tok::Str => {
                    let text = token.text.clone();
                    self.at += 1;
                    Ok(text)
                }
                _ => Err("expected the escaped name of a literal".to_string()),
            };
        }
        self.name()
    }

    /// A type as `let` and a lambda declare it: a name, one reached
    /// through a package, or a collection of one. None of it is kept --
    /// what the value turns out to be is what the rule is evaluated
    /// against -- so this only has to get past it.
    fn type_name(&mut self) -> Result<String, String> {
        let mut name = self.name()?;
        while self.eat("::") {
            name = self.name()?;
        }
        if self.eat("(") {
            self.type_name()?;
            self.expect(")")?;
        }
        Ok(name)
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    /// The derivations and operations the specification writes so that
    /// they cannot be read as they stand, which is a different list
    /// from the constraints and a longer one. It is read here from the
    /// metamodel's own text, before `rules::UNCLOSED` closes any of it.
    ///
    /// Two kinds sit in it. Some are the specification's own text:
    /// `Namespace::resolveGlobal` and three of its neighbours are
    /// written as prose about what they would do rather than as OCL,
    /// and `Expression::modelLevelEvaluable` stops in the middle of a
    /// `forAll(` it never closes, as `deriveFeatureCrossFeature` and
    /// `deriveTransitionUsageSource` each stop one `endif` short --
    /// those three are closed and read, since the grammar leaves one
    /// place for the closing, and `ControlNode::multiplicityHasBounds`
    /// is a fourth. The rest are the specification's own too, or this
    /// subset's: `OperatorExpression::instantiatedType` writes a string
    /// in double quotes, which OCL spells with single ones.
    ///
    /// It is pinned so that the list cannot grow unnoticed, and so that
    /// closing one of the gaps shows up here as the gain it is.
    #[test]
    fn what_the_specification_writes_that_this_subset_cannot_read() {
        const UNREADABLE: [&str; 17] = [
            "ControlNode::multiplicityHasBounds",
            "Expression::modelLevelEvaluable",
            "Feature::isFeaturingType",
            "Feature::ownedCrossFeature",
            "FeatureChainExpression::sourceTargetFeature",
            "Membership::isDistinguishableFrom",
            "MetadataFeature::evaluateFeature",
            "MetadataFeature::syntaxElement",
            "Namespace::qualificationOf",
            "Namespace::resolveGlobal",
            "Namespace::unqualifiedNameOf",
            "Namespace::visibilityOf",
            "NamespaceImport::importedMemberships",
            "OperatorExpression::instantiatedType",
            "Type::multiplicities",
            "derive deriveFeatureCrossFeature",
            "derive deriveTransitionUsageSource",
        ];
        let mut refused = Vec::new();
        for rule in sysml_model::DERIVATIONS {
            // the metamodel writes `property = expression`, and where
            // it wrote the expression alone the rule's name says which
            let body = match rule.ocl.split_once('=') {
                Some((head, body)) if head.trim().chars().all(char::is_alphanumeric) => body,
                _ => rule.ocl,
            };
            if let Err(why) = parse(body) {
                refused.push((format!("derive {}", rule.name), why));
            }
        }
        for op in sysml_model::OPERATIONS {
            if let Err(why) = parse(op.ocl) {
                refused.push((format!("{}::{}", op.metaclass.name(), op.name), why));
            }
        }
        refused.sort();
        refused.dedup_by(|a, b| a.0 == b.0);
        let names: Vec<&str> = refused.iter().map(|(name, _)| name.as_str()).collect();
        let said = refused
            .iter()
            .map(|(name, why)| format!("{name}: {why}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(names, UNREADABLE, "{said}");
    }

    /// One constraint of the specification cannot be read, and it is a
    /// defect in the specification's own text rather than a gap in this
    /// subset: `validateFeatureEndNoDirection` is written `isEnd
    /// implied direction = null`, and `implied` is not an OCL operator.
    /// `rules::UNCLOSED` repairs it before it is run, on the pilot
    /// implementation's word rather than this parser's; what is pinned
    /// here is the specification's own text, which still says `implied`.
    ///
    /// It is named here so that a second one cannot appear unnoticed:
    /// what this test holds is that the subset reads everything else.
    #[test]
    fn every_constraint_the_specification_states_parses_but_its_own_one_defect() {
        const DEFECTIVE: [&str; 1] = ["validateFeatureEndNoDirection"];
        let mut refused = Vec::new();
        for rule in sysml_model::RULES {
            if let Err(why) = parse(rule.ocl) {
                refused.push((rule.name, why));
            }
        }
        let names: Vec<&str> = refused.iter().map(|(name, _)| *name).collect();
        // built whether or not it is needed, so that what it says is
        // the same thing this test ran
        let said = refused
            .iter()
            .map(|(name, why)| format!("{name}: {why}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(names, DEFECTIVE, "{said}");
        assert_eq!(sysml_model::RULES.len(), 180);
    }

    /// The subset reads what the specification writes; what it does not
    /// read, it says where.
    #[test]
    fn what_cannot_be_read_says_where_it_stopped() {
        // every way the reading can stop, and the words it stops with
        for (source, complaint) in [
            ("'never closed", "never closed"),
            ("if a then b", "expected `else`"),
            ("(a", "expected `)`, found the end"),
            ("a.", "expected a name, found the end"),
            ("a.1", "expected a name, found `1`"),
            ("= 1", "expected a value, found `=`"),
            ("", "expected a value, found the end"),
            ("Kind::_ 1", "escaped name"),
            ("a b", "expected the end of the expression"),
            ("let x", "expected `=`"),
            ("Set{1}->select(x : | true)", "expected `)`"),
        ] {
            let why = parse(source).expect_err(&format!("`{source}` is not OCL this reads"));
            assert!(why.contains(complaint), "`{source}`: {why}");
        }

        // and the shapes that are easy to write and easy to miss
        for source in [
            // a collection operation written without its parentheses
            "chainingFeature->size = 2",
            // a type reached through a package, in a `let` and in a
            // lambda alike
            "let x : ScalarValues::Integer = 1 in x = 1",
            "Set{1}->forAll(n : ScalarValues::Integer | n = 1)",
            // an escaped literal, and a nested collection type
            "direction = FeatureDirectionKind::_'in'",
            "let x : OrderedSet(Feature) = feature in x->isEmpty()",
            // `Set(Element){}` names what its emptiness is empty of;
            // the same words with no literal after them are a name
            // like any other, and the reading steps back to read them
            // that way
            "Sequence(Feature)->isEmpty()",
        ] {
            assert!(
                parse(source).is_ok(),
                "`{source}` is OCL the specification writes"
            );
        }
    }
}

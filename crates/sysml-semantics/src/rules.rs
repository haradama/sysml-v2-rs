//! The specification's well-formedness constraints, evaluated.
//!
//! [`sysml_model::RULES`] states them in OCL and [`crate::ocl`] reads
//! them; this runs them over a resolved model and says which do not
//! hold.
//!
//! **Three answers, not two.** The abstract syntax the constraints
//! navigate is larger than the model this toolchain builds: a rule may
//! reach for `operator` or `result`, which the builder does not
//! materialise. Answering `false` there would report a violation the
//! model never had, and answering `true` would say a model is sound
//! when nothing looked. So an unanswerable navigation is [`Val::Unknown`],
//! anything it touches becomes unknown in turn, and a rule that comes
//! out unknown is reported as *not evaluated* rather than as either.
//! What can be checked is checked; what cannot is named.

use std::collections::HashMap;

use sysml_model::{ElementId, ElementKind, Value};

use crate::ocl::{self, Expr, Op};
use crate::Workspace;

/// One constraint that does not hold, and of what.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    /// The constraint, by the name the specification gives it.
    pub rule: &'static str,
    /// What the specification says it means.
    pub says: &'static str,
    /// The element it does not hold of.
    pub element: ElementId,
}

/// What running the constraints over a model came to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Checked {
    pub violations: Vec<Violation>,
    /// The constraints that were asked of something and held, by name.
    pub held: Vec<&'static str>,
    /// Constraints no element of the model could answer, by name and
    /// why -- the property the abstract syntax has and this model does
    /// not, or the defect in the specification's own text.
    pub unevaluated: Vec<(&'static str, String)>,
}

impl Workspace {
    /// Run every constraint the specification states over the elements
    /// of `files`.
    ///
    /// A constraint is asked of every element of the metaclass it is
    /// about, subtypes included, since that is what "of every instance"
    /// means.
    pub fn check_rules(&mut self, files: &[usize]) -> Checked {
        let mut parsed: Vec<(&sysml_model::Rule, Option<Expr>, Option<String>)> = Vec::new();
        for rule in sysml_model::RULES {
            match ocl::parse(rule.ocl) {
                Ok(expr) => parsed.push((rule, Some(expr), None)),
                Err(why) => parsed.push((rule, None, Some(why))),
            }
        }
        // Grouped by metaclass once. Asked element by element, every
        // rule walks the whole model to find the few it is about, and
        // the whole model is where this is meant to be run.
        let mut by_kind: HashMap<ElementKind, Vec<ElementId>> = HashMap::new();
        for &file in files {
            for &elem in self.file_elements(file) {
                by_kind
                    .entry(self.model().kind(elem))
                    .or_default()
                    .push(elem);
            }
        }

        let mut checked = Checked::default();
        for (rule, expr, refused) in &parsed {
            let Some(expr) = expr else {
                checked
                    .unevaluated
                    .push((rule.name, refused.clone().expect("refused, or it parsed")));
                continue;
            };
            let about: Vec<ElementId> = by_kind
                .iter()
                .filter(|(kind, _)| kind.is_a(rule.metaclass))
                .flat_map(|(_, them)| them.iter().copied())
                .collect();
            let mut asked = false;
            let mut unknown = None;
            for elem in about {
                match self.holds(expr, elem) {
                    Ok(true) => asked = true,
                    Ok(false) => {
                        asked = true;
                        checked.violations.push(Violation {
                            rule: rule.name,
                            says: rule.says,
                            element: elem,
                        });
                    }
                    // an element that cannot answer says nothing about
                    // whether another one can
                    Err(why) => unknown = unknown.or(Some(why)),
                }
            }
            match (asked, unknown) {
                (true, _) => checked.held.push(rule.name),
                (false, Some(why)) => checked.unevaluated.push((rule.name, why)),
                // no element of that metaclass is in the model at all,
                // which is not a rule failing to be evaluated
                (false, None) => {}
            }
        }
        checked
    }

    /// Evaluate one constraint of one element: whether it holds, or why
    /// this model cannot say.
    fn holds(&mut self, expr: &Expr, elem: ElementId) -> Result<bool, String> {
        let mut scope = Scope {
            ws: self,
            bound: HashMap::new(),
            self_: elem,
        };
        match scope.eval(expr) {
            Val::Bool(it) => Ok(it),
            Val::Unknown(why) => Err(why),
            // a constraint whose body comes to a value rather than a
            // condition is one this cannot judge either
            other => Err(format!("the constraint came to {other:?}")),
        }
    }
}

/// What an OCL expression comes to.
#[derive(Clone, Debug, PartialEq)]
enum Val {
    /// Nothing here can answer: a property the abstract syntax has and
    /// this model does not, or an operation not implemented. Carries
    /// what could not be answered, for the report.
    Unknown(String),
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    Elem(ElementId),
    Set(Vec<Val>),
}

impl Val {
    /// The elements of a value, whether it is one thing or many.
    fn many(&self) -> Vec<Val> {
        match self {
            Val::Set(items) => items.clone(),
            Val::Null => Vec::new(),
            other => vec![other.clone()],
        }
    }

    fn unknown(&self) -> Option<Val> {
        match self {
            Val::Unknown(_) => Some(self.clone()),
            Val::Set(items) => items.iter().find_map(Val::unknown),
            _ => None,
        }
    }
}

struct Scope<'a> {
    ws: &'a mut Workspace,
    /// `let` and lambda variables.
    bound: HashMap<String, Val>,
    self_: ElementId,
}

impl Scope<'_> {
    fn eval(&mut self, expr: &Expr) -> Val {
        match expr {
            Expr::Null => Val::Null,
            Expr::Bool(it) => Val::Bool(*it),
            Expr::Int(it) => Val::Int(*it),
            Expr::Str(it) => Val::Str(it.clone()),
            // an enumeration literal is the word the model stores
            Expr::Enum(_, literal) => Val::Str(literal.clone()),
            // `Sequence{2..n}` is the whole numbers from two to n, not
            // a collection holding one collection: what a range comes
            // to is already the elements.
            Expr::Collection(items) => Val::Set(
                items
                    .iter()
                    .flat_map(|item| self.eval(item).many())
                    .collect(),
            ),
            Expr::Name(name) => self.name(name),
            Expr::Not(inner) => match self.eval(inner) {
                Val::Bool(it) => Val::Bool(!it),
                other => unknown_from(&other, "not"),
            },
            Expr::If(condition, then, otherwise) => match self.eval(condition) {
                Val::Bool(true) => self.eval(then),
                Val::Bool(false) => self.eval(otherwise),
                other => unknown_from(&other, "the condition of an `if`"),
            },
            Expr::Let(name, value, body) => {
                let value = self.eval(value);
                let shadowed = self.bound.insert(name.clone(), value);
                let result = self.eval(body);
                match shadowed {
                    Some(old) => self.bound.insert(name.clone(), old),
                    None => self.bound.remove(name),
                };
                result
            }
            Expr::Binary(op, left, right) => self.binary(*op, left, right),
            Expr::Nav(target, name) => {
                let target = self.eval(target);
                self.navigate(&target, name)
            }
            Expr::Call {
                target,
                arrow,
                name,
                args,
                lambda,
            } => self.call(target.as_deref(), *arrow, name, args, lambda.as_ref()),
        }
    }

    /// A bare name: a variable in scope, `self`, or a property of the
    /// element the rule is about.
    fn name(&mut self, name: &str) -> Val {
        if let Some(value) = self.bound.get(name) {
            return value.clone();
        }
        if name == "self" {
            return Val::Elem(self.self_);
        }
        let target = Val::Elem(self.self_);
        self.navigate(&target, name)
    }

    fn binary(&mut self, op: Op, left: &Expr, right: &Expr) -> Val {
        // the three that answer without their second half
        let left = self.eval(left);
        match (op, &left) {
            (Op::And, Val::Bool(false)) => return Val::Bool(false),
            (Op::Or, Val::Bool(true)) => return Val::Bool(true),
            (Op::Implies, Val::Bool(false)) => return Val::Bool(true),
            _ => {}
        }
        let right = self.eval(right);
        match (op, &left, &right) {
            (Op::Eq, _, _) => equal(&left, &right),
            (Op::Ne, _, _) => match equal(&left, &right) {
                Val::Bool(it) => Val::Bool(!it),
                other => other,
            },
            (Op::And, Val::Bool(a), Val::Bool(b)) => Val::Bool(*a && *b),
            (Op::Or, Val::Bool(a), Val::Bool(b)) => Val::Bool(*a || *b),
            (Op::Xor, Val::Bool(a), Val::Bool(b)) => Val::Bool(a != b),
            (Op::Implies, Val::Bool(a), Val::Bool(b)) => Val::Bool(!a || *b),
            (Op::Lt, Val::Int(a), Val::Int(b)) => Val::Bool(a < b),
            (Op::Le, Val::Int(a), Val::Int(b)) => Val::Bool(a <= b),
            (Op::Gt, Val::Int(a), Val::Int(b)) => Val::Bool(a > b),
            (Op::Ge, Val::Int(a), Val::Int(b)) => Val::Bool(a >= b),
            (Op::Sub, Val::Int(a), Val::Int(b)) => Val::Int(a - b),
            (Op::Range, Val::Int(a), Val::Int(b)) => Val::Set((*a..=*b).map(Val::Int).collect()),
            _ => match right.unknown() {
                Some(unknown) => unknown,
                None => unknown_from(&left, "an operand"),
            },
        }
    }

    /// `a.b`: the property `b` of everything `a` came to.
    fn navigate(&mut self, target: &Val, name: &str) -> Val {
        match target {
            Val::Unknown(_) => target.clone(),
            Val::Null => Val::Null,
            // OCL collects a property over a collection
            Val::Set(items) => {
                let mut out = Vec::new();
                for item in items {
                    // a collection never holds nothing -- `Set{null}`
                    // is the empty collection -- so what comes back is
                    // either one thing or several
                    match self.navigate(&item.clone(), name) {
                        Val::Set(more) => out.extend(more),
                        one => out.push(one),
                    }
                }
                Val::Set(out)
            }
            Val::Elem(elem) => self.property(*elem, name),
            other => unknown_from(other, &format!("`{name}` of it")),
        }
    }

    /// One property of one element, or [`Val::Unknown`] where this model
    /// does not carry it.
    fn property(&mut self, elem: ElementId, name: &str) -> Val {
        let model = self.ws.model();
        // What the metamodel calls an owned X is an owned element that
        // is an X, and an owning Y the owner where the owner is a Y.
        // Both are the containment the model does keep.
        if let Some(kind) = owned_kind(name) {
            let owned: Vec<Val> = model
                .owned(elem)
                .iter()
                .filter(|&&child| model.kind(child).is_a(kind))
                .map(|&child| Val::Elem(child))
                .collect();
            // Finding none of them is the ambiguous answer: the builder
            // reifies some of the relationships the abstract syntax has
            // and not others, so an empty answer is as likely to be one
            // it does not build as one the element does not have.
            if owned.is_empty() {
                return Val::Unknown(format!(
                    "`{name}` is empty here, and this model does not build every {} \
                     the abstract syntax has",
                    kind.name()
                ));
            }
            return Val::Set(owned);
        }
        // `owningType` and its kin are not the containment this model
        // keeps. A `snapshot` written inside a `first ... then` is
        // nested under the succession here and owned by the enclosing
        // type in the abstract syntax, and answering with the one where
        // the rule means the other reports a violation of a model that
        // is sound. Whoever makes the builder say which is which can
        // take these off the list.
        if owning_kind(name).is_some() || name == "owner" {
            return Val::Unknown(format!(
                "`{name}` is ownership in the abstract syntax, which is not the containment \
                 this model builds"
            ));
        }
        match model.get(elem, name) {
            Some(Value::Bool(it)) => Val::Bool(*it),
            Some(Value::String(it)) => Val::Str(it.clone()),
            Some(Value::EnumLit(it)) => Val::Str(it.to_string()),
            Some(Value::Int(it)) => Val::Int(*it),
            Some(Value::Real(_)) => Val::Unknown(format!("`{name}` holds a real number")),
            Some(Value::Ref(it)) => Val::Elem(*it),
            Some(Value::RefList(them)) => Val::Set(them.iter().map(|&it| Val::Elem(it)).collect()),
            // A property with nothing under it is not an empty one.
            // Whether the builder would have filled it in is not
            // something the absence can say -- `relatedFeature` is kept
            // for some metaclasses and not others -- and reading it as
            // empty is how a checker comes to report a violation of a
            // model that never said anything of the sort.
            None => Val::Unknown(format!(
                "`{name}` is part of the abstract syntax that this model does not build here"
            )),
        }
    }

    fn call(
        &mut self,
        target: Option<&Expr>,
        arrow: bool,
        name: &str,
        args: &[Expr],
        lambda: Option<&(String, Box<Expr>)>,
    ) -> Val {
        let target = match target {
            Some(expr) => self.eval(expr),
            None => Val::Elem(self.self_),
        };
        if let Some(unknown) = target.unknown() {
            return unknown;
        }
        // a collection operation reads its target as many things; an
        // operation on an element reads it as one
        if arrow || COLLECTION.contains(&name) {
            return self.collection(&target, name, args, lambda);
        }
        self.operation(&target, name, args)
    }

    /// The collection operations, over whatever the target came to.
    fn collection(
        &mut self,
        target: &Val,
        name: &str,
        args: &[Expr],
        lambda: Option<&(String, Box<Expr>)>,
    ) -> Val {
        let items = target.many();
        // The operations that take one. Evaluated here so that an
        // argument nothing can answer stops the operation rather than
        // being compared against and found unequal, which would make an
        // unknown come out as a definite `false`.
        const TAKES: [&str; 7] = [
            "at",
            "including",
            "excluding",
            "includes",
            "excludes",
            "union",
            "intersection",
        ];
        let taken = if TAKES.contains(&name) {
            let argument = self.argument(args);
            if let Some(unknown) = argument.unknown() {
                return unknown;
            }
            argument
        } else {
            Val::Null
        };
        match name {
            "size" => Val::Int(items.len() as i64),
            "isEmpty" => Val::Bool(items.is_empty()),
            "notEmpty" => Val::Bool(!items.is_empty()),
            "asSet" | "asOrderedSet" | "asBag" | "asSequence" | "flatten" => {
                Val::Set(items.iter().flat_map(Val::many).collect())
            }
            "first" => items.first().cloned().unwrap_or(Val::Null),
            "last" => items.last().cloned().unwrap_or(Val::Null),
            "at" => match taken {
                // OCL counts from one
                Val::Int(index) if index >= 1 => {
                    items.get(index as usize - 1).cloned().unwrap_or(Val::Null)
                }
                other => unknown_from(&other, "the index of `at`"),
            },
            "including" => {
                let mut out = items;
                out.push(taken);
                Val::Set(out)
            }
            "excluding" => {
                let dropped = taken;
                Val::Set(
                    items
                        .into_iter()
                        .filter(|item| equal(item, &dropped) != Val::Bool(true))
                        .collect(),
                )
            }
            "includes" | "excludes" => {
                let wanted = taken;
                let found = items
                    .iter()
                    .any(|item| equal(item, &wanted) == Val::Bool(true));
                Val::Bool(if name == "includes" { found } else { !found })
            }
            "union" => {
                let mut out = items;
                out.extend(taken.many());
                Val::Set(out)
            }
            "intersection" => {
                let other = taken.many();
                Val::Set(
                    items
                        .into_iter()
                        .filter(|item| other.iter().any(|it| equal(item, it) == Val::Bool(true)))
                        .collect(),
                )
            }
            "isUnique" => {
                // written over a lambda, and what it says is that no two
                // of them agree
                let mut seen: Vec<Val> = Vec::new();
                for item in items {
                    let value = match lambda {
                        Some(_) => self.over(&item, lambda),
                        None => item,
                    };
                    if let Some(unknown) = value.unknown() {
                        return unknown;
                    }
                    if seen.iter().any(|it| equal(it, &value) == Val::Bool(true)) {
                        return Val::Bool(false);
                    }
                    seen.push(value);
                }
                Val::Bool(true)
            }
            "selectByKind" | "selectAsKind" => {
                let Some(kind) = args.first().and_then(metaclass_named) else {
                    return Val::Unknown(format!("`{name}` of a kind this does not know"));
                };
                Val::Set(
                    items
                        .into_iter()
                        .filter(|item| {
                            matches!(item, Val::Elem(id)
                            if self.ws.model().kind(*id).is_a(kind))
                        })
                        .collect(),
                )
            }
            "forAll" | "exists" | "select" | "reject" | "collect" | "any" => {
                let mut kept = Vec::new();
                for item in items {
                    let value = self.over(&item, lambda);
                    if let Some(unknown) = value.unknown() {
                        return unknown;
                    }
                    match (name, &value) {
                        ("collect", _) => kept.push(value),
                        ("forAll", Val::Bool(false)) => return Val::Bool(false),
                        ("exists", Val::Bool(true)) => return Val::Bool(true),
                        ("select", Val::Bool(true)) | ("any", Val::Bool(true)) => kept.push(item),
                        ("reject", Val::Bool(false)) => kept.push(item),
                        ("forAll" | "exists" | "select" | "reject" | "any", Val::Bool(_)) => {}
                        _ => return unknown_from(&value, &format!("the body of `{name}`")),
                    }
                }
                match name {
                    "forAll" => Val::Bool(true),
                    "exists" => Val::Bool(false),
                    "any" => kept.first().cloned().unwrap_or(Val::Null),
                    _ => Val::Set(kept),
                }
            }
            _ => Val::Unknown(format!("`->{name}` is not implemented")),
        }
    }

    /// A lambda body, over one element of a collection.
    fn over(&mut self, item: &Val, lambda: Option<&(String, Box<Expr>)>) -> Val {
        let Some((bound, body)) = lambda else {
            // `->select(visibility = VisibilityKind::public)` binds
            // nothing and reads its body against each element
            return Val::Unknown("a collection operation written without a variable".to_string());
        };
        let shadowed = self.bound.insert(bound.clone(), item.clone());
        let result = self.eval(body);
        match shadowed {
            Some(old) => self.bound.insert(bound.clone(), old),
            None => self.bound.remove(bound),
        };
        result
    }

    /// An operation on one thing rather than on a collection of them.
    fn operation(&mut self, target: &Val, name: &str, args: &[Expr]) -> Val {
        match name {
            "oclIsKindOf" | "oclIsTypeOf" => {
                let (Some(kind), Val::Elem(elem)) =
                    (args.first().and_then(metaclass_named), target)
                else {
                    return Val::Unknown(format!("`{name}` of a kind this does not know"));
                };
                let actual = self.ws.model().kind(*elem);
                Val::Bool(if name == "oclIsKindOf" {
                    actual.is_a(kind)
                } else {
                    actual == kind
                })
            }
            // the metamodel casts to read a property, and reading a
            // property does not need the cast
            "oclAsType" => target.clone(),
            "specializes" => {
                let (Val::Elem(elem), Val::Elem(up)) = (target, self.argument(args)) else {
                    return Val::Unknown(
                        "`specializes` of something that is not an element".to_string(),
                    );
                };
                Val::Bool(self.specializes(*elem, up))
            }
            "specializesFromLibrary" => {
                let (Val::Elem(elem), Val::Str(qualified)) = (target, self.argument(args)) else {
                    return Val::Unknown(
                        "`specializesFromLibrary` of something that is not an element".to_string(),
                    );
                };
                // by the name it answers to, then by the whole of it:
                // the index is over declared names, and the root
                // namespace is no scope a library name resolves from
                let declared = qualified.rsplit("::").next().unwrap_or(&qualified);
                let found = self
                    .ws
                    .search_names(declared, 500)
                    .into_iter()
                    .find(|&it| self.ws.qualified_name_of(it) == qualified);
                match found {
                    Some(up) => Val::Bool(self.specializes(*elem, up)),
                    // the library is not loaded, which is not the model
                    // failing a rule
                    None => Val::Unknown(format!("`{qualified}` is not in this workspace")),
                }
            }
            _ => Val::Unknown(format!("`{name}()` is not implemented")),
        }
    }

    /// Whether `elem` specializes `up`, directly or through anything in
    /// between. An element specializes itself, as OCL's `specializes`
    /// does.
    fn specializes(&mut self, elem: ElementId, up: ElementId) -> bool {
        let mut queue = vec![elem];
        let mut seen = Vec::new();
        while let Some(current) = queue.pop() {
            if current == up {
                return true;
            }
            if seen.contains(&current) {
                continue;
            }
            seen.push(current);
            queue.extend(self.ws.supertypes(current));
        }
        false
    }

    fn argument(&mut self, args: &[Expr]) -> Val {
        match args.first() {
            Some(expr) => self.eval(expr),
            None => Val::Unknown("an operation written without the argument it takes".to_string()),
        }
    }
}

/// An unknown carrying where it came from, or a fresh one saying what
/// could not be judged.
fn unknown_from(value: &Val, what: &str) -> Val {
    match value.unknown() {
        Some(unknown) => unknown,
        None => Val::Unknown(format!("{what} came to {value:?}")),
    }
}

/// Two values, compared the way OCL compares them.
fn equal(left: &Val, right: &Val) -> Val {
    if let Some(unknown) = left.unknown().or_else(|| right.unknown()) {
        return unknown;
    }
    match (left, right) {
        // a property with nothing in it is the empty collection or null
        // depending on how the metamodel declared it, and a rule asking
        // whether it is null means the same by both
        (Val::Null, Val::Set(items)) | (Val::Set(items), Val::Null) => Val::Bool(items.is_empty()),
        _ => Val::Bool(left == right),
    }
}

/// `ownedX` where every owned element that is an `X` is one.
fn owned_kind(name: &str) -> Option<ElementKind> {
    let kind = match name {
        "ownedRelationship" => ElementKind::Relationship,
        "ownedMembership" => ElementKind::Membership,
        "ownedSpecialization" => ElementKind::Specialization,
        "ownedSubsetting" => ElementKind::Subsetting,
        "ownedRedefinition" => ElementKind::Redefinition,
        "ownedFeatureMembership" => ElementKind::FeatureMembership,
        "ownedImport" => ElementKind::Import,
        _ => return None,
    };
    Some(kind)
}

/// `owningX` where the owner is an `X`.
fn owning_kind(name: &str) -> Option<ElementKind> {
    let kind = match name {
        "owningType" => ElementKind::Type,
        "owningNamespace" => ElementKind::Namespace,
        "owningMembership" => ElementKind::Membership,
        "owningFeatureMembership" => ElementKind::FeatureMembership,
        "owningRelationship" => ElementKind::Relationship,
        _ => return None,
    };
    Some(kind)
}

/// The metaclass a type name in an OCL expression stands for.
fn metaclass_named(expr: &Expr) -> Option<ElementKind> {
    match expr {
        Expr::Name(name) => ElementKind::from_name(name),
        _ => None,
    }
}

/// The operations that read their target as a collection even where the
/// metamodel wrote them with a dot.
const COLLECTION: [&str; 4] = ["size", "isEmpty", "notEmpty", "asSet"];

#[cfg(test)]
impl Workspace {
    /// Evaluate one OCL expression of one element: `Some(true)`,
    /// `Some(false)`, or `None` where this model cannot answer.
    ///
    /// The constraints the specification states reach for parts of the
    /// abstract syntax a model may not have, so most of them cannot be
    /// asked of a model built here. What can be tested for itself is
    /// the evaluation, and this is what tests it.
    fn judge(&mut self, ocl: &str, elem: ElementId) -> Option<bool> {
        let expr = ocl::parse(ocl).expect("the test writes OCL this reads");
        self.holds(&expr, elem).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A workspace over one file, and the element declared under `name`.
    fn about(source: &str, name: &str) -> (Workspace, ElementId) {
        let mut ws = Workspace::new();
        ws.add_file("test.sysml", source);
        ws.resolve_all();
        let elem = ws
            .named_elements()
            .find(|(_, declared)| *declared == name)
            .map(|(id, _)| id)
            .expect("the element is declared");
        (ws, elem)
    }

    #[test]
    fn the_evaluation_answers_what_the_model_carries() {
        let (mut ws, car) = about(
            "part def Source;\nabstract part def Car :> Source {\n\tin part w : Source;\n}\n",
            "Car",
        );

        // a property the model keeps, read from the element the rule is
        // about and through a navigation
        assert_eq!(ws.judge("isAbstract = true", car), Some(true));
        assert_eq!(ws.judge("declaredName = 'Car'", car), Some(true));
        assert_eq!(ws.judge("declaredName <> 'Lorry'", car), Some(true));

        // the metaclass tests, over the element and over a collection
        assert_eq!(
            ws.judge("self.oclIsKindOf(PartDefinition)", car),
            Some(true)
        );
        assert_eq!(ws.judge("self.oclIsKindOf(Behavior)", car), Some(false));
        assert_eq!(ws.judge("self.oclIsTypeOf(Definition)", car), Some(false));

        // the logical operators, including the three that answer
        // without their second half
        assert_eq!(ws.judge("false and unmodelled", car), Some(false));
        assert_eq!(ws.judge("true or unmodelled", car), Some(true));
        assert_eq!(ws.judge("false implies unmodelled", car), Some(true));
        assert_eq!(ws.judge("not (declaredName = 'Lorry')", car), Some(true));
        assert_eq!(ws.judge("isAbstract xor false", car), Some(true));

        // `if`, `let`, arithmetic and the range
        assert_eq!(
            ws.judge("if isAbstract then 1 else 2 endif = 1", car),
            Some(true)
        );
        assert_eq!(
            ws.judge("let n : Integer = 3 in n - 1 = 2", car),
            Some(true)
        );
        assert_eq!(
            ws.judge(
                "Sequence{1..3}->size() = 3 and 2 <= 3 and 3 >= 3 and 1 < 2 and 3 > 2",
                car
            ),
            Some(true)
        );

        // and the collection operations, over what the element owns
        assert_eq!(
            ws.judge("Set{1, 2, 3}->select(n | n > 1)->size() = 2", car),
            Some(true)
        );
        assert_eq!(
            ws.judge(
                "Set{1, 2}->forAll(n | n > 0) and Set{1, 2}->exists(n | n = 2)",
                car
            ),
            Some(true)
        );
        assert_eq!(
            ws.judge(
                "Set{1, 2}->including(3)->excluding(1)->includes(3) and Set{1}->excludes(2)",
                car
            ),
            Some(true)
        );
        assert_eq!(
            ws.judge(
                "Set{1, 2}->union(Set{3})->intersection(Set{2, 3})->asSet()->size() = 2",
                car
            ),
            Some(true)
        );
        assert_eq!(
            ws.judge(
                "Set{1, 2}->isUnique(n | n) and Set{1, 2}->reject(n | n = 1)->first() = 2",
                car
            ),
            Some(true)
        );
        assert_eq!(
            ws.judge(
                "Set{1, 2}->collect(n | n - 1)->last() = 1 and Set{1, 2}->at(2) = 2",
                car
            ),
            Some(true)
        );
        assert_eq!(
            ws.judge(
                "Set{1}->notEmpty() and Set{}->isEmpty() and Set{1, 2}->any(n | n = 2) = 2",
                car
            ),
            Some(true)
        );

        // A library name nothing in this workspace answers to is not a
        // model failing the rule: the library is simply not loaded, and
        // saying `false` would report every model checked without one.
        assert_eq!(
            ws.judge("self.specializesFromLibrary('Parts::Part')", car),
            None
        );
    }

    /// Every corner of the evaluation, reached from OCL a rule could be
    /// written in.
    #[test]
    fn the_corners_of_the_evaluation() {
        let (mut ws, car) = about(
            "package Parts {\n\tpart def Part;\n}\n\
             package Actions {\n\taction def Action;\n}\n\
             part def Car :> Source;\npart def Source;\n",
            "Car",
        );

        // nothing, as a collection and as something to navigate through
        assert_eq!(ws.judge("null->size() = 0", car), Some(true));
        assert_eq!(ws.judge("null.declaredName = null", car), Some(true));
        assert_eq!(ws.judge("null = Set{}", car), Some(true));
        assert_eq!(
            ws.judge("Set{null}.declaredName->isEmpty()", car),
            Some(true)
        );

        // a name bound twice, in a `let` and over a lambda
        assert_eq!(
            ws.judge("let n : Integer = 1 in let n : Integer = 2 in n = 2", car),
            Some(true)
        );
        assert_eq!(
            ws.judge("Set{1}->forAll(n | Set{2}->forAll(n | n = 2))", car),
            Some(true)
        );

        // `or` where the left half does not settle it
        assert_eq!(ws.judge("1 = 2 or 1 = 1", car), Some(true));

        // navigating a collection of elements, and navigating something
        // that is not an element at all
        assert_eq!(
            ws.judge("Set{self, self}.declaredName->size() = 2", car),
            Some(true)
        );
        assert_eq!(
            ws.judge("Set{self}.ownedRelationship->notEmpty()", car),
            Some(true)
        );
        assert_eq!(ws.judge("Set{1}->first().declaredName = null", car), None);

        // the ways an operation is asked for something it cannot give
        assert_eq!(ws.judge("Set{1}->at(0) = 1", car), None);
        assert_eq!(ws.judge("Set{1}->includes()", car), None);
        assert_eq!(ws.judge("Set{1}->forAll(n | n)", car), None);
        assert_eq!(ws.judge("self.oclIsKindOf(NotAMetaclass)", car), None);
        assert_eq!(ws.judge("self.oclIsKindOf(1)", car), None);
        assert_eq!(
            ws.judge("Set{self}->selectByKind(NotAMetaclass)->isEmpty()", car),
            None
        );
        assert_eq!(ws.judge("Set{1}->first().specializes(self)", car), None);
        assert_eq!(
            ws.judge("Set{1}->first().specializesFromLibrary('Parts::Part')", car),
            None
        );

        // `isUnique`, with and without a variable, and both answers
        assert_eq!(ws.judge("Set{1, 2}->isUnique()", car), Some(true));
        assert_eq!(ws.judge("Set{1, 1}->isUnique(n | n)", car), Some(false));
        assert_eq!(ws.judge("Set{1}->isUnique(n | operator)", car), None);

        // and the library names, answered out of packages that are
        // there: what it does specialize, what it does not, and itself
        assert_eq!(
            ws.judge("self.specializesFromLibrary('Parts::Part')", car),
            Some(true)
        );
        assert_eq!(
            ws.judge("self.specializesFromLibrary('Actions::Action')", car),
            Some(false)
        );
        assert_eq!(ws.judge("self.specializes(self)", car), Some(true));
    }

    /// A number the model stores, of the two kinds it stores.
    #[test]
    fn a_number_in_the_model_is_read_as_one() {
        let mut ws = Workspace::new();
        ws.add_file(
            "test.sysml",
            "attribute whole = 1;\nattribute part = 1.5;\n",
        );
        ws.resolve_all();
        let of_kind = |ws: &Workspace, kind: ElementKind| {
            ws.model()
                .ids()
                .find(|&id| ws.model().kind(id) == kind)
                .expect("the literal is built")
        };
        let whole = of_kind(&ws, ElementKind::LiteralInteger);
        let real = of_kind(&ws, ElementKind::LiteralRational);
        assert_eq!(ws.judge("value = 1", whole), Some(true));
        // a real number is not something this compares, and saying so
        // is the honest answer
        assert_eq!(ws.judge("value = 1", real), None);
    }

    /// The answer that is neither true nor false is the point: a model
    /// this toolchain builds is smaller than the abstract syntax the
    /// constraints navigate, and reading what is missing as `false`
    /// reports a violation of a model that never said any such thing.
    #[test]
    fn what_the_model_does_not_build_is_unknown_rather_than_false() {
        let (mut ws, car) = about("part def Car;\n", "Car");

        // a property of the abstract syntax this model does not build
        assert_eq!(ws.judge("operator = '.'", car), None);
        // ownership, which is not the containment this model keeps
        assert_eq!(ws.judge("owningType <> null", car), None);
        assert_eq!(ws.judge("owner <> null", car), None);
        // an operation nothing here implements
        assert_eq!(ws.judge("referencedFeatureTarget() <> null", car), None);
        assert_eq!(ws.judge("self->sortedBy(f | f)->size() = 0", car), None);
        // and anything an unknown reaches
        assert_eq!(ws.judge("not (operator = '.')", car), None);
        assert_eq!(ws.judge("operator = '.' and isAbstract", car), None);
        assert_eq!(
            ws.judge("if operator = '.' then true else false endif", car),
            None
        );
        assert_eq!(ws.judge("Set{1}->forAll(n | operator = '.')", car), None);
        assert_eq!(ws.judge("operator->size() = 0", car), None);
        // a body that is not a condition at all
        assert_eq!(ws.judge("declaredName", car), None);
    }

    /// A control node written at the top of a file is a feature of
    /// nothing, and the specification says a control node is composite.
    /// It is the one constraint of the nine this model can answer that
    /// a model written by hand can break.
    #[test]
    fn a_constraint_the_model_breaks_is_reported_against_the_element() {
        let mut ws = Workspace::new();
        let file = ws.add_file("test.sysml", "action a;\njoin j;\n");
        ws.resolve_all();
        let checked = ws.check_rules(&[file]);

        assert_eq!(checked.violations.len(), 1, "{checked:?}");
        let violation = &checked.violations[0];
        assert_eq!(violation.rule, "validateControlNodeIsComposite");
        assert!(violation.says.contains("composite"), "{violation:?}");
        assert_eq!(ws.qualified_name_of(violation.element), "j");
        // and the constraint is counted as one that was asked, since a
        // rule that answers of one element and not another still ran
        assert!(
            checked.held.contains(&"validateControlNodeIsComposite"),
            "{checked:?}"
        );
    }

    /// Every constraint is asked of every element of its metaclass, and
    /// what cannot be asked is named rather than passed over.
    #[test]
    fn a_check_says_what_it_could_not_evaluate_and_why() {
        let mut ws = Workspace::new();
        let file = ws.add_file("test.sysml", "part def Car { part w : Car; }\n");
        ws.resolve_all();
        let checked = ws.check_rules(&[file]);

        assert!(checked.violations.is_empty(), "{checked:?}");
        // the two the specification's own text defeats are among them,
        // carrying the parser's reason rather than a model's
        let named: Vec<&str> = checked.unevaluated.iter().map(|(name, _)| *name).collect();
        assert!(
            named.contains(&"validateFeatureEndNoDirection"),
            "{named:?}"
        );
        // and every reason says what could not be answered
        assert!(checked.unevaluated.iter().all(|(_, why)| !why.is_empty()));
    }
}

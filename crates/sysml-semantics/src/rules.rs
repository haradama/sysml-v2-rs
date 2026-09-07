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

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use sysml_model::{ElementId, ElementKind, Value};

use crate::ocl::{self, Expr, Op};
use crate::Workspace;

/// How each derived property is worked out, by the metaclass it belongs
/// to and the name it answers to.
///
/// The metamodel states these in OCL beside the constraints, so a
/// derived property is answered by evaluating what the specification
/// says it is rather than by a second account of it written here.
/// Parsed once: there are 235 of them and they are asked for constantly.
fn derivations() -> &'static [(ElementKind, String, Expr)] {
    static PARSED: OnceLock<Vec<(ElementKind, String, Expr)>> = OnceLock::new();
    PARSED.get_or_init(|| {
        let mut out = Vec::new();
        for rule in sysml_model::DERIVATIONS {
            // `property = expression` names what it derives; where the
            // metamodel wrote the expression alone, the rule's own name
            // does -- `deriveInvocationExpressionArgument` after the
            // metaclass is `argument`.
            let (name, body) = match assignment(rule.ocl) {
                Some((name, body)) => (name.to_string(), body),
                None => (named_after(rule), rule.ocl),
            };
            let Ok(expr) = ocl::parse(body) else {
                continue;
            };
            out.push((rule.metaclass, name, expr));
        }
        out
    })
}

/// `property = expression` split in two, where the body is of that
/// shape. A question about the text rather than about what it parses
/// to: the metamodel writes the property first, and `a <= b` has an `=`
/// in it without anything before it being a property.
fn assignment(ocl: &str) -> Option<(&str, &str)> {
    let (head, body) = ocl.split_once('=')?;
    let name = head.trim();
    let named = !name.is_empty() && name.chars().all(|it| it.is_alphanumeric() || it == '_');
    named.then_some((name, body))
}

/// What each operation of the abstract syntax answers, by the metaclass
/// it belongs to and the name it is called by.
///
/// A constraint calls `inputParameters()` as readily as it navigates a
/// property, and the metamodel defines the operation in the same OCL.
/// Parsed once, beside the derivations and for the same reason.
fn operations() -> &'static [Defined] {
    static PARSED: OnceLock<Vec<Defined>> = OnceLock::new();
    PARSED.get_or_init(|| {
        sysml_model::OPERATIONS
            .iter()
            .filter_map(|operation| {
                Some(Defined {
                    of: operation.metaclass,
                    called: operation.name,
                    parameters: operation.parameters,
                    body: ocl::parse(operation.ocl).ok()?,
                })
            })
            .collect()
    })
}

/// One operation of the abstract syntax, read.
struct Defined {
    of: ElementKind,
    called: &'static str,
    parameters: &'static [&'static str],
    body: Expr,
}

/// Flags this toolchain writes wherever they hold, beyond the ones the
/// builder reads off a keyword.
///
/// Name resolution writes every specialization the semantic libraries
/// imply, marking the relationship and the element that gained one. So a
/// relationship carrying neither is one nothing implied, which is what
/// the metamodel declares the default to be.
const WRITTEN_FLAGS: [&str; 2] = ["isImplied", "isImpliedIncluded"];

/// Constraints whose OCL parses and says something other than what the
/// constraint says in words.
///
/// `Specialization::specific` is the more specific of the two types a
/// specialization relates -- the one doing the specializing, which is
/// the element the constraint is being asked of. So
/// `ownedSpecialization.specific->exists(isVariation)` asks whether a
/// variation is a variation, which it is, and the rule reports every
/// well-formed variation in the corpus. What it says in words -- "a
/// variation may not specialize any variation" -- is about `general`.
///
/// Running one of these would report a violation of a model that is
/// sound, so what they are is said instead. The two the OCL subset
/// cannot even parse are pinned in `ocl.rs` alongside.
const MISWRITTEN: [(&str, &str); 2] = [
    (
        "validateDefinitionVariationSpecialization",
        "the specification's own OCL reads `specific` where the constraint says `general`",
    ),
    (
        "validateUsageVariationSpecialization",
        "the specification's own OCL reads `specific` where the constraint says `general`",
    ),
];

/// The property a derivation is about, read off its name where the body
/// does not say: `derive` then the metaclass then the property.
fn named_after(rule: &sysml_model::Rule) -> String {
    let tail = rule
        .name
        .strip_prefix("derive")
        .and_then(|rest| rest.strip_prefix(rule.metaclass.name()))
        .unwrap_or(rule.name);
    let mut chars = tail.chars();
    let first: String = chars
        .by_ref()
        .take(1)
        .flat_map(char::to_lowercase)
        .collect();
    first + chars.as_str()
}

/// How many derivations and operations deep one evaluation may go.
///
/// The specification writes them in terms of one another -- an
/// annotating element's annotated element is its annotation's -- so
/// something has to stop a chain that comes back round to where it
/// started. A bound stops it by construction; watching for the return
/// needs a guard no model can be shown to reach, which is a guard
/// nothing checks.
const DEPTH: usize = 4;

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
            if let Some(defect) = MISWRITTEN.iter().find(|(name, _)| *name == rule.name) {
                parsed.push((rule, None, Some(defect.1.to_string())));
                continue;
            }
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

        // Which metaclasses this model builds at all. A `selectByKind`
        // that keeps nothing says one thing where the model has such
        // elements elsewhere and another where it has none anywhere:
        // the second is this toolchain not building them.
        let present: HashSet<ElementKind> = by_kind.keys().copied().collect();

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
                match self.holds(expr, elem, &present) {
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
    fn holds(
        &mut self,
        expr: &Expr,
        elem: ElementId,
        present: &HashSet<ElementKind>,
    ) -> Result<bool, String> {
        let mut scope = Scope {
            ws: self,
            bound: HashMap::new(),
            self_: elem,
            implicit: None,
            depth: 0,
            present,
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
    /// A membership the model keeps as containment rather than as an
    /// element: the standard owns every member through one, and this
    /// model owns it directly and keeps what the membership said on the
    /// member itself. Put back together here, of the pair it relates,
    /// the way an interchange writer puts it back together.
    Membership {
        owner: ElementId,
        member: ElementId,
    },
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
    /// What an unqualified name is read of where a collection operation
    /// left its variable unwritten. `self` is not taken over by one --
    /// it still means the element the constraint is being asked of --
    /// so the two are kept apart.
    implicit: Option<Val>,
    /// How many derivations and operations deep this already is. They
    /// are written in terms of one another, so a bound is what stops a
    /// chain that comes back round to where it started -- which
    /// terminates by construction rather than by watching for it.
    depth: usize,
    /// The metaclasses this model builds anywhere.
    present: &'a HashSet<ElementKind>,
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
        let target = self.implicit.clone().unwrap_or(Val::Elem(self.self_));
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
            Val::Membership { owner, member } => self.membership_property(*owner, *member, name),
            other => unknown_from(other, &format!("`{name}` of it")),
        }
    }

    /// The metaclass a value stands for, where it stands for one.
    fn kind_of(&self, value: &Val) -> Option<ElementKind> {
        match value {
            Val::Elem(elem) => Some(self.ws.model().kind(*elem)),
            Val::Membership { member, .. } => {
                Some(sysml_model::membership_kind(self.ws.model(), *member))
            }
            _ => None,
        }
    }

    /// One property of a membership the model keeps as containment.
    ///
    /// Only what the specification's own constraints navigate on one is
    /// answered; anything else is said to be unknown rather than guessed
    /// at, the way any property this model does not carry is.
    fn membership_property(&mut self, owner: ElementId, member: ElementId, name: &str) -> Val {
        match name {
            // what it relates, from either side
            "memberElement"
            | "ownedMemberElement"
            | "ownedMemberFeature"
            | "ownedRelatedElement"
            | "relatedElement" => Val::Elem(member),
            "membershipOwningNamespace"
            | "owningRelatedElement"
            | "owningNamespace"
            | "owner"
            | "owningType" => Val::Elem(owner),
            "memberName" => match self.ws.model().name(member) {
                Some(named) => Val::Str(named.to_string()),
                None => Val::Null,
            },
            // `public` unless the source wrote otherwise, which is what
            // the model records on the member
            "visibility" => Val::Str(
                match self.ws.model().member_visibility(member) {
                    Some(sysml_model::Vis::Private) => "private",
                    Some(sysml_model::Vis::Protected) => "protected",
                    _ => "public",
                }
                .to_string(),
            ),
            _ => Val::Unknown(format!(
                "`{name}` of a membership, which this model keeps as the containment it stands for"
            )),
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
            // A relationship that is not itself a feature -- a typing,
            // a subsetting, an import -- is an owned relationship
            // outright. Everything else is a *member*, owned through a
            // membership the model keeps as the containment itself, so
            // the membership is put back together here rather than
            // being absent from an answer the standard says it belongs
            // in. A connector is both a relationship and a feature, and
            // it is the feature half that says how it is owned.
            let owned: Vec<Val> = model
                .owned(elem)
                .iter()
                .filter_map(|&child| {
                    if is_bare_relationship(model.kind(child)) {
                        return model.kind(child).is_a(kind).then_some(Val::Elem(child));
                    }
                    sysml_model::membership_kind(model, child)
                        .is_a(kind)
                        .then_some(Val::Membership {
                            owner: elem,
                            member: child,
                        })
                })
                .collect();
            // Finding none of them is the ambiguous answer where the
            // answer is relationships: the builder reifies some of the
            // ones the abstract syntax has and not others, so an empty
            // answer is as likely to be one it does not build as one the
            // element does not have. A membership is not like that --
            // one stands for each member the element owns, so owning no
            // member of that kind is what an empty answer means.
            if owned.is_empty() && !kind.is_a(ElementKind::Membership) {
                return Val::Unknown(format!(
                    "`{name}` is empty here, and this model does not build every {} \
                     the abstract syntax has",
                    kind.name()
                ));
            }
            return Val::Set(owned);
        }
        // An owning Y is the owner where the owner is a Y, and the
        // membership an element is owned through is the one standing for
        // that containment. A relationship is owned without one.
        if let Some(kind) = owning_kind(name) {
            let Some(owner) = model.owner(elem) else {
                return Val::Null;
            };
            if kind.is_a(ElementKind::Relationship) {
                if is_bare_relationship(model.kind(elem)) {
                    return Val::Null;
                }
                let membership = Val::Membership {
                    owner,
                    member: elem,
                };
                return match self.kind_of(&membership) {
                    Some(actual) if actual.is_a(kind) => membership,
                    _ => Val::Null,
                };
            }
            return match model.kind(owner).is_a(kind) {
                true => Val::Elem(owner),
                false => Val::Null,
            };
        }
        if name == "owner" {
            return match model.owner(elem) {
                Some(owner) => Val::Elem(owner),
                None => Val::Null,
            };
        }
        // What an element was written with, which the model keeps on the
        // element rather than on the membership standing over it --
        // `private import P::*` says the import is private, and the
        // default the standard gives a member is `public`.
        if name == "visibility" && model.kind(elem).feature(name).is_some() {
            // Writing nothing means `public` of a member and `private`
            // of an import: what a namespace declares is visible from
            // outside it, and what it brings in is not passed on.
            let unwritten = match model.kind(elem).is_a(ElementKind::Import) {
                true => sysml_model::Vis::Private,
                false => sysml_model::Vis::Public,
            };
            let visibility = model.member_visibility(elem).unwrap_or(unwritten);
            return Val::Str(visibility.keyword().to_string());
        }
        // A property that redefines another is the one a model holds:
        // `Subsetting::subsettedFeature` redefines
        // `Specialization::general`, and a constraint written of the
        // general one is asking about the same thing under the name the
        // metaclass it is being asked of gives it.
        let held = model.get(elem, name).or_else(|| {
            redefining(model.kind(elem), name).and_then(|under| model.get(elem, under))
        });
        match held {
            Some(Value::Bool(it)) => Val::Bool(*it),
            Some(Value::String(it)) => Val::Str(it.clone()),
            Some(Value::EnumLit(it)) => Val::Str(it.to_string()),
            Some(Value::Int(it)) => Val::Int(*it),
            Some(Value::Real(_)) => Val::Unknown(format!("`{name}` holds a real number")),
            Some(Value::Ref(it)) => Val::Elem(*it),
            Some(Value::RefList(them)) => Val::Set(them.iter().map(|&it| Val::Elem(it)).collect()),
            // A derived property is never stored -- the metamodel says
            // so -- and what answers for it is the specification's own
            // account of how it is worked out.
            // A flag the builder reads off the source for every
            // metaclass that has it: nothing written is the model
            // saying what the specification declares the default to be,
            // and the metamodel states that beside the property.
            None if (sysml_model::BUILT_FLAGS.contains(&name) || WRITTEN_FLAGS.contains(&name))
                && model
                    .kind(elem)
                    .feature(name)
                    .is_some_and(|meta| meta.default.is_some()) =>
            {
                Val::Bool(
                    model
                        .kind(elem)
                        .feature(name)
                        .and_then(|meta| meta.default)
                        .expect("the arm this matched"),
                )
            }
            None => match self.derive(elem, name) {
                Some(value) => value,
                // A property the metaclass does not declare at all is
                // not one this model fails to build. Some are the
                // specification's own text asking for something that is
                // not there -- `connectorEnds` where the metamodel
                // declares `connectorEnd` -- and some are navigations
                // the metamodel writes as an end owned by an
                // association rather than as an attribute of the class,
                // which is not among what the metaclasses declare.
                // Either way the fault is not here.
                None if self.ws.model().kind(elem).feature(name).is_none() => {
                    Val::Unknown(format!(
                        "the metamodel declares no `{name}` on `{}`",
                        self.ws.model().kind(elem).name()
                    ))
                }
                // A property with nothing under it is not an empty one.
                // Whether the builder would have filled it in is not
                // something the absence can say -- `relatedFeature` is
                // kept for some metaclasses and not others -- and
                // reading it as empty is how a checker comes to report a
                // violation of a model that never said anything of the
                // sort.
                None => Val::Unknown(format!(
                    "`{name}` is part of the abstract syntax that this model does not build here"
                )),
            },
        }
    }

    /// A derived property, worked out the way the specification says.
    ///
    /// The derivation is evaluated of the element itself, in a scope of
    /// its own: what it says is about that element, not about whatever
    /// lambda the navigation happened to be inside.
    fn derive(&mut self, elem: ElementId, name: &str) -> Option<Val> {
        let kind = self.ws.model().kind(elem);
        // A derivation that reads another goes one level deeper. Past
        // four the answer has stopped improving, and the bound is what
        // keeps two properties derived from each other from going round
        // for ever.
        let (_, _, body) = derivations()
            .iter()
            .find(|(about, property, _)| property == name && kind.is_a(*about))
            .filter(|_| self.depth < DEPTH)?;
        let mut scope = Scope {
            ws: self.ws,
            bound: HashMap::new(),
            self_: elem,
            implicit: None,
            depth: self.depth + 1,
            present: self.present,
        };
        Some(scope.eval(body))
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
            None => self.implicit.clone().unwrap_or(Val::Elem(self.self_)),
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
                    // `->isUnique()` with nothing written at all says
                    // no two of the elements agree; anything written is
                    // read of each of them
                    let value = match (lambda, args) {
                        (None, []) => item,
                        _ => self.over(&item, args, lambda),
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
                let kept: Vec<Val> = items
                    .into_iter()
                    .filter(|item| self.kind_of(item).is_some_and(|it| it.is_a(kind)))
                    .collect();
                // Keeping none of them is the ambiguous answer where the
                // model has no element of that kind anywhere: an
                // implicit conjugated port definition is not absent from
                // one port, it is absent from this toolchain.
                // A membership is put back together from the
                // containment rather than built, so it is in no such
                // tally: keeping none of them means there are none.
                if kept.is_empty()
                    && !kind.is_a(ElementKind::Membership)
                    && !self.present.iter().any(|it| it.is_a(kind))
                {
                    return Val::Unknown(format!(
                        "no `{}` is built anywhere in this model",
                        kind.name()
                    ));
                }
                Val::Set(kept)
            }
            "closure" => {
                // OCL's transitive closure: the body read of each
                // element, then of everything that comes back, until
                // nothing new turns up. What it started from is in the
                // answer only where the walk reaches it again, and the
                // ones already found are what stops it going round.
                let mut found: Vec<Val> = Vec::new();
                let mut queue = items;
                while let Some(item) = queue.pop() {
                    let value = self.over(&item, args, lambda);
                    if let Some(unknown) = value.unknown() {
                        return unknown;
                    }
                    for next in value.many() {
                        if !found.iter().any(|it| equal(it, &next) == Val::Bool(true)) {
                            found.push(next.clone());
                            queue.push(next);
                        }
                    }
                }
                Val::Set(found)
            }
            "forAll" | "exists" | "select" | "reject" | "collect" | "any" => {
                let mut kept = Vec::new();
                for item in items {
                    let value = self.over(&item, args, lambda);
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
    fn over(&mut self, item: &Val, args: &[Expr], lambda: Option<&(String, Box<Expr>)>) -> Val {
        let Some((bound, body)) = lambda else {
            // `->exists(not oclIsKindOf(OwningMembership))` names no
            // variable: OCL reads the body of each element in turn, and
            // that is what an unqualified name in it stands for.
            let [body] = args else {
                return Val::Unknown(
                    "a collection operation written with more than one body".to_string(),
                );
            };
            let outer = self.implicit.replace(item.clone());
            let result = self.eval(body);
            self.implicit = outer;
            return result;
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
            // `oclIsType` is how the specification spells the exact-type
            // question OCL calls `oclIsTypeOf`
            "oclIsKindOf" | "oclIsTypeOf" | "oclIsType" => {
                let (Some(kind), Some(actual)) =
                    (args.first().and_then(metaclass_named), self.kind_of(target))
                else {
                    return Val::Unknown(format!("`{name}` of a kind this does not know"));
                };
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
            // What a type specializes. The specification writes
            // `Feature::supertypes` in terms of `Type::supertypes`
            // through an `oclAsType`, which an operation looked up by
            // the metaclass of its target cannot tell apart from the
            // call it is written inside. This workspace works the same
            // question out for inherited-member lookup, so that is the
            // answer -- with what the standard implies included, which
            // is what every rule asking for them asks for.
            "supertypes" => match (target, self.argument(args)) {
                (Val::Elem(elem), Val::Bool(false)) => Val::Set(
                    self.ws
                        .supertypes(*elem)
                        .into_iter()
                        .map(Val::Elem)
                        .collect(),
                ),
                (_, other) => unknown_from(
                    &other,
                    "`supertypes` of something that is not a type, or excluding what the \
                     standard implies",
                ),
            },
            // The abstract syntax defines its own operations in OCL
            // beside its constraints, so what one answers is what the
            // specification says it answers.
            _ => self
                .invoke(target, name, args)
                .unwrap_or_else(|| Val::Unknown(format!("`{name}()` is not implemented"))),
        }
    }

    /// One of the abstract syntax's own operations, worked out the way
    /// the specification defines it.
    fn invoke(&mut self, target: &Val, name: &str, args: &[Expr]) -> Option<Val> {
        let Val::Elem(elem) = target else {
            // every operation the specification defines is of a
            // metaclass, so one of a number or a string is none of them
            return None;
        };
        let kind = self.ws.model().kind(*elem);
        let defined = operations()
            .iter()
            .find(|it| it.called == name && it.parameters.len() == args.len() && kind.is_a(it.of))
            .filter(|_| self.depth < DEPTH)?;
        let bound: HashMap<String, Val> = defined
            .parameters
            .iter()
            .map(|it| it.to_string())
            .zip(args.iter().map(|arg| self.eval(arg)))
            .collect();
        let mut scope = Scope {
            ws: self.ws,
            bound,
            self_: *elem,
            implicit: None,
            depth: self.depth + 1,
            present: self.present,
        };
        Some(scope.eval(&defined.body))
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

/// The property of `kind` that redefines `name`, where one does.
///
/// A model holds the redefining name -- a `Subclassification` says
/// `superclassifier`, not `general` -- so a constraint written of the
/// property it redefines has to be told where to look. Redefinition
/// chains, so the walk keeps going until it runs out.
fn redefining(kind: ElementKind, name: &str) -> Option<&'static str> {
    let of_kind = || {
        std::iter::once(kind)
            .chain(kind.ancestors().iter().copied())
            .flat_map(|it| it.own_features())
    };
    let mut found = None;
    let mut wanted = name;
    // Redefinition chains -- `referencedFeature` redefines
    // `subsettedFeature`, which redefines `general` -- so the walk keeps
    // going, and the deepest name is the one the model holds.
    while let Some(next) = of_kind().find(|meta| meta.redefines == Some(wanted)) {
        wanted = next.name;
        found = Some(next.name);
    }
    found
}

/// Whether an element is owned as a relationship rather than as a
/// member.
///
/// A typing, a subsetting or an import is a relationship and nothing
/// else, and its owner owns it directly. A connector is a relationship
/// too, but it is also a feature, and a feature of a type is owned
/// through a membership like any other member.
fn is_bare_relationship(kind: ElementKind) -> bool {
    kind.is_a(ElementKind::Relationship) && !kind.is_a(ElementKind::Feature)
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
        // `import P::*;` is owned by the namespace it brings the names
        // into, which is the containment like any other
        "importOwningNamespace" => ElementKind::Namespace,
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
        let present: HashSet<ElementKind> =
            self.model().ids().map(|it| self.model().kind(it)).collect();
        self.holds(&expr, elem, &present).ok()
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

        // a collection operation that leaves its variable unwritten is
        // read of each element in turn -- and says so rather than
        // guessing where the elements are not elements at all
        assert_eq!(
            ws.judge("Sequence{1, 2}->exists(oclIsKindOf(Feature))", car),
            None
        );
        // the transitive closure of a body, which stops when nothing
        // new turns up rather than going round for ever
        assert_eq!(
            ws.judge("Set{1}->closure(n | Set{})->isEmpty()", car),
            Some(true)
        );
        assert_eq!(
            ws.judge(
                "Set{1}->closure(n | if n = 1 then Set{2} else Set{} endif)->size() = 1",
                car
            ),
            Some(true)
        );
        // and one that keeps coming back to where it started stops
        // there rather than going round for ever
        assert_eq!(
            ws.judge("Set{1}->closure(n | Set{1})->size() = 1", car),
            Some(true)
        );
        // a walk it cannot take a step of does not answer half of one
        assert_eq!(
            ws.judge("Set{self}->closure(t | t.operator)->isEmpty()", car),
            None
        );
        // the exact-type question, which the specification spells
        // `oclIsType` and OCL spells `oclIsTypeOf`
        assert_eq!(ws.judge("oclIsType(PartDefinition)", car), Some(true));
        assert_eq!(ws.judge("oclIsType(Definition)", car), Some(false));
        // an iterator reads one body, and a collection operation
        // written with more than one is not OCL this reads
        assert_eq!(ws.judge("Set{1}->exists(1 = 1, 2 = 2)", car), None);
        // and what a type specializes is worked out here with the
        // standard's implied supertypes in it, which is what every rule
        // asking for them asks for -- so leaving them out is not
        // something this can answer
        assert_eq!(ws.judge("supertypes(true)->isEmpty()", car), None);
        assert_eq!(ws.judge("supertypes(false)->notEmpty()", car), Some(true));
        // a metaclass is a question about something that stands for one,
        // and a number stands for none -- so selecting by kind keeps it
        // out rather than reading it as one
        assert_eq!(
            ws.judge("Set{1}->selectByKind(Package)->isEmpty()", car),
            Some(true)
        );
        // and `self` is not taken over by one, however deep: it still
        // means the element the constraint is being asked of
        assert_eq!(
            ws.judge(
                "Set{self}->exists(Set{self}->exists(self.declaredName = 'Car'))",
                car
            ),
            Some(true)
        );

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
        // ownership is the containment this model keeps, so it answers:
        // a definition written at the top of a file is owned by the root
        // namespace, which is no type
        assert_eq!(ws.judge("owner <> null", car), Some(true));
        assert_eq!(ws.judge("owningType <> null", car), Some(false));
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

    /// A derived property is one the metamodel declares is never
    /// stored, so a model that held it would hold it twice. What
    /// answers for it is the specification's own account of how it is
    /// worked out, evaluated the same way a constraint is.
    #[test]
    fn a_derived_property_is_worked_out_the_way_the_specification_says() {
        let (mut ws, a) = about("part def Car {\n\tattribute a;\n\tpart w;\n}\n", "a");
        // `deriveUsageIsReference: isReference = not isComposite`, and
        // an attribute is referential
        assert_eq!(ws.judge("isReference", a), Some(true));

        let (mut ws, w) = about("part def Car {\n\tattribute a;\n\tpart w;\n}\n", "w");
        assert_eq!(ws.judge("isReference", w), Some(false));

        // A derivation reaching for what the model does not build is
        // answered by neither, and one reaching for what it does is
        // answered outright. `ownedMember` is read off the memberships,
        // and a membership stands for each member the containment
        // holds -- so a definition that owns nothing owns no member,
        // and there is nothing an empty answer could be hiding. An
        // owned specialization is not like that: the builder reifies
        // some of them and not others.
        let (mut ws, car) = about("part def Car;\n", "Car");
        assert_eq!(ws.judge("ownedMember->isEmpty()", car), Some(true));
        assert_eq!(ws.judge("ownedSpecialization->isEmpty()", car), None);
    }

    /// A `selectByKind` that keeps nothing says one thing where the
    /// model has such elements elsewhere and another where it has none
    /// anywhere: the second is this toolchain not building them, and
    /// answering "none, so the rule is satisfied" is a false assurance.
    #[test]
    fn keeping_none_of_a_kind_nothing_builds_is_unknown() {
        let (mut ws, car) = about("part def Car {\n\tpart w;\n}\n", "Car");
        // nothing here builds a conjugation, so "at most one of them"
        // is not something this can vouch for
        assert_eq!(
            ws.judge(
                "ownedRelationship->selectByKind(Conjugation)->size() <= 1",
                car
            ),
            None
        );
        // where the kind is built, the selection is the answer
        let (mut ws, w) = about("part def Car {\n\tpart v;\n\tpart w :> v;\n}\n", "w");
        assert_eq!(
            ws.judge("ownedRelationship->selectByKind(Subsetting)->size() = 1", w),
            Some(true)
        );
    }

    /// A flag the builder reads off the source for every metaclass that
    /// has it is one a model can be silent about, and silence there is
    /// the specification's own default rather than something this
    /// cannot answer.
    #[test]
    fn a_flag_the_source_did_not_write_is_the_default_the_specification_states() {
        let (mut ws, car) = about("part def Car;\n", "Car");
        // written nowhere, and the metamodel says what that means
        assert_eq!(ws.judge("isAbstract", car), Some(false));
        assert_eq!(ws.judge("isVariation", car), Some(false));
        // `isUnique` is the one that holds unless the source says
        // otherwise, and the answer comes from the metamodel either way
        let (mut ws, w) = about("part def Car {\n\tpart w;\n}\n", "w");
        assert_eq!(ws.judge("isUnique", w), Some(true));
        let (mut ws, w) = about("part def Car {\n\tpart w [*] nonunique;\n}\n", "w");
        assert_eq!(ws.judge("isUnique", w), Some(false));

        // and a property the builder does not read is still unknown
        assert_eq!(ws.judge("operator = \'.\'", w), None);
    }

    /// A constraint calls an operation of the abstract syntax as readily
    /// as it navigates a property, and the metamodel defines what each
    /// one answers in the same OCL.
    #[test]
    fn an_operation_answers_what_the_specification_says_it_answers() {
        let (mut ws, car) = about("part def Car;\n", "Car");
        // `Type::isCompatibleWith(other) = specializes(other)`, and
        // everything specializes itself
        assert_eq!(ws.judge("self.isCompatibleWith(self)", car), Some(true));
        // one nothing defines is still unknown
        assert_eq!(ws.judge("self.noSuchOperation() = 1", car), None);
        // and so is one asked of something that is not an element:
        // every operation the specification defines is of a metaclass
        assert_eq!(ws.judge("self.isAbstract.noSuchOperation()", car), None);
    }

    /// `assign v := 1;` refers to `v` without owning it, and the
    /// specification asks for exactly that: `An AssignmentActionUsage
    /// must have an ownedMembership that is not an OwningMembership and
    /// whose memberElement is a Feature.` Asking it needs the implicit
    /// iterator as well, since the constraint binds no variable to read
    /// the memberships of.
    #[test]
    fn what_an_assignment_refers_to_is_there_to_be_asked_for() {
        let mut ws = Workspace::new();
        let file = ws.add_file(
            "test.sysml",
            "action def A {\n\tattribute v;\n\taction x;\n\tthen assign v := 1;\n}\n",
        );
        ws.resolve_all();
        let checked = ws.check_rules(&[file]);

        assert!(checked.violations.is_empty(), "{checked:?}");
        assert!(
            checked
                .held
                .contains(&"validateAssignmentActionUsageReferent"),
            "{checked:?}"
        );
    }

    /// The standard owns every member through a `Membership`; this
    /// model owns it directly and keeps what the membership said on the
    /// member itself. Put back together the two are the same thing --
    /// which is what lets a constraint navigate `ownedMembership` at
    /// all, and half of them do.
    #[test]
    fn a_membership_the_model_keeps_as_containment_answers_like_one() {
        let (mut ws, car) = about(
            "part def W;\npart def Car {\n\tprivate part wheel : W;\n}\n",
            "Car",
        );
        let one = |what: &str| format!("ownedMembership->at(1).{what}");

        // the member it relates and the namespace it is owned by
        assert_eq!(ws.judge("ownedMembership->size() = 1", car), Some(true));
        assert_eq!(
            ws.judge(&one("memberElement.declaredName = 'wheel'"), car),
            Some(true)
        );
        assert_eq!(
            ws.judge(&one("ownedMemberFeature.declaredName = 'wheel'"), car),
            Some(true)
        );
        assert_eq!(
            ws.judge(&one("membershipOwningNamespace = self"), car),
            Some(true)
        );
        // and what the source wrote of the membership itself, which the
        // model keeps on the member
        assert_eq!(ws.judge(&one("memberName = 'wheel'"), car), Some(true));
        assert_eq!(
            ws.judge(&one("visibility = VisibilityKind::private"), car),
            Some(true)
        );
        // a feature of a type sits behind a `FeatureMembership`, so it
        // is one of those the type owns and not merely a membership
        assert_eq!(
            ws.judge("ownedFeatureMembership->size() = 1", car),
            Some(true)
        );
        // anything else about it is unknown rather than guessed at
        assert_eq!(ws.judge(&one("isImplied"), car), None);

        // Read from the member, the same membership stands for the same
        // containment. A typing is a relationship and nothing else, so
        // its owner owns it without one.
        let (mut ws, wheel) = about(
            "part def W;\npart def Car {\n\tprivate part wheel : W;\n}\n",
            "wheel",
        );
        assert_eq!(
            ws.judge("owningMembership.memberElement = self", wheel),
            Some(true)
        );
        assert_eq!(
            ws.judge("owningFeatureMembership <> null", wheel),
            Some(true)
        );
        assert_eq!(
            ws.judge("owningType.declaredName = 'Car'", wheel),
            Some(true)
        );
        let typing = ws
            .model()
            .ids()
            .find(|&id| ws.model().kind(id) == ElementKind::FeatureTyping)
            .expect("the typing is reified");
        assert_eq!(ws.judge("owningMembership = null", typing), Some(true));
        // a package member is owned plainly, so it is behind no feature
        // membership at all
        let (mut ws, package) = about("package P {\n\tpart def Q;\n}\n", "Q");
        assert_eq!(
            ws.judge("owningFeatureMembership = null", package),
            Some(true)
        );
        // and the root namespace is owned by nothing
        let root = ws.root();
        assert_eq!(ws.judge("owningMembership = null", root), Some(true));
        assert_eq!(ws.judge("owner = null", root), Some(true));

        // The visibility is the member's own and `public` where the
        // source wrote none, and a member with no name gives the
        // membership none either.
        let (mut ws, hub) = about(
            "part def W;\npart def Hub {\n\tprotected part guard : W;\n\
             \tpart plain : W;\n\tpart : W;\n}\n",
            "Hub",
        );
        let at = |n: usize, what: &str| format!("ownedMembership->at({n}).{what}");
        assert_eq!(
            ws.judge(&at(1, "visibility = VisibilityKind::protected"), hub),
            Some(true)
        );
        assert_eq!(
            ws.judge(&at(2, "visibility = VisibilityKind::public"), hub),
            Some(true)
        );
        assert_eq!(ws.judge(&at(3, "memberName = null"), hub), Some(true));

        // An alias is a membership the model does build, and it answers
        // the same way. Writing nothing means `public` of a member and
        // `private` of an import: what a namespace declares is visible
        // from outside it, and what it brings in is not passed on.
        let (mut ws, alias) = about("package P;\nalias Q for P;\n", "Q");
        assert_eq!(
            ws.judge("visibility = VisibilityKind::public", alias),
            Some(true)
        );
        let (mut ws, brought) = about(
            "package P {\n\tpart def X;\n}\npackage R {\n\
                                       \timport P::*;\n}\n",
            "R",
        );
        let import = ws
            .model()
            .owned(brought)
            .iter()
            .copied()
            .find(|&child| ws.model().kind(child).is_a(ElementKind::Import))
            .expect("the import is built");
        assert_eq!(
            ws.judge("visibility = VisibilityKind::private", import),
            Some(true)
        );
    }

    /// A model holds the redefining name -- a `Subclassification` says
    /// `superclassifier`, not `general` -- and a constraint may be
    /// written of the property it redefines. They are the same thing
    /// under two names, and half the constraints about specialization
    /// ask under the older one.
    #[test]
    fn a_property_that_redefines_another_answers_under_both_names() {
        let (mut ws, _) = about("part def A;\npart def B :> A;\n", "B");
        let model = ws.model();
        let subclassification = model
            .ids()
            .find(|&id| model.kind(id) == ElementKind::Subclassification)
            .expect("the specialization is reified");
        // the name the model holds, and the one it redefines
        assert_eq!(
            ws.judge("superclassifier.declaredName = 'A'", subclassification),
            Some(true)
        );
        assert_eq!(
            ws.judge("general.declaredName = 'A'", subclassification),
            Some(true)
        );
        assert_eq!(
            ws.judge("specific.declaredName = 'B'", subclassification),
            Some(true)
        );
        // Nothing implied this one: the source wrote it, and name
        // resolution marks every specialization it implies, so an
        // unmarked one is not implied rather than unknown.
        assert_eq!(ws.judge("isImplied", subclassification), Some(false));
        // A name the metamodel declares nowhere is not a property this
        // model fails to build, and saying so puts the fault where it
        // belongs -- `connectorEnds` is the specification's own OCL
        // asking for something that is not there.
        assert_eq!(
            ws.judge("connectorEnds->isEmpty()", subclassification),
            None
        );
    }

    /// A control node written at the top of a file is wrong in two ways
    /// the specification names: a control node is composite, and what
    /// owns one is an action. Both are asked of every element, and both
    /// are reported against the node itself.
    #[test]
    fn a_constraint_the_model_breaks_is_reported_against_the_element() {
        let mut ws = Workspace::new();
        let file = ws.add_file("test.sysml", "action a;\njoin j;\n");
        ws.resolve_all();
        let checked = ws.check_rules(&[file]);

        let broken: Vec<&str> = checked.violations.iter().map(|it| it.rule).collect();
        assert_eq!(
            broken,
            [
                "validateControlNodeOwningType",
                "validateControlNodeIsComposite"
            ],
            "{checked:?}"
        );
        assert!(
            checked
                .violations
                .iter()
                .all(|violation| { ws.qualified_name_of(violation.element) == "j" }),
            "{checked:?}"
        );
        assert!(
            checked.violations[1].says.contains("composite"),
            "{checked:?}"
        );
        // and each is counted as one that was asked, since a rule that
        // answers of one element and not another still ran
        assert!(
            broken.iter().all(|rule| checked.held.contains(rule)),
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

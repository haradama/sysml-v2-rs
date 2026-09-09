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

use sysml_model::{ElementId, ElementKind, Role, Value};

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
            let ocl = closed(rule.name, rule.ocl);
            let (name, body) = match assignment(&ocl) {
                Some((name, body)) => (name.to_string(), body),
                None => (named_after(rule), ocl.as_ref()),
            };
            if let Ok(expr) = ocl::parse(body) {
                out.push((rule.metaclass, name, expr));
            }
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
                    body: ocl::parse(&closed(operation.name, operation.ocl)).ok()?,
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

/// The names a usage gives to what it is typed by.
///
/// `Usage::definition` and the narrower names that stand beside it, one
/// per kind of usage. `ItemUsage::itemDefinition` is left out: the
/// metamodel states that one in OCL, and what it says is not quite this
/// -- the structures among the occurrence definitions.
const TYPED_BY: [&str; 5] = [
    "actionDefinition",
    "definition",
    "occurrenceDefinition",
    "partDefinition",
    "portDefinition",
];

/// The names a membership metaclass gives to the one thing it owns.
///
/// `SubjectMembership::ownedSubjectParameter`,
/// `StateSubactionMembership::action`,
/// `ElementFilterMembership::condition` and their kin: each is the
/// member, under whichever name the metaclass declares. Their
/// `referenced` counterparts -- `referencedConcern`,
/// `verifiedRequirement` -- name something the membership does not own,
/// and are left alone.
/// `ParameterMembership::ownedMemberParameter` is left out. Answered,
/// it reaches four constraints -- three about what an expression comes
/// to, the result of a feature reference and the type of a multiplicity
/// bound among them -- and this model keeps an expression as the text
/// it was written as rather than as the parameters the specification
/// counts. Read as the member, `result` is a definite nothing rather
/// than an unanswered question, and 2847 of a sound corpus are reported
/// as violations: 1704 of `validateFeatureReferenceExpressionResult`,
/// 1125 of `validateMultiplicityRangeBoundResultTypes` and 18 of
/// `validateElementFilterMembershipConditionIsBoolean`. The fourth,
/// `validateParameterMembershipParameterDirection`, holds.
const OWNED_MEMBER: [&str; 14] = [
    "action",
    "condition",
    "ownedMemberParameter",
    "ownedActorParameter",
    "ownedConcern",
    "ownedConstraint",
    "ownedObjectiveRequirement",
    "ownedRendering",
    "ownedRequirement",
    "ownedResultExpression",
    "ownedStakeholderParameter",
    "ownedSubjectParameter",
    "ownedVariantUsage",
    "transitionFeature",
];

/// Flags this toolchain writes wherever they hold, beyond the ones the
/// builder reads off a keyword.
///
/// Name resolution writes every specialization the semantic libraries
/// imply, marking the relationship and the element that gained one. So a
/// relationship carrying neither is one nothing implied, which is what
/// the metamodel declares the default to be.
const WRITTEN_FLAGS: [&str; 2] = ["isImplied", "isImpliedIncluded"];

/// Where the specification's own OCL names something it does not
/// declare, and what it plainly means.
///
/// `Type::inheritableMemberships` takes `excludedTypes` and its body
/// reads `excludedType`. There is no other reading: no metaclass has a
/// property of that name, the operation has no other parameter it could
/// be, and the two are one letter apart. Twenty constraints reach the
/// memberships a type inherits through that operation, so the choice is
/// between reading what it means and answering none of them.
///
/// `validateAssertConstraintUsageReference` calls
/// `referencedFeaureTarget()`, and the metamodel declares no operation
/// of that name. Its two siblings -- the same constraint about a
/// requirement and about a state -- are written out in the same file
/// with `referencedFeatureTarget()`, and so is the second call in this
/// one. Read as written the guard cannot be answered, and the rule
/// stops on `oclIsKindOf` of the null it was guarding against.
///
/// This is not the same as the two below. Those say something other than
/// what the constraint says in words, and running them would report a
/// violation of a model that is sound; the corpus is what says whether
/// reading these as they are meant is right.
/// `ControlNode::multiplicityHasBounds` calls `oclisKindOf`, which is
/// spelled `oclIsKindOf` everywhere else in the metamodel and twice in
/// the very body that misspells it once.
///
/// Each of these is read as meant where the body calls it, which is the
/// only place a name that belongs to nothing can appear.
/// `deriveFeatureType` calls `exist(`, and OCL spells the operation
/// `exists`; the same body writes `reject` and `closure` correctly, and
/// nothing anywhere declares an `exist`. It is the derivation every
/// constraint about the types of a feature reads.
///
/// `deriveMetadataFeatureMetaclass` binds `metaclassTypes` and reads
/// `metaClassTypes` back on the next line but one, and
/// `validateRedefinitionFeaturingTypes` binds `redefinedFeaturingTypes`
/// and reads `redefinedFeaturingType` back on the line after. Nothing
/// binds the name either of them reads, the name each bound is read
/// nowhere, and both are written correctly beside the slip -- the
/// second reads `redefiningFeaturingTypes` in the very comparison that
/// misspells its other half. `InstantiationExpression::instantiatedType`
/// is a third of the same shape: it binds `members` and reads
/// `typeMembers` two lines later, and nothing binds that.
const MISSPELLED: [(&str, &str); 7] = [
    ("excludedType", "excludedTypes"),
    ("referencedFeaureTarget", "referencedFeatureTarget"),
    ("oclisKindOf", "oclIsKindOf"),
    ("exist", "exists"),
    ("metaClassTypes", "metaclassTypes"),
    ("redefinedFeaturingType", "redefinedFeaturingTypes"),
    ("typeMembers", "members"),
];

/// Where the specification's own OCL does not close what it opens, and
/// the one place the closing can go.
///
/// `deriveFeatureCrossFeature` writes two `if`s and one `endif`:
///
/// ```text
/// crossFeature =
///     if ownedCrossSubsetting = null then null
///     else
///         let chainingFeatures : Sequence(Feature) =
///             ownedCrossSubsetting.crossedFeature.chainingFeature in
///         if chainingFeatures->size() < 2 then null
///         else chainingFeatures->at(2)
///     endif
/// ```
///
/// A `let` runs to the end of what follows it, so the inner `if` is the
/// whole of the outer one's `else` and the single `endif` written can
/// only close the inner. The outer is left open, and there is no other
/// point in the text where inserting an `endif` makes it parse: the
/// grammar leaves one position, not a choice of them.
///
/// `validateFeatureEndNoDirection` is written `isEnd implied direction
/// = null`, where `implied` is no OCL operator at all. Choosing one
/// would be choosing among readings, which is not what closing an open
/// bracket is -- so it was left unread until something other than this
/// parser said which. The pilot implementation says: `checkFeature`
/// writes `if (f.isEnd && f.direction !== null) error(...)`, which is
/// `implies` and nothing else.
///
/// Nor is it the same as [`MISWRITTEN`] below, where the OCL parses and
/// says something other than the constraint's own words.
///
/// Closing what is open only helps where what the body then goes on to
/// evaluate can be answered. `Type::multiplicities` is left out for
/// that reason: closed, it parses, and
/// `validateFeatureEndMultiplicity` then reports twelve hundred
/// violations of a sound corpus, because the `allSuperTypes()` and
/// `hasBounds(1, 1)` it goes on to call answer nothing and an
/// `exists` over nothing is a definite `false`. Unreadable is the
/// better answer there.
const UNCLOSED: [(&str, &str, &str); 5] = [
    (
        "validateFeatureEndNoDirection",
        "isEnd implied",
        "isEnd implies",
    ),
    (
        "deriveFeatureCrossFeature",
        "chainingFeatures->at(2)",
        "chainingFeatures->at(2) endif",
    ),
    // `deriveTransitionUsageSource` opens an `if` and closes none, and
    // the text ends there.
    (
        "deriveTransitionUsageSource",
        "oclAsType(ActionUsage)",
        "oclAsType(ActionUsage) endif",
    ),
    // `ControlNode::multiplicityHasBounds` opens an `exists(` and
    // closes it nowhere; the `endif` that ends the `if` it sits in is
    // the one place the `)` can go before.
    (
        "multiplicityHasBounds",
        "hasBounds(lower, upper)\nendif",
        "hasBounds(lower, upper))\nendif",
    ),
    // `Expression::modelLevelEvaluable` opens two `forAll(` and closes
    // one, and stops in the middle of the second.
    (
        "modelLevelEvaluable",
        "f.oclAsType(Expression).modelLevelEvaluable(visited)",
        "f.oclAsType(Expression).modelLevelEvaluable(visited))",
    ),
];

/// The specification's OCL for one of its rules, with what it leaves
/// open closed.
fn closed(name: &str, ocl: &'static str) -> std::borrow::Cow<'static, str> {
    match UNCLOSED.iter().find(|(rule, _, _)| *rule == name) {
        Some((_, written, meant)) => ocl.replace(written, meant).into(),
        None => ocl.into(),
    }
}

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
/// `validateMergeNodeIncomingSuccessions` and
/// `validateDecisionNodeOutgoingSuccessions` hand a connector *end* to
/// `multiplicityHasBounds`, whose parameter is a `Multiplicity`, and
/// bind it as `sourceMult` and `targetMult`. Read as written both are
/// false of every merge and decision node there is. Their two siblings
/// in the same file -- `validateControlNodeIncomingSuccessions` and
/// `validateControlNodeOutgoingSuccessions` -- are written the same way
/// down to the line breaks and say `connectorEnd->at(2).multiplicity`,
/// so what was left out is not in doubt.
///
/// Reading it in does not help, because the four cannot all hold. The
/// specification is explicit that these multiplicities are enforced "in
/// the abstract syntax, even if not shown explicitly in the concrete
/// syntax notation", and `ActionTest.sysml` writes `then decide; if
/// true then m;` with `m` a merge node. That one succession must have
/// its target end 0..1 (out of a decision) and 1..1 (into a control
/// node), and its source end 0..1 (into a merge) and 1..1 (out of a
/// control node). The corpus is what says which pair to keep: the
/// ControlNode two hold of every succession in it.
///
/// `validateSubsettingFeaturingTypes` is `subsettingFeature.canAccess(
/// subsettedFeature)`, and `canAccess` holds the subsetted feature to
/// being featured within one of the featuring types the subsetting one
/// reaches -- walking *up* from it, never down. Read as the metamodel
/// states it, 1234 subsettings of the corpus are rejected.
///
/// The pilot implementation does run this constraint, and answers one
/// step of it more widely than the metamodel states: `TypeUtil.
/// isCompatible` also holds two features compatible where neither owns
/// features of its own, they redefine something in common, and the one
/// is featured where the other is. Answered that way here, 1231 are
/// still rejected -- so the wider step is not what the corpus turns on.
/// What it turns on is the direction: `accept a : A` gives a transition
/// an `accepted` featured by the transition and a payload featured by
/// the trigger the transition owns, and no walk upwards from the one
/// reaches the other.
///
/// `validateRedefinitionFeaturingTypes` says in words that the
/// redefining feature "must have at least one featuringType that is not
/// also a featuringType of the redefinedFeature", and in OCL that the
/// two sets are unequal, which is not the same thing. Neither holds of
/// `FeatureChains.kerml`, where `redefinition b.f redefines b.a;`
/// redefines one feature of `B` by another: both are featured by `B`
/// alone, so the sets are equal and there is no featuring type the one
/// has and the other has not. The pilot implementation reads it the
/// same way -- `checkRedefinition` errors where the two sets are equal
/// -- and the one guard it adds beside that, for a redefinition owning
/// the feature it redefines, is not this.
///
/// Running one of these would report a violation of a model that is
/// sound, so what they are is said instead. The two the OCL subset
/// cannot even parse are pinned in `ocl.rs` alongside.
/// `validateMultiplicityRangeBoundResultTypes` holds every bound to
/// coming to a `ScalarValues::Integer`, and `[nCauses]` counts with an
/// attribute the notation gives no type -- `attribute nCauses =
/// size(causes);`. The pilot implementation does not run the OCL at
/// all: `checkMultiplicityRange` carries "TODO: Correct
/// validateMultiplicityBoundResults OCL from KERML-199" and asks
/// instead whether the bound evaluates to an integer.
///
/// `validateElementFilterMembershipConditionIsBoolean` holds a filter's
/// condition to coming to a boolean, and the specification's own
/// operator table maps `|` to `DataFunctions::'|'`, which returns a
/// `DataValue`. The pilot implementation says so in as many words --
/// "Non-conditional 'Boolean' operations in DataFunctions actually have
/// result DataValue. This infers that they are actually BooleanFunctions
/// if their arguments are Boolean" -- and infers what the specification
/// does not state.
const MISWRITTEN: [(&str, &str); 8] = [
    (
        "validateMultiplicityRangeBoundResultTypes",
        "a bound written as a name comes to whatever that name is typed by, and the notation \
         types none of them; the pilot implementation does not run this OCL either, against \
         KERML-199",
    ),
    (
        "validateElementFilterMembershipConditionIsBoolean",
        "the specification's own operator table maps `|` to `DataFunctions::'|'`, which returns \
         a data value rather than a boolean; the pilot implementation infers the boolean the \
         specification does not state",
    ),
    (
        "validateSubsettingFeaturingTypes",
        "`canAccess` walks up from the subsetting feature and the corpus features what it \
         subsets further down, so 1234 subsettings are rejected -- and 1231 still are with \
         the wider compatibility the pilot implementation answers with",
    ),
    (
        "validateRedefinitionFeaturingTypes",
        "the OCL asks for two sets of featuring types to differ where the constraint asks for \
         one the redefining feature has and the redefined one has not, and neither holds of a \
         redefinition between two features of the same type",
    ),
    (
        "validateDefinitionVariationSpecialization",
        "the specification's own OCL reads `specific` where the constraint says `general`",
    ),
    (
        "validateUsageVariationSpecialization",
        "the specification's own OCL reads `specific` where the constraint says `general`",
    ),
    (
        "validateMergeNodeIncomingSuccessions",
        "the OCL hands a connector end to `multiplicityHasBounds`, which takes a multiplicity, \
         and reading in the `.multiplicity` its siblings write contradicts \
         `validateControlNodeIncomingSuccessions` on the corpus's own decision-to-merge \
         succession",
    ),
    (
        "validateDecisionNodeOutgoingSuccessions",
        "the OCL hands a connector end to `multiplicityHasBounds`, which takes a multiplicity, \
         and reading in the `.multiplicity` its siblings write contradicts \
         `validateControlNodeOutgoingSuccessions` on the corpus's own decision-to-merge \
         succession",
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
///
/// Six is what this can answer for. Past it the walk reaches
/// `Type::directionOfExcluding`, which climbs the supertypes of every
/// feature of every type, and the constraints it opens up are ones the
/// model cannot yet meet: at eight one, at ten a hundred, at twelve
/// four hundred and sixty of the corpus are reported as violations --
/// an `accept` node's payload and receiver, a `send` node's three
/// parameters, a requirement's subject. Those are the model missing
/// what the specification counts, not the bound being too low, and
/// raising it turns a truthful "cannot say" into a false finding.
///
/// The bound also only buys work. The memberships a type inherits are
/// worked out through five operations that call one another over every
/// supertype, and no depth completes them: at twelve they were two
/// thirds of every operation the check invoked, and a third of the
/// time it took, all of it spent arriving at the same "cannot say".
const DEPTH: usize = 6;

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
            let defect = MISWRITTEN.iter().find(|(name, _)| *name == rule.name);
            // Every constraint the specification states can be read
            // once `closed` has repaired the one it writes with a word
            // that is no operator, and
            // `every_constraint_the_specification_states_parses_but_its_own_one_defect`
            // holds it so. What is left unread is what is left unrun,
            // which is what `MISWRITTEN` names.
            let expr = defect.is_none().then(|| {
                ocl::parse(&closed(rule.name, rule.ocl)).expect("every constraint can be read")
            });
            parsed.push((rule, expr, defect.map(|(_, why)| why.to_string())));
        }
        // Grouped by metaclass once. Asked element by element, every
        // rule walks the whole model to find the few it is about, and
        // the whole model is where this is meant to be run.
        // Every element under those files, not only the ones a syntax
        // node stands for. Name resolution reifies the relationships the
        // notation leaves implicit -- a typing, a subsetting, the ends of
        // a connector -- and the constraints are about those as much as
        // about what the source wrote. Asking only what was written left
        // every rule about a `Subsetting` asked of nothing, with two
        // thousand of them in the model.
        let mut under: Vec<ElementId> = Vec::new();
        let mut seen: HashSet<ElementId> = HashSet::new();
        for &file in files {
            for &elem in self.file_elements(file) {
                let mut stack = vec![elem];
                while let Some(id) = stack.pop() {
                    if !seen.insert(id) {
                        continue;
                    }
                    under.push(id);
                    stack.extend(self.model().owned(id).iter().copied());
                }
            }
        }
        // in the order the model holds them, so a report reads down the
        // file rather than in the order the walk came upon them
        under.sort_unstable();
        // Each paired with the element a violation of it names: a
        // membership is not an element of this model, so what is
        // reported is the member it holds, which is what the source
        // wrote and what a reader would go and look at.
        let mut by_kind: HashMap<ElementKind, Vec<(ElementId, Val)>> = HashMap::new();
        for elem in under {
            by_kind
                .entry(self.model().kind(elem))
                .or_default()
                .push((elem, Val::Elem(elem)));
            // Every member of a namespace is held under a membership,
            // and this model keeps the containment the membership
            // stands for rather than the membership itself. Twenty-two
            // constraints are about one -- what a `SubjectMembership`
            // may own, which way a `ParameterMembership` passes it --
            // and asked only of elements they were asked of nothing.
            // Put back together, they are asked of what the model does
            // hold, the way an interchange writer puts them back.
            // A relationship is owned outright rather than through a
            // membership, and the root is owned by nothing.
            let held = self
                .model()
                .owner(elem)
                .filter(|_| !is_bare_relationship(self.model().kind(elem)));
            if let Some(owner) = held {
                by_kind
                    .entry(sysml_model::membership_kind(self.model(), elem))
                    .or_default()
                    .push((
                        elem,
                        Val::Membership {
                            owner,
                            member: elem,
                        },
                    ));
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
            let about: Vec<(ElementId, Val)> = by_kind
                .iter()
                .filter(|(kind, _)| kind.is_a(rule.metaclass))
                .flat_map(|(_, them)| them.iter().cloned())
                .collect();
            let mut asked = false;
            let mut unknown = None;
            for (elem, it) in about {
                match self.holds(expr, &it, &present) {
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
        about: &Val,
        present: &HashSet<ElementKind>,
    ) -> Result<bool, String> {
        let mut scope = Scope {
            ws: self,
            bound: HashMap::new(),
            self_: about.clone(),
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
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Val {
    /// Nothing here can answer: a property the abstract syntax has and
    /// this model does not, or an operation not implemented. Carries
    /// what could not be answered, for the report.
    Unknown(String),
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    /// `*`, the unbounded end of a multiplicity: equal to itself and to
    /// no number, which is what the bounds of a multiplicity are
    /// compared for.
    Unlimited,
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
    self_: Val,
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
            Expr::Unlimited => Val::Unlimited,
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
        // A name the body binds under one spelling and reads back under
        // another. Only where what it meant is bound: a property that
        // happens to be spelled like a slip still answers as itself.
        if let Some(value) = self.bound.get(meant(name)) {
            return value.clone();
        }
        if name == "self" {
            return self.self_.clone();
        }
        let target = self.implicit.clone().unwrap_or_else(|| self.self_.clone());
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
            (Op::Add, Val::Int(a), Val::Int(b)) => Val::Int(a + b),
            // `qualifiedName + '::' + escapedName()` -- the one thing
            // the specification adds strings for is building a name
            (Op::Add, Val::Str(a), Val::Str(b)) => Val::Str(format!("{a}{b}")),
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
            // what it relates, from either side.
            // `validateRedefinitionFeaturingTypes` reads `modelElement`
            // off what `resolveGlobal` answers with; no metaclass
            // declares such a property, and every other caller of
            // `resolveGlobal` in the metamodel writes `memberElement`.
            "memberElement"
            | "modelElement"
            | "ownedMemberElement"
            | "ownedMemberFeature"
            | "ownedRelatedElement"
            | "relatedElement" => Val::Elem(member),
            // Each membership metaclass names what it owns under a name
            // of its own -- `SubjectMembership::ownedSubjectParameter`,
            // `VariantMembership::ownedVariantUsage` -- and every one of
            // them is the member. The metaclass this membership stands
            // for is what says which of the names it answers to: the
            // `referenced` half of the pairs beside them names something
            // the membership does not own, and is not this.
            name if OWNED_MEMBER.contains(&name)
                && sysml_model::membership_kind(self.ws.model(), member)
                    .feature(name)
                    .is_some() =>
            {
                Val::Elem(member)
            }
            "membershipOwningNamespace"
            | "owningRelatedElement"
            | "owningNamespace"
            | "owner"
            | "owningType" => Val::Elem(owner),
            // `RequirementKind : RequirementConstraintMembership =
            // 'assume' { kind = 'assumption' } | 'require' { kind =
            // 'requirement' }`, and a `verify` and a `frame` are
            // requirements too. A transition's is which of its three
            // parts the member is, which the model works out from the
            // property the transition holds it under.
            "kind" => {
                let model = self.ws.model();
                match sysml_model::membership_kind(model, member) {
                    ElementKind::TransitionFeatureMembership => Val::Str(
                        sysml_model::transition_role(model, member)
                            .expect("it is one of the three because the transition holds it")
                            .to_string(),
                    ),
                    kind if kind.is_a(ElementKind::RequirementConstraintMembership) => {
                        match model.member_role(member) {
                            Some(Role::Assume) => Val::Str("assumption".to_string()),
                            _ => Val::Str("requirement".to_string()),
                        }
                    }
                    kind => Val::Unknown(format!(
                        "`kind` of a `{}`, which the notation writes nowhere",
                        kind.name()
                    )),
                }
            }
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

    /// Fill in [`REVERSE_ENDS`] for the whole model, once.
    ///
    /// Read one at a time each of these is a scan of every element, and
    /// the derivation of a feature's types walks them over every type
    /// it reaches -- which is the difference between the check taking
    /// seconds and taking a quarter of a minute.
    fn index_reverse_ends(&mut self) {
        if self.ws.reverse.0 == self.ws.model().len() {
            return;
        }
        let model = self.ws.model();
        let mut index: HashMap<(&'static str, ElementId), Vec<ElementId>> = HashMap::new();
        for it in model.ids() {
            let kind = model.kind(it);
            for &(end, relationship, forward) in &REVERSE_ENDS {
                if !kind.is_a(relationship) {
                    continue;
                }
                let held = model
                    .get(it, forward)
                    .or_else(|| redefining(kind, forward).and_then(|under| model.get(it, under)));
                if let Some(Value::Ref(of)) = held {
                    index.entry((end, *of)).or_default().push(it);
                }
            }
        }
        self.ws.reverse = (model.len(), index);
    }

    /// What features `elem`: what a `featured by` writes, else the type
    /// that owns it as a feature -- and where nothing does either, what
    /// features the feature it is written inside.
    ///
    /// That last step is what tells a multiplicity apart from a feature.
    /// A type owns its multiplicity through an `OwningMembership` and
    /// not a `FeatureMembership`, so a multiplicity has no `owningType`
    /// and is featured wherever the feature carrying it is -- which is
    /// what `validateFeatureMultiplicityDomain` asks for ("the
    /// featuringTypes of the multiplicity must be the same as those of
    /// the Feature itself") and what
    /// `validateClassifierMultiplicityDomain` asks for from the other
    /// side, a classifier's multiplicity having none at all.
    ///
    /// A chain is featured where its first step is, which is the one
    /// part of the unreadable derivation that is written plainly.
    ///
    /// Both walks end: the first climbs the ownership tree, and the
    /// second reads a `chainingFeature`, which only a connector end
    /// carries and whose steps are the features the source named.
    fn collect_featuring_types(&mut self, elem: ElementId, into: &mut Vec<ElementId>) {
        let model = self.ws.model();
        let written: Vec<ElementId> = model
            .owned(elem)
            .iter()
            .copied()
            .filter(|&it| model.kind(it).is_a(ElementKind::TypeFeaturing))
            .flat_map(|it| model.featuring_type(it))
            .copied()
            .collect();
        let owner = model.owner(elem);
        let owns_it_as_a_feature = owner.is_some_and(|_| {
            sysml_model::membership_kind(model, elem).is_a(ElementKind::FeatureMembership)
        });
        let inside = owner.filter(|&it| model.kind(it).is_a(ElementKind::Feature));
        let first_step = match model.get(elem, "chainingFeature") {
            Some(Value::RefList(chain)) => chain.first().copied(),
            _ => None,
        };
        match (written.is_empty(), owns_it_as_a_feature, inside) {
            (false, _, _) => into.extend(written),
            (true, true, _) => into.extend(owner),
            (true, false, Some(inside)) => self.collect_featuring_types(inside, into),
            (true, false, None) => {}
        }
        if let Some(step) = first_step {
            self.collect_featuring_types(step, into);
        }
    }

    /// The name an element answers to from the root namespace, or null
    /// where it has none: an unnamed element, or one inside one.
    fn qualified_name(&self, elem: ElementId) -> Val {
        let model = self.ws.model();
        let mut segments = Vec::new();
        let mut at = Some(elem);
        while let Some(it) = at.filter(|&it| it != self.ws.root) {
            let Some(named) = model.name(it) else {
                return Val::Null;
            };
            segments.push(named);
            at = model.owner(it);
        }
        segments.reverse();
        match segments.is_empty() {
            // the root namespace is what every other name is read from
            // and answers to no name of its own
            true => Val::Null,
            false => Val::Str(segments.join("::")),
        }
    }

    /// The element a qualified name names, read from the root.
    fn global(&mut self, qualified: &str) -> Option<ElementId> {
        self.ws.named_globally(qualified)
    }

    /// One property of one element, or [`Val::Unknown`] where this model
    /// does not carry it.
    fn property(&mut self, elem: ElementId, name: &str) -> Val {
        let model = self.ws.model();
        let name = written_as_meant(model.kind(elem), name);
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
                    let member = Val::Membership {
                        owner: elem,
                        member: child,
                    };
                    // Every membership is a relationship, so asked for
                    // relationships at large the answer is every child
                    // and which membership each stands for need not be
                    // worked out. It is the most asked-for property
                    // there is, and working it out is the walk that
                    // `membership_kind` does.
                    if kind == ElementKind::Relationship {
                        return Some(member);
                    }
                    sysml_model::membership_kind(model, child)
                        .is_a(kind)
                        .then_some(member)
                })
                .collect();
            // Finding none of them is the ambiguous answer only where
            // the answer is relationships at large: the builder reifies
            // some of the ones the abstract syntax has and not others,
            // so an empty answer there is as likely to be one it does
            // not build as one the element does not have. Asked for a
            // kind of relationship it does build -- a membership, an
            // import, a specialization -- owning none of them is what
            // an empty answer means.
            if owned.is_empty() && kind == ElementKind::Relationship {
                return Val::Unknown(format!(
                    "`{name}` is empty here, and this model does not build every {} \
                     the abstract syntax has",
                    kind.name()
                ));
            }
            // What the metamodel declares single-valued answers with
            // the value rather than with a collection of one:
            // `ownedPortConjugator` is `[0..1]`, and
            // `ownedPortConjugator.originalPortDefinition =
            // originalPortDefinition` compares one against one.
            if model
                .kind(elem)
                .feature(name)
                .is_some_and(|meta| !meta.many)
            {
                return owned.into_iter().next().unwrap_or(Val::Null);
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
        // `Usage::definition` -- "the Definitions that are types of this
        // Usage" -- and the narrower names beside it: an occurrence
        // usage's `occurrenceDefinition`, a part usage's
        // `partDefinition`. The metamodel states each in prose and
        // states none of them in OCL, so nothing the evaluation reads
        // can work them out; each is the usage's types of the kind its
        // own metaclass declares for it, whether the source wrote the
        // type or the standard implied it.
        let typed_by = TYPED_BY
            .contains(&name)
            .then(|| model.kind(elem).feature(name).map(|meta| meta.ty))
            .flatten();
        if let Some(sysml_model::FeatureType::Class(of)) = typed_by {
            return self.typed_by(elem, of);
        }
        // A conjugated port definition is declared inside the port it
        // is the conjugate of, which is how the metamodel states its
        // `originalPortDefinition`: "the `owningNamespace` of the
        // `ConjugatedPortDefinition`". A port conjugation states a
        // property of the same name meaning the other end of itself,
        // and the model holds that one, so the two are told apart by
        // what is being asked rather than by the name.
        if name == "originalPortDefinition"
            && model.kind(elem).is_a(ElementKind::ConjugatedPortDefinition)
        {
            return model.owner(elem).map_or(Val::Null, Val::Elem);
        }
        // The other side of that: an annotation this model builds is
        // owned by the annotating element -- the comment, the metadata
        // usage -- and never by what it annotates.
        if name == "owningAnnotatedElement" && model.kind(elem).is_a(ElementKind::Annotation) {
            return Val::Null;
        }
        // The one derived property of a Type the metamodel states in
        // prose alone: it "indicates whether this Type has an
        // ownedConjugator", and a conjugator is a Conjugation the type
        // owns. `class B conjugates A;` writes one; the statement form
        // gives it to the namespace instead, and by the standard's own
        // account that leaves the type it names unconjugated.
        if name == "isConjugated" && model.kind(elem).is_a(ElementKind::Type) {
            return Val::Bool(
                model
                    .owned(elem)
                    .iter()
                    .any(|&child| model.kind(child).is_a(ElementKind::Conjugation)),
            );
        }
        // The connectors that relate this feature, from either end.
        // The metamodel writes both as ends owned by an association, so
        // no metaclass declares them, and the model keeps a connector's
        // related features rather than a feature's connectors -- so the
        // answer is found by looking the other way about.
        if matches!(name, "sourceConnector" | "targetConnector")
            && model.kind(elem).is_a(ElementKind::Feature)
        {
            // the first related feature is what a connector goes from,
            // and the rest are what it goes to
            let from = name == "sourceConnector";
            return Val::Set(
                model
                    .ids()
                    .filter(|&it| model.kind(it).is_a(ElementKind::Connector))
                    .filter(|&it| {
                        let mut related = model.related_feature(it).iter();
                        match from {
                            true => related.next() == Some(&elem),
                            false => related.skip(1).any(|&at| at == elem),
                        }
                    })
                    .map(Val::Elem)
                    .collect(),
            );
        }
        // A property no metaclass declares, because the association
        // that has it owns the end: `Feature::typing` is "the
        // FeatureTypings for which a certain Feature is the
        // typedFeature". The model keeps the relationship, so the
        // answer is found by looking the other way about, the way
        // `sourceConnector` is above.
        if let Some(&(end, ..)) = REVERSE_ENDS.iter().find(|(end, ..)| *end == name) {
            self.index_reverse_ends();
            let found = self.ws.reverse.1.get(&(end, elem));
            return Val::Set(
                found
                    .map(|them| them.iter().copied().map(Val::Elem).collect())
                    .unwrap_or_default(),
            );
        }
        // "The Types that feature this Feature". The metamodel derives
        // it from a `featuring` that no metaclass declares, so the
        // derivation cannot be read -- but the metamodel says what it
        // comes to all the same, in `Feature::isFeaturingType`: "if not
        // isVariable then type = owningType".
        if name == "featuringType" && model.kind(elem).is_a(ElementKind::Feature) {
            let mut them = Vec::new();
            self.collect_featuring_types(elem, &mut them);
            return Val::Set(them.into_iter().map(Val::Elem).collect());
        }
        // `FlowDefinition::flowEnd` is declared derived and derived
        // nowhere. KerML states the same property of a `Flow` --
        // "the connectorEnds of this Flow that are FlowEnds" -- and a
        // definition is an association rather than a connector, so
        // what stands for its connector ends is the ends it owns.
        if name == "flowEnd" && model.kind(elem).is_a(ElementKind::FlowDefinition) {
            return Val::Set(
                model
                    .owned(elem)
                    .iter()
                    .copied()
                    .filter(|&it| model.kind(it).is_a(ElementKind::FlowEnd))
                    .map(Val::Elem)
                    .collect(),
            );
        }
        // Every membership in a namespace: the containments it keeps
        // and what its imports bring in. The metamodel derives it as a
        // union of the two, and the resolver works out what an import
        // brings in for name lookup already.
        if name == "membership" && model.kind(elem).is_a(ElementKind::Namespace) {
            let mut all: Vec<Val> = model
                .owned(elem)
                .iter()
                // an import is a relationship and not a membership, so
                // it is not one of them however many it brings in
                .filter(|&&member| !is_bare_relationship(model.kind(member)))
                .map(|&member| Val::Membership {
                    owner: elem,
                    member,
                })
                .collect();
            for member in self.ws.imported_members(elem) {
                all.push(Val::Membership {
                    owner: elem,
                    member,
                });
            }
            return Val::Set(once_each(all));
        }
        // The memberships a type inherits. The metamodel works this
        // out through five operations that call one another over every
        // supertype -- `removeRedefinedFeatures(inheritableMemberships(
        // ...))` -- and no depth of evaluation completes them: two
        // thirds of every operation the check invoked went on that
        // chain, to arrive at "cannot say". The resolver walks the same
        // specializations to find a name, so that walk is the answer,
        // and what it finds is what the standard describes.
        if name == "inheritedMembership" && model.kind(elem).is_a(ElementKind::Type) {
            return Val::Set(self.inherited(elem));
        }
        if name == "owner" {
            return match model.owner(elem) {
                Some(owner) => Val::Elem(owner),
                None => Val::Null,
            };
        }
        // `owningNamespace.qualifiedName + '::' + escapedName()`, one
        // namespace at a time, through an `escapedName()` the metamodel
        // writes with `indexOf`. The resolver spells the same name to
        // resolve one, so that is the answer -- and null where anything
        // along the way has no name, which is where the derivation's own
        // `null` would have propagated from.
        if name == "qualifiedName" {
            return self.qualified_name(elem);
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
            // `direction` is the one property of a feature the builder
            // reads off every declaration that can carry one, and the
            // notation writes it only where it holds: `in`, `out` and
            // `inout` are all there is to write, and a feature written
            // without one has none. The metamodel declares it optional
            // and states no default, so nothing written is the model
            // saying it is null rather than the model being silent.
            None if name == "direction" && model.kind(elem).feature(name).is_some() => Val::Null,
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

    /// Every membership a type inherits: those of everything it
    /// specializes, and of everything those specialize in turn, less
    /// what is private to them and less what a redefinition has
    /// replaced.
    ///
    /// A supertype's imports are inherited with its own memberships --
    /// `membershipsOfVisibility` unions the two -- and the resolver
    /// works out what an import brings in for name lookup already.
    fn inherited(&mut self, elem: ElementId) -> Vec<Val> {
        let mut queue = self.ws.supertypes(elem);
        let mut seen = vec![elem];
        let mut at = 0;
        let mut memberships: Vec<(ElementId, ElementId)> = Vec::new();
        while at < queue.len() {
            let up = queue[at];
            at += 1;
            if seen.contains(&up) {
                continue;
            }
            seen.push(up);
            for member in self.ws.model().owned(up).to_vec() {
                // what a type keeps to itself is not inherited
                if self.ws.model().member_visibility(member) == Some(sysml_model::Vis::Private) {
                    continue;
                }
                memberships.push((up, member));
            }
            for member in self.ws.imported_members(up) {
                memberships.push((up, member));
            }
            queue.extend(self.ws.supertypes(up));
        }
        // A feature that redefines another stands in its place, so what
        // it replaced is not inherited beside it -- whether the
        // redefining feature is one of these or one the type declares
        // itself.
        let mut replaced = HashSet::new();
        let mine = self.ws.model().owned(elem).to_vec();
        for &feature in memberships
            .iter()
            .map(|(_, member)| member)
            .chain(mine.iter())
        {
            self.collect_redefined(feature, &mut replaced);
        }
        // A member declared with a role the standard allows one of
        // stands in the place of the one that would be inherited: a
        // requirement's own subject replaces the subject it inherits,
        // as a function's own result parameter replaces the one of the
        // function it specializes. The standard says so with an implied
        // redefinition, which is written into a model only where the
        // implied relationships are materialised.
        let mut taken: Vec<Role> = mine
            .iter()
            .filter_map(|&it| role_of_one(self.ws, it))
            .collect();
        memberships.retain(|(_, member)| {
            if replaced.contains(member) {
                return false;
            }
            match role_of_one(self.ws, *member) {
                // nearest first, so the first of a role is the one that
                // stands and the rest are the ones it stands for
                Some(role) if taken.contains(&role) => false,
                Some(role) => {
                    taken.push(role);
                    true
                }
                None => true,
            }
        });
        memberships
            .into_iter()
            .map(|(owner, member)| Val::Membership { owner, member })
            .collect()
    }

    /// Everything a feature redefines, directly or through what it
    /// redefines in turn.
    fn collect_redefined(&self, feature: ElementId, into: &mut HashSet<ElementId>) {
        // `redefinedFeature` is Redefinition's alone, so what holds one
        // is a redefinition and what does not is not
        for owned in self.ws.model().owned(feature) {
            let target = match self.ws.model().get(*owned, "redefinedFeature") {
                Some(Value::Ref(target)) => *target,
                _ => continue,
            };
            if into.insert(target) {
                self.collect_redefined(target, into);
            }
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
        let mut wanted = name;
        let found = loop {
            let hit = derivations()
                .iter()
                .find(|(about, property, _)| property == wanted && kind.is_a(*about));
            match (hit, redefined_property(kind, wanted)) {
                (Some(hit), _) => break Some(hit),
                (None, Some(up)) => wanted = up,
                (None, None) => break None,
            }
        };
        let (_, _, body) = found.filter(|_| self.depth < DEPTH)?;
        let mut scope = Scope {
            ws: self.ws,
            bound: HashMap::new(),
            self_: Val::Elem(elem),
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
            None => self.implicit.clone().unwrap_or_else(|| self.self_.clone()),
        };
        // A name read from the root namespace does not depend on where
        // it is read from, and the metamodel writes `resolveGlobal` of
        // things that are not there to read it from: `Feature::canAccess`
        // writes it of `subsettingFeature`, which is a property of a
        // Subsetting and not of the Feature the operation is about.
        if name != "resolveGlobal" {
            if let Some(unknown) = target.unknown() {
                return unknown;
            }
        }
        // Read as meant before it is dispatched, so that both the
        // operations written here and those the metamodel defines are
        // found under the name the specification's own body calls them.
        let name = meant(name);
        // a collection operation reads its target as many things; an
        // operation on an element reads it as one
        if arrow || COLLECTION.contains(&name) {
            return self.collection(&target, name, args, lambda);
        }
        // `supertypes(excludeImplied)->reject(...).nonPrivateMemberships(...)`
        // -- an operation written of many things is written of each of
        // them, the way a property read of many is read of each.
        if let Val::Set(items) = &target {
            let mut out = Vec::new();
            for item in items.clone() {
                match self.operation(&item, name, args) {
                    Val::Set(more) => out.extend(more),
                    one => out.push(one),
                }
            }
            return Val::Set(out);
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
            // A `Set` and an `OrderedSet` hold each value once; a `Bag`
            // and a `Sequence` hold what they were given.
            "asSet" | "asOrderedSet" => {
                Val::Set(once_each(items.iter().flat_map(Val::many).collect()))
            }
            "asBag" | "asSequence" | "flatten" => {
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
            // `relatedFeature->subSequence(2, relatedFeature->size())`
            // is how a connector says every end but the first. Counted
            // from one and taking both ends, as OCL does.
            "subSequence" => {
                let [first, last] = args else {
                    return Val::Unknown(
                        "`subSequence` written without the two ends it takes".to_string(),
                    );
                };
                let (first, last) = (self.eval(first), self.eval(last));
                let (from, to) = match (&first, &last) {
                    (Val::Int(from), Val::Int(to)) => (*from, *to),
                    (other, Val::Int(_)) | (_, other) => {
                        return unknown_from(other, "an end of `subSequence`")
                    }
                };
                // one past the end is the empty sequence, which is what
                // `subSequence(2, 1)` of a single related feature says
                if from < 1 || to < from - 1 {
                    return Val::Unknown(format!(
                        "`subSequence({from}, {to})`, which is no part of a sequence"
                    ));
                }
                Val::Set(
                    items
                        .iter()
                        .take(to as usize)
                        .skip(from as usize - 1)
                        .cloned()
                        .collect(),
                )
            }
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
                Val::Set(once_each(out))
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
                // The transitive closure: the body read of each
                // element, then of everything that comes back, until
                // nothing new turns up, with the ones already found
                // what stops it going round.
                //
                // What it started from is in the answer too.
                // `allRedefinedFeatures()` is written
                // `ownedRedefinition.redefinedFeature->
                // closure(ownedRedefinition.redefinedFeature)`, and read
                // without them it says nothing a feature redefines
                // directly -- which is all that nearly every feature
                // redefines, and the operation is then a no-op that
                // takes `removeRedefinedFeatures` down with it.
                let mut found: Vec<Val> = items.clone();
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
            // `oclIsType` is the metamodel's own spelling, used in three
            // constraints and nowhere defined; `oclIsTypeOf`, the exact
            // question, it never writes at all. What it means is the
            // kind question: `validateObjectiveMembershipOwningType`
            // asks that the owning type "be a CaseDefinition or
            // CaseUsage", and every objective in the corpus is owned by
            // an `AnalysisCaseDefinition`, a `UseCaseDefinition` or a
            // `VerificationCaseDefinition` -- twenty-nine of them, none
            // of which is exactly a `CaseDefinition`.
            "oclIsKindOf" | "oclIsTypeOf" | "oclIsType" => {
                let (Some(kind), Some(actual)) =
                    (args.first().and_then(metaclass_named), self.kind_of(target))
                else {
                    return Val::Unknown(format!("`{name}` of a kind this does not know"));
                };
                Val::Bool(match name {
                    "oclIsTypeOf" => actual == kind,
                    _ => actual.is_a(kind),
                })
            }
            // the metamodel casts to read a property, and reading a
            // property does not need the cast -- it spells the cast
            // both ways
            "oclAsType" | "oclAsKindOf" => target.clone(),
            // What the model calls this element's metaclass. The
            // reflective libraries hold one per class of the abstract
            // syntax, and every constraint that asks for a type asks
            // for it so as to name it and look it up again -- so the
            // answer is that declaration, found the way the pilot
            // implementation finds it.
            "oclType" => {
                let Some(kind) = self.kind_of(target) else {
                    return Val::Unknown(
                        "the metaclass of something that is not an element".to_string(),
                    );
                };
                METACLASS_PACKAGES
                    .iter()
                    .find_map(|package| self.global(&format!("{package}::{}", kind.name())))
                    .map_or_else(
                        || {
                            Val::Unknown(format!(
                                "the library declares no metaclass `{}`",
                                kind.name()
                            ))
                        },
                        Val::Elem,
                    )
            }
            // A name read from the root namespace, which answers with
            // the membership the namespace holds it under.
            "resolveGlobal" => match self.argument(args) {
                Val::Str(qualified) => match self.global(&qualified) {
                    Some(found) => Val::Membership {
                        // a name resolves inside a namespace; the root
                        // is the one thing held under no membership,
                        // and it answers to no name to be found by
                        owner: self.ws.model().owner(found).unwrap_or(self.ws.root),
                        member: found,
                    },
                    // not in this workspace, which is the answer OCL
                    // gives for a name that resolves to nothing
                    None => Val::Null,
                },
                other => unknown_from(&other, "a name to resolve that is not a string"),
            },
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
                match self.global(&qualified) {
                    Some(up) => Val::Bool(self.specializes(*elem, up)),
                    // the library is not loaded, which is not the model
                    // failing a rule
                    None => Val::Unknown(format!("`{qualified}` is not in this workspace")),
                }
            }
            // Which way a feature is passed, as seen from a type.
            // `Type::directionOf(feature)` and its
            // `directionOfExcluding`, and `Feature::directionFor(type)`
            // from the other side. The specification defines it by
            // recursion over every supertype of every feature of every
            // type, which the bound on derivation depth stops short of
            // -- and it stands between every constraint that counts
            // what a behaviour is handed and an answer.
            "directionOf" | "directionOfExcluding" => match (target, self.argument(args)) {
                (Val::Elem(of), Val::Elem(feature)) => self.direction_of(*of, feature),
                (_, other) => {
                    unknown_from(&other, "the direction of something that is not a feature")
                }
            },
            "directionFor" => match (target, self.argument(args)) {
                (Val::Elem(feature), Val::Elem(of)) => self.direction_of(of, *feature),
                (_, other) => unknown_from(
                    &other,
                    "the direction seen from something that is not a type",
                ),
            },
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
                .unwrap_or_else(|| match self.kind_of(target) {
                    // an operation the metamodel states of no metaclass
                    // this target is
                    Some(_) => Val::Unknown(format!("`{name}()` is not implemented")),
                    // and one asked of a property this model leaves
                    // empty, which is a different thing to say
                    None => unknown_from(
                        target,
                        &format!("`{name}()` of something that is not an element"),
                    ),
                }),
        }
    }

    /// One of the abstract syntax's own operations, worked out the way
    /// the specification defines it.
    fn invoke(&mut self, target: &Val, name: &str, args: &[Expr]) -> Option<Val> {
        // Every operation the specification defines is of a metaclass,
        // so one of a number or a string is none of them -- and one of a
        // membership is of the metaclass this model keeps as the
        // containment standing for it, which is what
        // `ParameterMembership::parameterDirection()` is asked of.
        let kind = self.kind_of(target)?;
        // The most specific metaclass answers:
        // `ReturnParameterMembership::parameterDirection()` returns
        // `out` and redefines the `in` a `ParameterMembership` returns,
        // and it is one -- so the general one would answer for both.
        let defined = operations()
            .iter()
            .filter(|it| it.called == name && it.parameters.len() == args.len() && kind.is_a(it.of))
            .max_by_key(|it| it.of.ancestors().len())
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
            self_: target.clone(),
            implicit: None,
            depth: self.depth + 1,
            present: self.present,
        };
        Some(scope.eval(&defined.body))
    }

    /// What a usage is typed by, of a kind.
    ///
    /// `individual timeslice t3 :> ind;` is typed by what `ind` is
    /// typed by, so the walk carries on past a feature rather than
    /// stopping at it.
    fn typed_by(&mut self, elem: ElementId, of: ElementKind) -> Val {
        let mut queue = self.ws.supertypes(elem);
        let mut seen = vec![elem];
        let mut types = Vec::new();
        let mut at = 0;
        while at < queue.len() {
            let up = queue[at];
            at += 1;
            if seen.contains(&up) {
                continue;
            }
            seen.push(up);
            if self.ws.model().kind(up).is_a(of) {
                types.push(Val::Elem(up));
                continue;
            }
            queue.extend(self.ws.supertypes(up));
        }
        Val::Set(types)
    }

    /// Which way a feature is passed, as seen from a type.
    ///
    /// The direction a type gives a feature is the one the feature was
    /// declared with where the type owns it, and otherwise the first
    /// one its supertypes give -- reversed at each conjugation, since
    /// conjugating a type turns what it takes in into what it puts out.
    fn direction_of(&mut self, of: ElementId, feature: ElementId) -> Val {
        let mut seen = Vec::new();
        match self.direction_seen(of, feature, &mut seen) {
            Some(direction) => Val::Str(direction.to_string()),
            None => Val::Null,
        }
    }

    fn direction_seen(
        &mut self,
        of: ElementId,
        feature: ElementId,
        seen: &mut Vec<ElementId>,
    ) -> Option<&'static str> {
        if seen.contains(&of) {
            return None;
        }
        seen.push(of);
        if self.ws.model().owner(feature) == Some(of) {
            return match self.ws.model().get(feature, "direction") {
                Some(Value::EnumLit(it)) => Some(it),
                _ => None,
            };
        }
        let direction = self
            .ws
            .supertypes(of)
            .into_iter()
            .find_map(|up| self.direction_seen(up, feature, seen))?;
        let conjugated = self
            .ws
            .model()
            .owned(of)
            .iter()
            .any(|&child| self.ws.model().kind(child).is_a(ElementKind::Conjugation));
        Some(match (conjugated, direction) {
            (true, "in") => "out",
            (true, "out") => "in",
            (_, it) => it,
        })
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
/// The same collection with nothing in it twice.
///
/// The memberships a type inherits arrive by as many routes as its
/// supertypes have in common: a literal reaches `Base::things::that`
/// five times over, and every feature of `Occurrences::Occurrence`
/// three times. An OCL `Set` and `OrderedSet` hold each value once, so
/// counting the duplicates makes "exactly one return parameter" true of
/// nothing at all -- and makes the walk that finds them several times
/// the work it is.
fn once_each(items: Vec<Val>) -> Vec<Val> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|it| seen.insert(it.clone()))
        .collect()
}

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

/// The ends of an association the metamodel declares on neither of the
/// classes it relates: the association owns them, so a metaclass names
/// only the way in and this is the way back out. Each pairs the name a
/// constraint asks for with the relationship to look through and the
/// property of it that names the element asked about.
const REVERSE_ENDS: [(&str, ElementKind, &str); 6] = [
    ("typing", ElementKind::FeatureTyping, "typedFeature"),
    ("subsetting", ElementKind::Subsetting, "subsettingFeature"),
    (
        "redefinition",
        ElementKind::Redefinition,
        "redefiningFeature",
    ),
    ("specialization", ElementKind::Specialization, "specific"),
    ("conjugator", ElementKind::Conjugation, "conjugatedType"),
    ("valuation", ElementKind::FeatureValue, "featureWithValue"),
];

/// Where the reflective libraries declare a metaclass, in the order the
/// pilot implementation looks for one: `KerML.kerml` splits the KerML
/// abstract syntax into three packages and `SysML.sysml` keeps the SysML
/// one in a single package, and the class name is the same in each.
const METACLASS_PACKAGES: [&str; 4] = [
    "KerML::Root",
    "KerML::Core",
    "KerML::Kernel",
    "SysML::Systems",
];

/// The property `name` itself redefines, where it redefines one.
///
/// This is `redefining` read the other way. A constraint written of a
/// redefining property -- `connectorEnd`, say -- is answered by the
/// derivation of what it redefines (`endFeature`) when the metamodel
/// gives the redefining name no derivation of its own.
fn redefined_property(kind: ElementKind, name: &str) -> Option<&'static str> {
    std::iter::once(kind)
        .chain(kind.ancestors().iter().copied())
        .flat_map(|it| it.own_features())
        .find(|meta| meta.name == name)
        .and_then(|meta| meta.redefines)
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

/// A property name the specification's OCL writes with an `s` the
/// metaclass does not declare, read as the one it does.
///
/// `featuringTypes`, `associationEnds`, `connectorEnds`,
/// `featureMemberships` and `subsettedFeatures` are all written that
/// way, and the metaclasses declare all five in the singular. There is
/// no other reading: the written name belongs to no metaclass at all,
/// and the two are one letter apart. The alternative is answering none
/// of the six constraints that navigate through them.
///
/// This only speaks where the written name is declared nowhere on the
/// metaclass, so a property the model simply does not build still says
/// so rather than being answered under another name.
fn written_as_meant(kind: ElementKind, name: &str) -> &str {
    if kind.feature(name).is_some() {
        return name;
    }
    match name.strip_suffix('s').and_then(|one| kind.feature(one)) {
        Some(meta) => meta.name,
        None => name,
    }
}

/// What the specification's OCL meant by a name it does not declare.
fn meant(written: &str) -> &str {
    match MISSPELLED.iter().find(|(slip, _)| *slip == written) {
        Some((_, meant)) => meant,
        None => written,
    }
}

/// The role of a member the standard allows a type only one of.
///
/// A function has one result parameter, a requirement one subject, a
/// case one objective, a view one rendering. A type may have any number
/// of variants or state subactions, so those say nothing about what is
/// replaced.
fn role_of_one(ws: &Workspace, member: ElementId) -> Option<Role> {
    match ws.model().member_role(member) {
        Some(
            role @ (Role::Return | Role::Result | Role::Subject | Role::Objective | Role::Render),
        ) => Some(role),
        _ => None,
    }
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
        // `[0..1]`, and a feature may own at most one -- which is what
        // `validateFeatureOwnedCrossSubsetting` asks
        "ownedCrossSubsetting" => ElementKind::CrossSubsetting,
        "ownedRedefinition" => ElementKind::Redefinition,
        // The metamodel writes `Feature::redefinition` as an end owned
        // by an association rather than as an attribute of the class,
        // so no metaclass declares it. It is the redefinitions of a
        // feature, and in this model a feature owns every one it has.
        "redefinition" => ElementKind::Redefinition,
        "ownedFeatureMembership" => ElementKind::FeatureMembership,
        "ownedImport" => ElementKind::Import,
        // an annotation owns no annotating element in this model: what
        // annotates owns the annotation, never the other way about
        "ownedAnnotatingElement" => ElementKind::AnnotatingElement,
        // `port def P` owns its conjugate, and the conjugate owns the
        // conjugation that says what it is the conjugate of
        "ownedPortConjugator" => ElementKind::PortConjugation,
        "ownedConjugator" => ElementKind::Conjugation,
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
        // The feature a cross subsetting crosses *from* is the one that
        // owns it -- `crossingFeature` redefines `owningFeature`, and
        // `end cart ... crosses selectedProduct.inCart` is written on
        // the end it crosses from.
        "crossingFeature" => ElementKind::Feature,
        // `comment about A` reifies the annotation under the comment,
        // so the annotating element is the one that owns it
        "owningAnnotatingElement" => ElementKind::AnnotatingElement,
        // The other side of that: an annotating element is owned by an
        // annotation only where the abstract syntax nests it inside
        // one, and this model never does -- the annotation is what the
        // comment owns, so a comment is owned by whatever it is written
        // in and never by an annotation.
        "owningAnnotatingRelationship" => ElementKind::Annotation,
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
        self.holds(&expr, &Val::Elem(elem), &present).ok()
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    /// A workspace over one file, and the element declared under `name`.
    fn about(source: &str, name: &str) -> (Workspace, ElementId) {
        named_in("test.sysml", source, name)
    }

    /// The same of a file the KerML notation is read from.
    fn about_kerml(source: &str, name: &str) -> (Workspace, ElementId) {
        named_in("test.kerml", source, name)
    }

    fn named_in(file: &str, source: &str, name: &str) -> (Workspace, ElementId) {
        let mut ws = Workspace::new();
        ws.add_file(file, source);
        ws.resolve_all();
        let elem = ws
            .named_elements()
            .find(|(_, declared)| *declared == name)
            .map(|(id, _)| id)
            .expect("the element is declared");
        (ws, elem)
    }

    /// Twenty-two constraints are about a membership, and this model
    /// keeps the containment one stands for rather than the membership
    /// itself. Put back together, they are asked of what the model does
    /// hold -- and what one of them reports is the member, which is
    /// what the source wrote.
    #[test]
    fn a_constraint_about_a_membership_is_asked_of_what_stands_for_it() {
        let mut ws = Workspace::new();
        let file = ws.add_file(
            "test.sysml",
            "use case def U {\n\tobjective inside;\n}\n\
             part def P {\n\tobjective outside;\n}\n",
        );
        ws.resolve_all();
        let checked = ws.check_rules(&[file]);

        // asked, and answered both ways: a use case definition is a
        // kind of case definition and may own an objective, and a part
        // definition may not
        assert!(
            checked
                .held
                .contains(&"validateObjectiveMembershipOwningType"),
            "{checked:?}"
        );
        let named: Vec<Option<&str>> = checked
            .violations
            .iter()
            .filter(|it| it.rule == "validateObjectiveMembershipOwningType")
            .map(|it| ws.model().name(it.element))
            .collect();
        assert_eq!(named, vec![Some("outside")], "the member is what is named");
    }

    /// Which part of a transition a member is, and which kind of
    /// constraint a requirement holds it as: both are written as a
    /// keyword rather than kept on anything, so the membership the
    /// model stands for is where they are worked out.
    #[test]
    fn a_membership_says_which_kind_of_one_it_is() {
        let mut ws = Workspace::new();
        ws.add_file(
            "test.sysml",
            "state def S {\n\tattribute def E;\n\
             \ttransition first a accept e : E if true do action f then b;\n\
             \tstate a;\n\tstate b;\n\taction f;\n}\n",
        );
        ws.resolve_all();
        let transition = ws
            .model()
            .ids()
            .find(|&id| ws.model().kind(id) == ElementKind::TransitionUsage)
            .expect("the transition is built");
        assert_eq!(
            ws.judge(
                "ownedMembership->selectByKind(TransitionFeatureMembership)\
                 ->exists(kind = TransitionFeatureKind::guard)",
                transition
            ),
            Some(true)
        );

        let (mut ws, requirement) = about(
            "requirement def R {\n\tassume constraint c;\n\trequire constraint d;\n}\n",
            "R",
        );
        assert_eq!(
            ws.judge(
                "ownedMembership->selectByKind(RequirementConstraintMembership)\
                 ->collect(kind)->asSet() = Set{'assumption', 'requirement'}",
                requirement
            ),
            Some(true)
        );
        // and a membership the notation says nothing of the sort about
        // says so rather than guessing
        let (mut ws, part) = about("part def P {\n\tpart w;\n}\n", "P");
        assert_eq!(ws.judge("ownedMembership->exists(kind = 'x')", part), None);
    }

    /// `FlowDefinition::flowEnd` is declared derived and derived
    /// nowhere, so what a flow definition has of them is what it owns.
    #[test]
    fn a_flow_definition_has_the_flow_ends_it_owns() {
        let (mut ws, f) = about(
            "flow def F;\npart def P {\n\tpart a;\n\tflow from a to a;\n}\n",
            "F",
        );
        assert_eq!(ws.judge("flowEnd->isEmpty()", f), Some(true));
        assert_eq!(ws.judge("flowEnd->size() <= 2", f), Some(true));
    }

    /// "The Types that feature this Feature": the type that owns it as
    /// a feature, or what a `featured by` says instead -- and, for a
    /// multiplicity, which no type owns as a feature, wherever the
    /// feature carrying it is featured.
    #[test]
    fn what_features_a_feature_is_what_owns_it_or_what_it_says() {
        let source = "package K {\n\
             \tclassifier C;\n\
             \tclassifier D {\n\t\tfeature x : C[0..1];\n\t}\n\
             \tfeature w : C featured by D;\n\
             \tfeature v : C;\n}\n";
        let features_d = "featuringType->size() = 1 and featuringType->forAll(declaredName = 'D')";
        let (mut ws, x) = about_kerml(source, "x");
        assert_eq!(ws.judge(features_d, x), Some(true));
        // the multiplicity of `x` is owned by `x` and not as a feature
        // of it, so it is featured where `x` is
        assert_eq!(
            ws.judge("multiplicity.featuringType = featuringType", x),
            Some(true)
        );

        let (mut ws, w) = about_kerml(source, "w");
        assert_eq!(ws.judge(features_d, w), Some(true));
        // and a package owns nothing as a feature of itself
        let (mut ws, v) = about_kerml(source, "v");
        assert_eq!(ws.judge("featuringType->isEmpty()", v), Some(true));
    }

    /// The ends the metamodel gives to an association rather than to
    /// either class it relates, which are read by looking the other way
    /// about -- and read from what was worked out once.
    #[test]
    fn the_ends_an_association_owns_are_read_the_other_way_about() {
        let (mut ws, w) = about("part def Car {\n\tpart v;\n\tpart w : Car :> v;\n}\n", "w");
        assert_eq!(ws.judge("typing->size() = 1", w), Some(true));
        assert_eq!(ws.judge("subsetting->size() = 1", w), Some(true));
        // a subsetting and a typing are both specializations
        assert_eq!(ws.judge("specialization->size() = 2", w), Some(true));
        // asked twice, from the one pass over the model
        assert_eq!(ws.judge("typing->size() = 1", w), Some(true));
        // and nothing redefines it
        assert_eq!(ws.judge("redefinition->isEmpty()", w), Some(true));

        // and the conjugation of `class B conjugates A;` is read from
        // the type it names rather than from the one it conjugates to
        let (mut ws, b) = about_kerml("package K {\n\tclass A;\n\tclass B conjugates A;\n}\n", "B");
        assert_eq!(ws.judge("conjugator->size() = 1", b), Some(true));
        let (mut ws, a) = about_kerml("package K {\n\tclass A;\n\tclass B conjugates A;\n}\n", "A");
        assert_eq!(ws.judge("conjugator->isEmpty()", a), Some(true));
    }

    /// The three the specification writes a metadata feature's
    /// constraint in terms of: the name an element answers to from the
    /// root, the element a name reads back to, and the metaclass of a
    /// thing -- which the constraint names so as to read it back.
    #[test]
    fn a_name_is_read_from_the_root_and_a_metaclass_named_by_it() {
        let mut ws = Workspace::new();
        ws.add_file(
            "kerml.kerml",
            "standard library package KerML {\n\
             \tpackage Core {\n\t\tmetaclass PartDefinition;\n\t}\n}\n",
        );
        ws.add_file(
            "test.sysml",
            "part def Car {\n\tdoc /* what is driven */\n\tpart w;\n}\n",
        );
        ws.resolve_all();
        let named = |ws: &Workspace, want: &str| {
            ws.named_elements()
                .find(|(_, declared)| *declared == want)
                .map(|(id, _)| id)
                .expect("the element is declared")
        };
        let (car, wheel) = (named(&ws, "Car"), named(&ws, "w"));

        assert_eq!(ws.judge("qualifiedName = 'Car'", car), Some(true));
        assert_eq!(ws.judge("qualifiedName = 'Car::w'", wheel), Some(true));
        // and the metaclass by the name it answers to, read back to
        // the same element it was named from
        assert_eq!(
            ws.judge(
                "oclType().qualifiedName = 'KerML::Core::PartDefinition'",
                car
            ),
            Some(true)
        );
        assert_eq!(
            ws.judge(
                "resolveGlobal('KerML::Core::PartDefinition').memberElement = oclType()",
                car
            ),
            Some(true)
        );
        // read from the root, so what it is written of does not matter
        // -- `Feature::canAccess` writes it of a property a Feature
        // does not have -- and `modelElement` is what one caller reads
        // off what comes back
        assert_eq!(
            ws.judge(
                "subsettingFeature.resolveGlobal('KerML::Core::PartDefinition').modelElement \
                 = oclType()",
                car
            ),
            Some(true)
        );
        // a name nothing answers to reads back to nothing, which is
        // an answer and not a refusal to answer
        assert_eq!(
            ws.judge("resolveGlobal('Nowhere::atAll') = null", car),
            Some(true)
        );
        // and what cannot be asked says so rather than guessing: a
        // metaclass the library does not declare, the metaclass of
        // something that is not an element, a name that is not one,
        // and the name of something with none
        assert_eq!(ws.judge("oclType() = null", wheel), None);
        assert_eq!(ws.judge("declaredName.oclType() = null", car), None);
        assert_eq!(ws.judge("resolveGlobal(1) = null", car), None);
        let unnamed = ws
            .model()
            .ids()
            .find(|&id| ws.model().name(id).is_none() && ws.model().owner(id).is_some())
            .expect("the model builds something anonymous");
        assert_eq!(ws.judge("qualifiedName = null", unnamed), Some(true));
        let root = ws
            .model()
            .ids()
            .find(|&id| ws.model().owner(id).is_none())
            .expect("everything is read from the root");
        assert_eq!(ws.judge("qualifiedName = null", root), Some(true));
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
        // new turns up rather than going round for ever, and answers
        // with what it started from as well
        assert_eq!(
            ws.judge("Set{1}->closure(n | Set{})->size() = 1", car),
            Some(true)
        );
        assert_eq!(
            ws.judge(
                "Set{1}->closure(n | if n = 1 then Set{2} else Set{} endif)->size() = 2",
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
        // `owningNamespace.qualifiedName + '::' + escapedName()` --
        // the specification adds strings to build a name, and numbers
        // where it counts
        assert_eq!(ws.judge("'a' + 'b' = 'ab'", car), Some(true));
        assert_eq!(ws.judge("1 + 2 = 3", car), Some(true));
        // `Set(Element){}` names the type its emptiness is empty of,
        // which says nothing the values do not
        assert_eq!(ws.judge("Set(Element){}->isEmpty()", car), Some(true));
        // `*` is the unbounded end of a multiplicity: equal to itself
        // and to no number, which is what a bound is compared for
        assert_eq!(ws.judge("* = *", car), Some(true));
        assert_eq!(ws.judge("* = 1", car), Some(false));
        assert_eq!(ws.judge("0 = *", car), Some(false));
        // `relatedFeature->subSequence(2, size)` is every end but the
        // first, counted from one and taking both ends
        assert_eq!(
            ws.judge("Sequence{1, 2, 3}->subSequence(2, 3)->size() = 2", car),
            Some(true)
        );
        // one past the end is the empty sequence, which is what a
        // connector with a single related feature asks for
        assert_eq!(
            ws.judge("Sequence{1}->subSequence(2, 1)->isEmpty()", car),
            Some(true)
        );
        assert_eq!(
            ws.judge("Sequence{1}->subSequence(0, 1)->size() = 1", car),
            None
        );
        // an end that is not a number, on either side, and one written
        // without the pair of them
        assert_eq!(
            ws.judge("Sequence{1}->subSequence('a', 1)->isEmpty()", car),
            None
        );
        assert_eq!(
            ws.judge("Sequence{1}->subSequence(1, 'a')->isEmpty()", car),
            None
        );
        assert_eq!(
            ws.judge("Sequence{1}->subSequence(1)->isEmpty()", car),
            None
        );
        // a cast reads a property and the cast itself adds nothing;
        // the metamodel spells it both ways
        assert_eq!(ws.judge("oclAsType(Type).isAbstract", car), Some(false));
        assert_eq!(ws.judge("oclAsKindOf(Type).isAbstract", car), Some(false));
        // What an operation returns is not one of the arguments it is
        // called with. Six name that parameter `result`, and each is
        // called with one fewer than it declares -- so `evaluate` takes
        // the one the specification passes it.
        assert_eq!(
            sysml_model::OPERATIONS
                .iter()
                .filter(|it| it.name == "evaluate")
                .map(|it| it.parameters)
                .collect::<Vec<_>>(),
            [&["target"]; 5]
        );
        // the exact question, which the specification never writes,
        // beside the kind question it spells `oclIsType`
        assert_eq!(ws.judge("oclIsTypeOf(PartDefinition)", car), Some(true));
        assert_eq!(ws.judge("oclIsTypeOf(Definition)", car), Some(false));
        assert_eq!(ws.judge("oclIsType(Definition)", car), Some(true));
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
        // an operation written of many things is written of each of
        // them, and what each answers with joins what the rest do
        assert_eq!(
            ws.judge("Set{self}.supertypes(false)->notEmpty()", car),
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
        // and there is nothing an empty answer could be hiding. The
        // same goes for each kind of relationship the builder does
        // write. Relationships at large are what it writes only some
        // of, so there an empty answer says nothing either way.
        let (mut ws, car) = about("part def Car;\n", "Car");
        assert_eq!(ws.judge("ownedMember->isEmpty()", car), Some(true));
        assert_eq!(ws.judge("ownedSpecialization->isEmpty()", car), Some(true));
        assert_eq!(ws.judge("ownedRelationship->isEmpty()", car), None);
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

    /// `isConjugated` is the one derived property of a Type that the
    /// metamodel states in prose and states nowhere in OCL, so nothing
    /// the evaluation reads can work it out. It is answered from what
    /// the type owns, which is what the prose says it means.
    #[test]
    fn a_type_is_conjugated_when_it_owns_the_conjugation_that_says_so() {
        let conjugated = "package K {\n\tclass A;\n\tclass B conjugates A;\n}\n";
        let mut ws = Workspace::new();
        ws.add_file("c.kerml", conjugated);
        ws.resolve_all();
        let named = |ws: &Workspace, want: &str| {
            ws.named_elements()
                .find(|(_, name)| *name == want)
                .map(|(id, _)| id)
                .expect("the class is declared")
        };
        let (a, b) = (named(&ws, "A"), named(&ws, "B"));
        assert_eq!(ws.judge("isConjugated", b), Some(true));
        // the original of a conjugation is not itself conjugated
        assert_eq!(ws.judge("isConjugated", a), Some(false));
    }

    /// The specification's own OCL names five properties with a final
    /// `s` that no metaclass declares, and declares all five without
    /// it. Read as written they answer nothing at all; read as meant
    /// they answer what the constraint is asking about.
    #[test]
    fn a_name_the_specification_writes_with_an_s_it_declares_without_one() {
        let (mut ws, car) = about("part def Car {\n\tpart w;\n}\n", "Car");
        // `Type::featureMembership` is what the metamodel declares, and
        // `validateElementFilterMembershipConditionIsBoolean` navigates
        // `featureMemberships`
        assert_eq!(
            ws.judge(
                "featureMemberships->size() = featureMembership->size()",
                car
            ),
            Some(true)
        );
        // and the reading only speaks where the written name is
        // declared nowhere: `featuringType` is a Feature's, so asking a
        // definition for it is still unanswered rather than answered
        // under a name that happens to be one letter away
        assert_eq!(ws.judge("featuringTypes->isEmpty()", car), None);
    }

    /// A `Set` and an `OrderedSet` hold each value once; a `Bag` and a
    /// `Sequence` hold what they were given.
    ///
    /// What this is really about is the memberships a type inherits.
    /// They arrive by as many routes as its supertypes have in common,
    /// and a literal reaches `Base::things::that` five times over. Held
    /// five times, "exactly one return parameter" is true of nothing.
    #[test]
    fn a_set_holds_each_value_once_and_a_sequence_holds_what_it_was_given() {
        let (mut ws, car) = about("part def Car {\n\tpart w;\n}\n", "Car");
        assert_eq!(
            ws.judge("Set{1, 1, 2}->asSet()->size() = 2", car),
            Some(true)
        );
        assert_eq!(
            ws.judge("Set{1, 2}->union(Set{2, 3})->size() = 3", car),
            Some(true)
        );
        assert_eq!(
            ws.judge("Set{1, 1, 2}->asSequence()->size() = 3", car),
            Some(true)
        );
    }

    /// A transitive closure answers with what it started from as well.
    ///
    /// `Feature::allRedefinedFeatures()` is written
    /// `ownedRedefinition.redefinedFeature->
    /// closure(ownedRedefinition.redefinedFeature)->asOrderedSet()->
    /// prepend(self)`. Read without the source it says only `self`,
    /// since nearly every feature redefines directly and nothing
    /// further -- and `removeRedefinedFeatures`, which is how a type
    /// stops inheriting what it has redefined, goes down with it.
    #[test]
    fn a_closure_answers_with_what_it_started_from() {
        const CHAIN: &str = "part def Car {\n\tpart u;\n\tpart v :>> u;\n\tpart w :>> v;\n}\n";
        const WALK: &str =
            "ownedRedefinition.redefinedFeature->closure(ownedRedefinition.redefinedFeature)";
        // `w` redefines `v`, which redefines `u`: the walk answers with
        // both, the one it started from included
        let (mut ws, w) = about(CHAIN, "w");
        assert_eq!(ws.judge(&format!("{WALK}->size() = 2"), w), Some(true));
        // `u` redefines nothing, so the walk answers with nothing
        let (mut ws, u) = about(CHAIN, "u");
        assert_eq!(ws.judge(&format!("{WALK}->isEmpty()"), u), Some(true));
    }

    /// What the metamodel declares single-valued answers with the
    /// value, not with a collection holding it.
    ///
    /// `Type::ownedConjugator` is `[0..1]` and
    /// `ConjugatedPortDefinition::ownedPortConjugator` is `[1..1]`, and
    /// `ownedPortConjugator.originalPortDefinition =
    /// originalPortDefinition` compares one against one -- a collection
    /// of one is equal to neither side of it.
    #[test]
    fn what_the_metamodel_declares_one_of_is_answered_as_one() {
        let (mut ws, b) = about_kerml("package K {\n\tclass A;\n\tclass B conjugates A;\n}\n", "B");
        assert_eq!(ws.judge("ownedConjugator <> null", b), Some(true));
        assert_eq!(
            ws.judge("ownedConjugator.originalType <> null", b),
            Some(true)
        );
        // a class that conjugates nothing has no conjugator, which is
        // null rather than an empty collection
        let (mut ws, a) = about_kerml("package K {\n\tclass A;\n\tclass B conjugates A;\n}\n", "A");
        assert_eq!(ws.judge("ownedConjugator = null", a), Some(true));
        // and what it declares many of still answers with all of them
        let (mut ws, car) = about("part def Car {\n\tpart v;\n\tpart w :> v;\n}\n", "w");
        assert_eq!(ws.judge("ownedSubsetting->size() = 1", car), Some(true));
    }

    /// Every closing this supplies is one the specification's own text
    /// leaves out, and supplying it is what lets the body be read.
    ///
    /// Held so that an entry cannot go stale: were the metamodel to
    /// close one of these itself, the text it is written against would
    /// no longer be there to close.
    #[test]
    fn what_is_closed_here_is_open_in_the_specification() {
        for (name, written, meant) in UNCLOSED {
            let bodies: Vec<&str> = sysml_model::DERIVATIONS
                .iter()
                .filter(|it| it.name == name)
                .map(|it| it.ocl)
                .chain(
                    sysml_model::OPERATIONS
                        .iter()
                        .filter(|it| it.name == name)
                        .map(|it| it.ocl),
                )
                .chain(
                    sysml_model::RULES
                        .iter()
                        .filter(|it| it.name == name)
                        .map(|it| it.ocl),
                )
                .collect();
            // a name may be stated of several metaclasses, and only
            // the text carrying the slip is the one this closes
            let open: Vec<&str> = bodies
                .into_iter()
                .filter(|it| it.contains(written))
                .collect();
            assert_eq!(open.len(), 1, "`{name}` is written `{written}` once");
            let body = |ocl: &str| match ocl.split_once('=') {
                Some((head, body)) if head.trim().chars().all(char::is_alphanumeric) => {
                    body.to_string()
                }
                _ => ocl.to_string(),
            };
            assert!(
                crate::ocl::parse(&body(open[0])).is_err(),
                "`{name}` cannot be read as the specification writes it"
            );
            assert!(
                crate::ocl::parse(&body(&closed(name, open[0]))).is_ok(),
                "`{name}` reads once `{meant}` closes it"
            );
        }
    }

    /// What a type inherits is what it specializes, less what is
    /// private to that and less what a redefinition has replaced.
    ///
    /// The metamodel works this out through five operations that call
    /// one another over every supertype, and no depth of evaluation
    /// completes them. The resolver walks the same specializations to
    /// find a name, so that walk is the answer.
    #[test]
    fn what_a_type_inherits_is_what_it_specializes_declares() {
        const MODEL: &str = "part def Sup {\n\tpart w;\n\tprivate part kept;\n}\n                             part def Sub :> Sup;\n";
        let (mut ws, sub) = about(MODEL, "Sub");
        // `w` and not `kept`: what a type keeps to itself is not
        // inherited
        assert_eq!(ws.judge("inheritedMembership->size() = 1", sub), Some(true));
        let (mut ws, sup) = about(MODEL, "Sup");
        assert_eq!(ws.judge("inheritedMembership->isEmpty()", sup), Some(true));

        // A member declared with a role the standard allows one of
        // stands in the place of the one that would be inherited.
        const ROLES: &str = "requirement def R {\n\tsubject s;\n}\n                             requirement def R2 :> R {\n\tsubject t;\n}\n                             requirement def R3 :> R;\n";
        let (mut ws, two) = about(ROLES, "R2");
        assert_eq!(
            ws.judge(
                "inheritedMembership->selectByKind(SubjectMembership)->isEmpty()",
                two
            ),
            Some(true)
        );
        // and one that declares none inherits the one that is there
        let (mut ws, three) = about(ROLES, "R3");
        assert_eq!(
            ws.judge(
                "inheritedMembership->selectByKind(SubjectMembership)->size() = 1",
                three
            ),
            Some(true)
        );
    }

    /// Every membership in a namespace: the containments it keeps and
    /// what its imports bring in. The metamodel derives it as the union
    /// of the two, and the resolver works out what an import brings in
    /// for name lookup already.
    #[test]
    fn every_membership_of_a_namespace_is_what_it_keeps_and_what_it_brings_in() {
        const MODEL: &str = "package P {\n\tpart a;\n}\n                             package Q {\n\timport P::*;\n\tpart b;\n}\n";
        let (mut ws, q) = about(MODEL, "Q");
        // what it owns is `b`; the import is a relationship and not a
        // membership, however many it brings in
        assert_eq!(ws.judge("ownedMembership->size() = 1", q), Some(true));
        // and `a` comes in besides
        assert_eq!(ws.judge("membership->size() = 2", q), Some(true));
        // a namespace importing nothing keeps only what it owns
        let (mut ws, p) = about(MODEL, "P");
        assert_eq!(ws.judge("membership->size() = 1", p), Some(true));
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

    /// What a usage is typed by, under the name its own metaclass
    /// gives it.
    ///
    /// `Usage::definition` -- "the Definitions that are types of this
    /// Usage" -- and `occurrenceDefinition`, `partDefinition` and their
    /// kin beside it. The metamodel states each in prose and none of
    /// them in OCL. `individual def IO1;` is an occurrence definition
    /// that names no kind, which is what `isIndividual` is declared on.
    #[test]
    fn a_usage_is_typed_by_the_definitions_it_names() {
        let source = "package K {\n\
                      \tindividual def IO1;\n\
                      \tpart def P;\n\
                      \tpart p : P;\n\
                      \tindividual occurrence io : IO1;\n\
                      \tindividual timeslice t :> io;\n\
                      }\n";
        let (mut ws, part) = about(source, "p");
        assert_eq!(ws.judge("partDefinition->notEmpty()", part), Some(true));
        // `individual def` names no kind and is an occurrence
        // definition all the same
        let (mut ws, occurrence) = about(source, "io");
        assert_eq!(
            ws.judge(
                "occurrenceDefinition->selectByKind(OccurrenceDefinition)\
                 ->select(isIndividual)->size() = 1",
                occurrence
            ),
            Some(true)
        );
        // and a usage is typed by what the usage it subsets is typed by
        let (mut ws, slice) = about(source, "t");
        assert_eq!(ws.judge("individualDefinition <> null", slice), Some(true));
    }

    /// Which way a feature is passed, as seen from a type.
    ///
    /// `Type::directionOfExcluding` is defined by recursion over every
    /// supertype of every feature of every type, which the bound on
    /// derivation depth stops short of -- and it stands between every
    /// constraint that counts what a behaviour is handed and an answer.
    /// A conjugated type turns what it takes in into what it puts out.
    #[test]
    fn the_direction_of_a_feature_is_seen_from_a_type() {
        let (mut ws, def) = about(
            "port def P {\n\tin attribute a;\n}\npart def Q :> P;\n",
            "P",
        );
        assert_eq!(
            ws.judge(
                "directionOf(feature->any(true)) = FeatureDirectionKind::_'in'",
                def
            ),
            Some(true)
        );
        // what a type does not own, it gives the direction its
        // supertypes give
        let (mut ws, sub) = about(
            "port def P {\n\tin attribute a;\n}\npart def Q :> P;\n",
            "Q",
        );
        assert_eq!(
            ws.judge(
                "feature->select(f | directionOf(f) = FeatureDirectionKind::_'in')->notEmpty()",
                sub
            ),
            Some(true)
        );
        // and one it never heard of has none
        assert_eq!(ws.judge("directionOf(self) = null", sub), Some(true));
        // asked of something that is not a feature of a type, it cannot
        // say either way
        assert_eq!(ws.judge("directionOf(1) = null", sub), None);
        assert_eq!(ws.judge("self.isAbstract.directionFor(self)", sub), None);

        // a conjugated type turns what it takes in into what it puts
        // out, and the other way about: `port p : ~P` is typed by the
        // conjugate `P` owns
        let source = "port def P {\n\tin attribute a;\n\tout attribute b;\n}\n\
                      part def Q {\n\tport p : ~P;\n}\n";
        let (mut ws, conjugate) = about(source, "~P");
        for seen in ["out", "_'in'"] {
            assert_eq!(
                ws.judge(
                    &format!(
                        "feature->select(f | directionOf(f) = FeatureDirectionKind::{seen})\
                         ->size() = 1"
                    ),
                    conjugate
                ),
                Some(true),
                "one feature of the conjugate is {seen}"
            );
        }
        // `Feature::directionFor(type) = type.directionOf(self)` is the
        // same question asked from the feature's side
        let (mut ws, taken) = about(source, "a");
        assert_eq!(
            ws.judge(
                "directionFor(owningType) = FeatureDirectionKind::_'in'",
                taken
            ),
            Some(true)
        );
    }

    /// A parameter membership fixes the direction of what it owns, and
    /// each membership names that one thing under a name of its own.
    ///
    /// `ParameterMembership::parameterDirection = FeatureDirectionKind::
    /// _'in'`, so a `subject` is what a requirement takes in --
    /// `input->first() = subjectParameter` asks for a subject that is an
    /// input, and none of them was one. And
    /// `SubjectMembership::ownedSubjectParameter` is that subject, under
    /// the name the membership metaclass gives it.
    #[test]
    fn a_parameter_membership_directs_and_names_what_it_owns() {
        let (mut ws, req) = about("requirement def R {\n\tsubject s;\n\tactor a;\n}\n", "R");
        assert_eq!(ws.judge("input->notEmpty()", req), Some(true));
        assert_eq!(
            ws.judge("input->first() = subjectParameter", req),
            Some(true)
        );
        // an actor is taken in the same way
        let (mut ws, actor) = about("requirement def R {\n\tsubject s;\n\tactor a;\n}\n", "a");
        assert_eq!(
            ws.judge("direction = FeatureDirectionKind::_'in'", actor),
            Some(true)
        );
        // and what is handed to something is not part of what it is
        // made of
        assert_eq!(ws.judge("isReference", actor), Some(true));
    }

    /// `direction` is written where it holds and nowhere else.
    ///
    /// `in`, `out` and `inout` are all the notation has to write, so a
    /// feature written without one has none -- the metamodel declares
    /// the property optional and states no default. Read as a property
    /// this model does not build, every constraint that counts a
    /// behaviour's input parameters was answered "cannot say".
    #[test]
    fn a_direction_the_source_did_not_write_is_none() {
        let (mut ws, w) = about("part def Car {\n\tin part w;\n}\n", "w");
        assert_eq!(
            ws.judge("direction = FeatureDirectionKind::_'in'", w),
            Some(true)
        );
        assert_eq!(ws.judge("direction = null", w), Some(false));
        let (mut ws, w) = about("part def Car {\n\tpart w;\n}\n", "w");
        assert_eq!(ws.judge("direction = null", w), Some(true));
        // and a metaclass that declares no direction at all still
        // cannot say
        let (mut ws, car) = about("part def Car;\n", "Car");
        assert_eq!(ws.judge("direction = null", car), None);
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
            "action def A {\n\tattribute v;\n\taction x;\n\tassign v := 1;\n}\n",
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
        // the ones the specification's own text defeats are among them,
        // carrying what they are rather than a model's answer
        let named: Vec<&str> = checked.unevaluated.iter().map(|(name, _)| *name).collect();
        assert!(
            named.contains(&"validateUsageVariationSpecialization"),
            "{named:?}"
        );
        // and every reason says what could not be answered
        assert!(checked.unevaluated.iter().all(|(_, why)| !why.is_empty()));
    }
}

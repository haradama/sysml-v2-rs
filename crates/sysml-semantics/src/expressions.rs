//! The names written inside an expression.
//!
//! The model keeps an expression as the text its author wrote rather than
//! as a tree of elements, so the names in a constraint body, in the result
//! of a `calc` and after an `=` are read off the syntax and looked up from
//! the element the expression belongs to -- the scope the language gives
//! them. Working it out is a pass of its own: what an invocation hands its
//! arguments to, what a chain steps through, what a cast casts to.

use std::collections::HashSet;

use sysml_model::{ElementId, ElementKind, Value};
use sysml_syntax::SyntaxNode;

use crate::lookup::Reach;
use crate::syntax::*;
use crate::{Access, ResolveStats, Workspace};

impl Workspace {
    /// What each expression in the tree refers to, and what it invokes.
    ///
    /// A feature reference names a feature; an invocation names the function
    /// it hands its arguments to, which an operator names through the
    /// specification's operator table. Both are held on a `Membership` the
    /// builder stands there ahead of everything else, since
    /// `instantiatedType()` and `referent` each read the first one.
    pub(crate) fn resolve_expression_tree(&mut self, asked: &[ElementId]) {
        // Only what was asked for. Walking a name before the file it is
        // written in has been resolved settles what the walk found into
        // the caches this workspace keeps, and the answer a later pass
        // gets is then the one from before that file was there.
        let asked: HashSet<ElementId> = asked.iter().copied().collect();
        // What the expression builder made, and only that: a
        // `RequirementUsage` is a kind of `BooleanExpression` in SysML,
        // and reading one as an invocation hands its subject over as an
        // argument of something it never invoked.
        let expressions: Vec<(ElementId, SyntaxNode)> = self
            .expressions
            .clone()
            .into_iter()
            .filter(|elem| asked.contains(elem))
            .filter_map(|elem| Some((elem, self.source.get(&elem)?.clone())))
            .collect();
        for (elem, node) in expressions {
            // `x istype T` names the type it asks about through a
            // membership of its own, beside the one naming the function
            if self.model.kind(elem) == ElementKind::Membership {
                match self.model.owner(elem).map(|it| self.model.kind(it)) {
                    Some(ElementKind::FeatureChainExpression) => self.chains_to(elem, &node),
                    _ => self.names_what_it_asks_about(elem, &node),
                }
                continue;
            }
            if self.model.kind(elem) == ElementKind::FeatureReferenceExpression {
                self.refers_where_it_is_written(elem);
                continue;
            }
            // `a#(1)` comes to one of the `a`s, whatever function the
            // `#` invokes -- and the argument it indexes is read before
            // the function is, since the rule holds where no library
            // is loaded to name one.
            if self.model.kind(elem) == ElementKind::IndexExpression {
                self.comes_to_one_of_what_it_indexes(elem);
            }
            let triggered = match self.model.maybe(elem, "kind") {
                Some(Value::EnumLit(kind)) => triggered_function(kind),
                _ => None,
            };
            let found = match (
                self.model.maybe(elem, "operator").and_then(Value::as_str),
                triggered,
            ) {
                (Some(operator), _) => {
                    invoked_function(operator).and_then(|it| self.named_globally(it))
                }
                (None, Some(named)) => self.named_globally(named),
                (None, None) => match invoked_through_arrow(&node) {
                    Some(named) => self.resolve_operand(elem, &[named]),
                    None => invoked_by_name(&node)
                        .and_then(|callee| self.resolve_operand(elem, &operand_segments(&callee))),
                },
            };
            let Some(target) = found else {
                continue;
            };
            let standing = self
                .model
                .owned(elem)
                .iter()
                .copied()
                .find(|&child| self.model.kind(child) == ElementKind::Membership);
            if let Some(membership) = standing {
                self.try_set(membership, "memberElement", Value::Ref(target));
            }
            // `private calc getElapsedUtcTime { ... }` names no type,
            // and `validateInvocationExpressionInstantiatedType` holds
            // what invokes it to invoking something typed by a
            // behaviour. The notation writes that type nowhere.
            self.materialize_implied_for(target);
            self.hands_over_what_it_takes(elem, target);
            self.comes_to_what_it_invokes(elem, target);
        }
    }
    /// What a feature reference refers to, looked up where it is
    /// written -- unless that is settled already.
    fn refers_where_it_is_written(&mut self, reference: ElementId) {
        if self.model.maybe(reference, "referent").is_some() {
            return;
        }
        let segments = self
            .source
            .get(&reference)
            .map(operand_segments)
            .unwrap_or_default();
        if let Some(target) = self.resolve_operand(reference, &segments) {
            self.refers_to(reference, target);
        }
    }
    /// What an index expression comes to.
    ///
    /// `checkIndexExpressionResultSpecialization`: the result of `a#(1)`
    /// subsets the result of `a`.
    fn comes_to_one_of_what_it_indexes(&mut self, index: ElementId) {
        let Some((mine, theirs)) = self.indexes(index) else {
            return;
        };
        self.reified(
            mine,
            ElementKind::Subsetting,
            &[
                ("subsettingFeature", Value::Ref(mine)),
                ("subsettedFeature", Value::Ref(theirs)),
                ("isImplied", Value::Bool(true)),
            ],
        );
        self.model.set(mine, "isImpliedIncluded", Value::Bool(true));
        // what it specializes was worked out before this was written
        self.supertypes.remove(&mine);
    }
    /// The result of an index expression and the result of what it
    /// indexes -- or nothing, where that is a collection: `#` on a
    /// `Collections::Array` picks an element out of it, which is not an
    /// array. Whether it is one is known once `a` has been looked up,
    /// and the builder stands the expression in front of its argument,
    /// so the argument is looked up here rather than waited for.
    fn indexes(&mut self, index: ElementId) -> Option<(ElementId, ElementId)> {
        let argument = self.takes(index).into_iter().next()?;
        let sequence = self
            .model
            .owned(argument)
            .iter()
            .copied()
            .find(|&it| self.model.kind(it) == ElementKind::FeatureValue)
            .and_then(|it| self.model.maybe(it, "value").and_then(Value::as_id))?;
        if self.model.kind(sequence) == ElementKind::FeatureReferenceExpression {
            self.refers_where_it_is_written(sequence);
        }
        let theirs = self.hands_back(sequence)?;
        let collection = self.named_globally("Collections::Collection");
        if collection.is_some_and(|it| self.reaches(theirs, it)) {
            return None;
        }
        Some((self.hands_back(index)?, theirs))
    }
    /// The type a classification operator asks about.
    ///
    /// `x istype T` hands over `x` and names `T`, and the name is held
    /// on a membership of the expression's own beside the one naming
    /// the function it invokes.
    fn names_what_it_asks_about(&mut self, membership: ElementId, node: &SyntaxNode) {
        let segments = operand_segments(node);
        let Some(target) = self.resolve_operand(membership, &segments) else {
            return;
        };
        self.try_set(membership, "memberElement", Value::Ref(target));
        self.comes_to_what_it_casts_to(membership, target);
    }
    /// A cast comes to the type it casts to.
    ///
    /// `as` is "select instances of type (cast)", and `BaseFunctions::'as'`
    /// returns `Anything`: the type is named beside the operator rather than
    /// returned, so what the expression comes to is nowhere in the model
    /// unless this puts it there. `(that as SpatialItem).localClock` reads
    /// `localClock` on the strength of it.
    fn comes_to_what_it_casts_to(&mut self, membership: ElementId, cast: ElementId) {
        let casts = self
            .model
            .owner(membership)
            .filter(|&it| {
                matches!(
                    self.model.maybe(it, "operator").and_then(Value::as_str),
                    Some("as" | "meta")
                )
            })
            .and_then(|expression| self.hands_back(expression));
        let Some(result) = casts else {
            return;
        };
        let (kind, from, to) = match self.model.kind(cast).is_a(ElementKind::Classifier) {
            true => (ElementKind::FeatureTyping, "typedFeature", "type"),
            false => (
                ElementKind::Subsetting,
                "subsettingFeature",
                "subsettedFeature",
            ),
        };
        self.reified(
            result,
            kind,
            &[
                (from, Value::Ref(result)),
                (to, Value::Ref(cast)),
                ("isImplied", Value::Bool(true)),
            ],
        );
        self.model
            .set(result, "isImpliedIncluded", Value::Bool(true));
        // what it specializes was worked out before this was written
        self.supertypes.remove(&result);
    }
    /// The feature a chain expression chains to.
    ///
    /// "If the membershipOwningNamespace is a FeatureChainExpression, then the
    /// local Namespace is the result parameter of the argument Expression":
    /// `(that as SpatialItem).localClock` reads `localClock` from what `that
    /// as SpatialItem` comes to, which for a cast is the type it casts to.
    fn chains_to(&mut self, membership: ElementId, node: &SyntaxNode) {
        let owner = self
            .model
            .owner(membership)
            .expect("the builder puts the membership on the expression it chains from");
        let mut segments = operand_segments(node);
        let Some(name) = segments.pop() else {
            return;
        };
        let held: Vec<ElementId> = self
            .takes(owner)
            .into_iter()
            .filter_map(|argument| {
                self.model
                    .owned(argument)
                    .iter()
                    .copied()
                    .find(|&it| self.model.kind(it) == ElementKind::FeatureValue)
                    .and_then(|it| self.model.maybe(it, "value").and_then(Value::as_id))
            })
            .collect();
        let mut scopes = Vec::new();
        for expression in held {
            // a cast names the type it casts to outright
            scopes.extend(
                self.model
                    .owned(expression)
                    .iter()
                    .copied()
                    .filter(|&it| self.model.kind(it) == ElementKind::Membership)
                    .filter_map(|it| self.model.get(it, "memberElement").and_then(Value::as_id))
                    .collect::<Vec<_>>(),
            );
            if let Some(result) = self.hands_back(expression) {
                scopes.extend(self.supertypes_of(result));
            }
        }
        for scope in scopes {
            // a chain chains to a feature: `targetFeature` is null where
            // what the name reaches is not one, which is what the
            // derivation's own `oclIsKindOf(Feature)` guard says
            let found = self
                .lookup(scope, &name, Reach::all(Access::External))
                .filter(|&it| self.model.kind(it).is_a(ElementKind::Feature));
            if let Some(target) = found {
                self.try_set(membership, "memberElement", Value::Ref(target));
                // `checkFeatureChainExpressionResultSpecialization` --
                // what a chain comes to is what it chains to
                if let Some(result) = self.hands_back(owner) {
                    self.reified(
                        result,
                        ElementKind::Subsetting,
                        &[
                            ("subsettingFeature", Value::Ref(result)),
                            ("subsettedFeature", Value::Ref(target)),
                            ("isImplied", Value::Bool(true)),
                        ],
                    );
                    self.model
                        .set(result, "isImpliedIncluded", Value::Bool(true));
                    self.supertypes.remove(&result);
                }
                return;
            }
        }
    }
    /// Each argument redefines the parameter it is handed to.
    ///
    /// `deriveInvocationExpressionArgument` reads an argument back as "the
    /// owned feature that redefines this input, and the value it holds", and
    /// `validateInvocationExpressionParameterRedefinition` holds every
    /// argument to redefining exactly one. The notation names none of them, so
    /// the order is what says which is which.
    fn hands_over_what_it_takes(&mut self, invocation: ElementId, function: ElementId) {
        let taken = self.taken_by(function, &mut Vec::new());
        let handed = self.takes(invocation);
        // An argument that names the parameter it is for says which one
        // it redefines; where none of them do, the order says it.
        let named: Vec<Option<ElementId>> = handed
            .iter()
            .map(|&argument| self.names_a_parameter(argument, function))
            .collect();
        let pairs: Vec<(ElementId, ElementId)> = match named.iter().all(Option::is_some) {
            true => handed
                .iter()
                .copied()
                .zip(named.into_iter().flatten())
                .collect(),
            false => handed.into_iter().zip(taken).collect(),
        };
        for (argument, input) in pairs {
            self.reified(
                argument,
                ElementKind::Redefinition,
                &[
                    ("redefiningFeature", Value::Ref(argument)),
                    ("redefinedFeature", Value::Ref(input)),
                    ("isImplied", Value::Bool(true)),
                ],
            );
            self.model
                .set(argument, "isImpliedIncluded", Value::Bool(true));
            // what it specializes was worked out before this was written
            self.supertypes.remove(&argument);
        }
    }
    /// An invocation comes to what the function it invokes hands back.
    ///
    /// The result an invocation owns redefines the function's own: `@Safety`
    /// is a boolean because `BaseFunctions::'@'` returns one, and nothing else
    /// in the model says so.
    ///
    /// `checkInvocationExpressionSpecialization` has the invocation specialize
    /// the function outright. Read that way it inherits the function's
    /// parameters too, and every constraint counting what an invocation is
    /// handed counts those beside them -- which the standard removes as
    /// redefined and this model does not.
    fn comes_to_what_it_invokes(&mut self, invocation: ElementId, function: ElementId) {
        let mine = self.hands_back(invocation).expect(
            "the builder gives every invocation the parameter it hands its value back through",
        );
        // `checkInvocationExpressionBehaviorResultSpecialization` --
        // where what is invoked is no function, what the invocation
        // comes to is the thing itself: `new A(...)` comes to an `A`,
        // and a classifier hands nothing back for the result to
        // redefine.
        let a_function = self.model.kind(function).is_a(ElementKind::Function)
            || self
                .supertypes_of(function)
                .into_iter()
                .any(|up| self.model.kind(up).is_a(ElementKind::Function));
        if !a_function {
            self.reified(
                mine,
                ElementKind::FeatureTyping,
                &[
                    ("typedFeature", Value::Ref(mine)),
                    ("type", Value::Ref(function)),
                    ("isImplied", Value::Bool(true)),
                ],
            );
            self.model.set(mine, "isImpliedIncluded", Value::Bool(true));
            self.supertypes.remove(&mine);
            return;
        }
        let Some(theirs) = self.handed_back_by(function, &mut Vec::new()) else {
            return;
        };
        self.reified(
            mine,
            ElementKind::Redefinition,
            &[
                ("redefiningFeature", Value::Ref(mine)),
                ("redefinedFeature", Value::Ref(theirs)),
                ("isImplied", Value::Bool(true)),
            ],
        );
        self.model.set(mine, "isImpliedIncluded", Value::Bool(true));
        // what it specializes was worked out before this was written
        self.supertypes.remove(&mine);
    }
    /// The parameter something hands its value back through.
    fn hands_back(&self, of: ElementId) -> Option<ElementId> {
        self.model
            .owned(of)
            .iter()
            .copied()
            .find(|&it| self.model.maybe(it, "direction") == Some(&Value::EnumLit("out")))
    }
    /// The one a function hands back, its own or the one it inherits.
    fn handed_back_by(
        &mut self,
        function: ElementId,
        seen: &mut Vec<ElementId>,
    ) -> Option<ElementId> {
        if seen.contains(&function) {
            return None;
        }
        seen.push(function);
        if let Some(own) = self.hands_back(function) {
            return Some(own);
        }
        for up in self.supertypes_of(function) {
            if let Some(found) = self.handed_back_by(up, seen) {
                return Some(found);
            }
        }
        None
    }
    /// The parameter an argument names itself for, where it names one.
    fn names_a_parameter(&mut self, argument: ElementId, function: ElementId) -> Option<ElementId> {
        let node = self.source.get(&argument)?.clone();
        let name = operand_segments(&node).pop()?;
        self.lookup(function, &name, Reach::all(Access::Internal))
    }
    /// What something is handed, in the order it is handed them.
    fn takes(&self, of: ElementId) -> Vec<ElementId> {
        self.model
            .owned(of)
            .iter()
            .copied()
            .filter(|&it| {
                matches!(
                    self.model.maybe(it, "direction"),
                    Some(Value::EnumLit("in") | Value::EnumLit("inout"))
                )
            })
            .collect()
    }
    /// What a function takes, its own or the ones it inherits.
    ///
    /// `calc getOutput` is a usage, and the parameters it is invoked
    /// with are declared on the calculation that defines it. Read off
    /// the usage alone there are none, and every argument then redefines
    /// nothing.
    fn taken_by(&mut self, function: ElementId, seen: &mut Vec<ElementId>) -> Vec<ElementId> {
        if seen.contains(&function) {
            return Vec::new();
        }
        seen.push(function);
        let own = self.takes(function);
        if !own.is_empty() {
            return own;
        }
        for up in self.supertypes_of(function) {
            let taken = self.taken_by(up, seen);
            if !taken.is_empty() {
                return taken;
            }
        }
        Vec::new()
    }
    /// The names a multiplicity counts with.
    ///
    /// `first [nCauses] causes.startShot ... { attribute nCauses =
    /// size(causes); }` counts with an attribute the succession declares, and
    /// `validateMultiplicityRangeBoundResultTypes` reads what such a bound
    /// comes to. The builder keeps a named bound as text; until the name is
    /// looked up it refers to nothing.
    pub(crate) fn count_with_what_is_named(&mut self, ids: &[ElementId]) {
        for &elem in ids {
            if self.model.kind(elem) != ElementKind::FeatureReferenceExpression
                || self.model.maybe(elem, "referent").is_some()
            {
                continue;
            }
            let Some(owner) = self
                .model
                .owner(elem)
                .filter(|&it| self.model.kind(it) == ElementKind::MultiplicityRange)
                .and_then(|range| self.model.owner(range))
            else {
                continue;
            };
            let written = self
                .written_as(elem)
                .expect("the builder keeps a named bound as the text it was written as");
            let segments: Vec<String> = written.split("::").map(str::to_string).collect();
            let found = self
                .resolve_inside(owner, &segments)
                .or_else(|| self.resolve_from(owner, &segments));
            if let Some(target) = found {
                self.refers_to(elem, target);
            }
        }
    }
    /// The text an element was written as, where the builder kept it.
    fn written_as(&self, elem: ElementId) -> Option<String> {
        self.model
            .owned(elem)
            .iter()
            .copied()
            .find(|&it| self.model.kind(it) == ElementKind::TextualRepresentation)
            .and_then(|it| self.model.maybe(it, "body"))
            .and_then(Value::as_str)
            .map(str::to_string)
    }
    /// Resolve every name written in an expression, from `owner`.
    ///
    /// The model keeps an expression as the text the author wrote, so the
    /// names in it are read off the syntax and looked up from the element it
    /// belongs to. That is the scope the language gives them: the constraint
    /// of a requirement sees the requirement's subject, the result of a `calc`
    /// its parameters.
    pub(crate) fn resolve_expression(
        &mut self,
        owner: ElementId,
        expr: &SyntaxNode,
        stats: &mut ResolveStats,
    ) {
        let file = self.elem_file.get(&owner).copied().unwrap_or(0);
        let mut chains = Vec::new();
        name_chains(expr, &mut chains);
        for chain in chains {
            let segments = operand_segments(&chain);
            let at = operand_ranges(&chain);
            let name_range = *at.last().expect("a name chain spells a name");
            let range = chain.text_range();
            match self.resolve_operand(owner, &segments) {
                Some(target) => {
                    stats.resolved += 1;
                    self.record(owner, file, range, name_range, &at, target);
                    // where the whole expression is that one name, the
                    // model reified it as a reference to a feature and
                    // this is the feature
                    if chain == *expr {
                        if let Some(reference) = self.reference_expression(owner) {
                            self.refers_to(reference, target);
                        }
                    }
                }
                None => {
                    self.record_miss(file, range, &segments, stats);
                }
            }
        }
    }
    /// The `FeatureReferenceExpression` an element's value was built
    /// into, where the model made one -- which it does exactly when the
    /// whole value is a name.
    pub(crate) fn reference_expression(&self, owner: ElementId) -> Option<ElementId> {
        let membership = self
            .model
            .owned(owner)
            .iter()
            .copied()
            .find(|&child| self.model.kind(child) == ElementKind::FeatureValue)?;
        let expression = self.model.maybe(membership, "value")?.as_id()?;
        (self.model.kind(expression) == ElementKind::FeatureReferenceExpression)
            .then_some(expression)
    }
    pub(crate) fn resolve_operand(
        &mut self,
        elem: ElementId,
        segments: &[String],
    ) -> Option<ElementId> {
        if !matches!(segments, [only] if only == "self" || only == "that") {
            return self.resolve_from(elem, segments);
        }
        // `self` and `that` are not looked up, so no qualified name was
        // walked to reach what they name.
        self.chain.clear();
        let mut scope = self.model.owner(elem);
        while let Some(current) = scope {
            if self.model.kind(current).is_a(ElementKind::Type) {
                return Some(current);
            }
            scope = self.model.owner(current);
        }
        None
    }
}

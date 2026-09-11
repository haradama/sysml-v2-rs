//! The relationships the notation leaves for the reader to infer, put into
//! the model.
//!
//! `connect a to b;` writes two ends and no redefinition, but the standard
//! says each end redefines the one its association declares, and a
//! constraint about `endFeature` is asked of a model that holds them. The
//! same goes for what an end participates in, the cross subsetting between
//! two ends, and the `result` a calculation returns without saying so.
//!
//! Nothing here reads a name that was written: each is a consequence of
//! what the model already says, materialised so that the constraints, the
//! interchange format and the diagram read it off rather than infer it
//! again.

use sysml_model::{ElementId, ElementKind, Role, Value};

use crate::syntax::*;
use crate::{implied_bases, reaches, Workspace, BINARY, OWNING_ENDS};

impl Workspace {
    /// The redefinition an end declared beside a supertype's implies.
    ///
    /// "If a Feature has isEnd = true and an owningType that is not empty,
    /// then, for each direct supertype of its owningType, it must redefine the
    /// endFeature at the same position, if any." Almost nothing writes it:
    /// `connect a to b` names no end at all. Read without it every such
    /// connector has four ends -- the two written and the two inherited -- and
    /// "a connector specializing a binary one is binary" is true of none of
    /// the eight hundred in the corpus.
    pub(crate) fn imply_end_redefinitions(&mut self) {
        for elem in self.model.ids().collect::<Vec<_>>() {
            let mine = self.own_ends(elem);
            if mine.is_empty() {
                continue;
            }
            let above: Vec<Vec<ElementId>> = self
                .supertypes_of(elem)
                .into_iter()
                .map(|up| self.ends_of(up))
                .collect();
            for (at, &end) in mine.iter().enumerate() {
                for other in above.iter().filter_map(|ends| ends.get(at).copied()) {
                    // What it already redefines it does not redefine
                    // again -- and only a redefinition counts, since
                    // only a redefinition stands in the place of what
                    // it names. `end feature transferSource references
                    // source` subsets the end it refers to and leaves
                    // it inherited beside itself.
                    if other == end || self.redefines(end, other) {
                        continue;
                    }
                    let redefinition = self.reified(
                        end,
                        ElementKind::Redefinition,
                        &[
                            ("redefiningFeature", Value::Ref(end)),
                            ("redefinedFeature", Value::Ref(other)),
                        ],
                    );
                    self.model.set(redefinition, "isImplied", Value::Bool(true));
                    self.model.set(end, "isImpliedIncluded", Value::Bool(true));
                    self.supertypes.clear();
                }
            }
        }
    }
    /// The library types an element specializes without saying so.
    ///
    /// What it specializes is not read off its metaclass alone: a
    /// connector or an association that relates more than two things is
    /// not a binary one, whatever keyword declared it.
    pub(crate) fn implied_bases_of(&mut self, elem: ElementId) -> Vec<&'static str> {
        let mut implied = implied_bases(self.model.kind(elem));
        // "numEnds != 2 ? base : binary" -- what a connector or an
        // association specializes without saying so is the binary
        // library type where it relates exactly two things and the
        // general one otherwise, counted over the ends it owns. A usage
        // that declares none of its own -- `interface i :
        // WheelHubInterface;` -- reaches whichever of the two its type
        // reached.
        let ends = self.own_ends(elem).len();
        if ends != 2 {
            implied.retain(|path| !BINARY.contains(path));
        }
        if ends == 0 {
            implied.retain(|path| !OWNING_ENDS.contains(path));
        }
        implied
    }
    /// Whether a feature redefines another, directly or through what it
    /// redefines in turn.
    fn redefines(&self, feature: ElementId, other: ElementId) -> bool {
        let mut queue = vec![feature];
        let mut seen = Vec::new();
        while let Some(at) = queue.pop() {
            if at == other {
                return true;
            }
            if seen.contains(&at) {
                continue;
            }
            seen.push(at);
            queue.extend(self.model.owned(at).iter().filter_map(|&owned| {
                match self.model.maybe(owned, "redefinedFeature") {
                    Some(Value::Ref(target)) => Some(*target),
                    _ => None,
                }
            }));
        }
        false
    }
    /// The end features a type declares itself, in the order it wrote
    /// them -- `ownedEndFeature`.
    fn own_ends(&self, elem: ElementId) -> Vec<ElementId> {
        self.model
            .owned(elem)
            .iter()
            .copied()
            .filter(|&it| self.model.flag(it, "isEnd"))
            .collect()
    }
    /// The end features a type has, its own or the ones it inherits.
    ///
    /// A type that declares no ends of its own stands for the ends of
    /// what it specializes -- `Connections::Connection` is reached
    /// through `BinaryConnection`, which is where the two ends are --
    /// so a position has to be looked for past a silent supertype
    /// rather than given up on there.
    fn ends_of(&mut self, elem: ElementId) -> Vec<ElementId> {
        let mut queue = vec![elem];
        let mut seen = Vec::new();
        let mut at = 0;
        while at < queue.len() {
            let up = queue[at];
            at += 1;
            if seen.contains(&up) {
                continue;
            }
            seen.push(up);
            let mine = self.own_ends(up);
            if !mine.is_empty() {
                return mine;
            }
            queue.extend(self.supertypes_of(up));
        }
        Vec::new()
    }
    /// Every end of an association or a connector subsets
    /// `Links::Link::participant`.
    ///
    /// "If a Feature has isEnd = true and an owningType that is an Association
    /// or a Connector, then it must directly or indirectly specialize
    /// `Links::Link::participant`", and the semantics section writes an N-ary
    /// association out with one `end feature eN[1..1] subsets
    /// Links::Link::participant;` per end.
    ///
    /// The first two ends reach it through the `source` and `target` they
    /// redefine. A third redefines nothing, because nothing above it has a
    /// third, and without this it has no supertype and so no type:
    /// `ConnectionTest.sysml`'s three-ended `abstract connection def C` was
    /// the one model in the corpus `validateAssociationEndTypes` reported, and
    /// it is sound.
    pub(crate) fn imply_end_participation(&mut self) {
        let Some(participant) = self.named_globally("Links::Link::participant") else {
            return;
        };
        for elem in self.model.ids().collect::<Vec<_>>() {
            let kind = self.model.kind(elem);
            if !kind.is_a(ElementKind::Association) && !kind.is_a(ElementKind::Connector) {
                continue;
            }
            for end in self.own_ends(elem) {
                if end == participant || self.reaches(end, participant) {
                    continue;
                }
                let subsetting = self.reified(
                    end,
                    ElementKind::Subsetting,
                    &[
                        ("subsettingFeature", Value::Ref(end)),
                        ("subsettedFeature", Value::Ref(participant)),
                    ],
                );
                self.model.set(subsetting, "isImplied", Value::Bool(true));
                self.model.set(end, "isImpliedIncluded", Value::Bool(true));
                self.supertypes.clear();
            }
        }
    }
    /// The subsetting an owned cross feature implies.
    ///
    /// "If this Feature is the ownedCrossFeature of an end Feature, then, for
    /// any end Feature that is redefined by the owning end Feature of this
    /// Feature, this Feature must subset the crossFeature of the redefined end
    /// Feature, if this exists." The association declares the cross feature and
    /// the redefinition and leaves what holds between them to the tool.
    pub(crate) fn imply_cross_subsettings(&mut self) {
        for elem in self.model.ids().collect::<Vec<_>>() {
            let Some(mine) = self.owned_cross_feature(elem) else {
                continue;
            };
            let redefined: Vec<ElementId> = self
                .model
                .owned(elem)
                .iter()
                .filter_map(|&it| match self.model.maybe(it, "redefinedFeature") {
                    Some(Value::Ref(target)) => Some(*target),
                    _ => None,
                })
                .collect();
            for up in redefined {
                let Some(theirs) = self.cross_feature(up) else {
                    continue;
                };
                if theirs == mine || reaches(&self.model, mine, theirs) {
                    continue;
                }
                let subsetting = self.reified(
                    mine,
                    ElementKind::Subsetting,
                    &[
                        ("subsettingFeature", Value::Ref(mine)),
                        ("subsettedFeature", Value::Ref(theirs)),
                    ],
                );
                self.model.set(subsetting, "isImplied", Value::Bool(true));
                self.model.set(mine, "isImpliedIncluded", Value::Bool(true));
                // what a feature specializes was worked out and
                // remembered while this pass was still deciding
                self.supertypes.clear();
            }
        }
    }
    /// The cross feature an end owns, where it wrote one.
    ///
    /// `ownedCrossFeature()` is "the first ownedMember of the Feature that is
    /// a Feature, but not a Multiplicity or a MetadataFeature, and whose
    /// owningMembership is not a FeatureMembership". The notation writes it
    /// two ways, and both are read here: `member feature inCart;` inside the
    /// end, and `end inCart[0..1] feature cart : ShoppingCart;` in front.
    fn owned_cross_feature(&self, elem: ElementId) -> Option<ElementId> {
        if !self.model.flag(elem, "isEnd") {
            return None;
        }
        self.model.owned(elem).iter().copied().find(|&it| {
            self.model.kind(it).is_a(ElementKind::Feature)
                && !self.model.kind(it).is_a(ElementKind::Multiplicity)
                && !self.model.kind(it).is_a(ElementKind::MetadataUsage)
                && self.source.get(&it).is_some_and(written_as_member)
        })
    }
    /// The cross feature of an end: the one it owns, or the second step
    /// of the chain its cross subsetting names.
    fn cross_feature(&self, elem: ElementId) -> Option<ElementId> {
        if let Some(owned) = self.owned_cross_feature(elem) {
            return Some(owned);
        }
        let crossed = self
            .model
            .owned(elem)
            .iter()
            .copied()
            .find(|&it| self.model.kind(it).is_a(ElementKind::CrossSubsetting))
            .and_then(|it| self.model.crossed_feature(it))?;
        // `crosses a.b` names the chain; a single name names no chain
        // at all, and the standard gives a cross feature nothing to be
        // the second step of
        self.model.chaining_feature(crossed).get(1).copied()
    }
    /// A feature that redefines an end is an end, and an end is not composite.
    ///
    /// `end` is written once: the corpus writes it on the outer feature and
    /// nests redefinitions without repeating the keyword.
    /// `validateRedefinitionEndConformance` holds a feature redefining an end
    /// to being one, and
    /// `validateFeatureEndNotDerivedAbstractCompositeOrPortion` holds an end
    /// to not being composite. Redefinitions are resolved by now, so this is
    /// where both can be said.
    pub(crate) fn carry_ends(&mut self) {
        loop {
            let mut carried = false;
            for elem in self.model.ids().collect::<Vec<_>>() {
                if self.model.flag(elem, "isEnd") {
                    continue;
                }
                let redefines_an_end = self.model.owned(elem).iter().any(|&owned| {
                    self.model.kind(owned).is_a(ElementKind::Redefinition)
                        && matches!(
                            self.model.maybe(owned, "redefinedFeature"),
                            Some(Value::Ref(up))
                                if self.model.flag(*up, "isEnd")
                        )
                });
                if redefines_an_end && self.model.kind(elem).feature("isEnd").is_some() {
                    self.model.set(elem, "isEnd", Value::Bool(true));
                    if self.model.kind(elem).feature("isComposite").is_some() {
                        self.model.set(elem, "isComposite", Value::Bool(false));
                    }
                    carried = true;
                }
            }
            // a redefinition of a redefinition of an end is one too
            if !carried {
                return;
            }
        }
    }
    /// Reify the implied specializations resolution reasons with, as the
    /// relationship elements the standard stores.
    ///
    /// Every definition and usage inherits from a semantic-library base -- a
    /// `part def` from `Parts::Part`, a feature from `Base::things` -- and
    /// resolution has always used those without materializing them. This pass
    /// writes each one the model does not already reach as an owned
    /// `Subclassification` or `Subsetting` with `isImplied` set, the way the
    /// standard interchanges them, and marks what gained one
    /// `isImpliedIncluded`.
    ///
    /// Call after [`resolve_all`](Workspace::resolve_all); bases that do not
    /// resolve are skipped, and running it again adds nothing. Returns how
    /// many relationships were written.
    pub fn materialize_implied(&mut self) -> usize {
        let mut written = 0;
        for elem in self.model.ids().collect::<Vec<_>>() {
            written += self.materialize_implied_for(elem);
        }
        written += self.imply_return_redefinitions();
        written
    }
    /// What one element specializes without saying so.
    ///
    /// `private calc getElapsedUtcTime { ... }` names no type, and what
    /// invokes it is held to invoking something typed by a behaviour --
    /// so the constraint reads a type the notation never wrote and the
    /// model has to hold.
    pub(crate) fn materialize_implied_for(&mut self, elem: ElementId) -> usize {
        let mut written = 0;
        {
            let kind = self.model.kind(elem);
            // relationships do not specialize; only types inherit
            if kind.is_a(ElementKind::Relationship) || !kind.is_a(ElementKind::Type) {
                return 0;
            }
            // "The specific Type of a Specialization cannot be a
            // conjugated Type" -- `validateSpecificationSpecificNot
            // Conjugated`. `class B conjugates A;` takes what it has
            // from the conjugation and specializes nothing, so writing
            // it a base of its own reports the model that wrote it:
            // `Conjugation.kerml` is one of the corpus's.
            if self
                .model
                .owned(elem)
                .iter()
                .any(|&child| self.model.kind(child).is_a(ElementKind::Conjugation))
            {
                return 0;
            }
            let mut bases = Vec::new();
            for path in self.implied_bases_of(elem) {
                let segments: Vec<String> = std::iter::once(String::new())
                    .chain(path.split("::").map(String::from))
                    .collect();
                if let Some(target) = self.resolve_from(elem, &segments) {
                    if target != elem && !bases.contains(&target) {
                        bases.push(target);
                    }
                }
            }
            // `#cause 'battery old' { ... }` implies the keyword's baseType
            for base in self.semantic_bases_of(elem) {
                if base != elem && !bases.contains(&base) {
                    bases.push(base);
                }
            }
            for base in bases {
                // implied only where nothing explicit -- or already
                // implied -- reaches the base; what this loop writes
                // counts for the bases after it. Nor does anything
                // specialize what specializes it: `Connections::
                // Connection` is a connection definition like any
                // other, and the base its metaclass names is
                // `BinaryConnection`, which specializes it.
                if reaches(&self.model, elem, base) || reaches(&self.model, base, elem) {
                    continue;
                }
                // a classifier subclassifies its base; a feature is
                // implicitly typed by a base classifier and subsets a
                // base feature
                let (kind, from, to) = if self.model.kind(elem).is_a(ElementKind::Classifier) {
                    (
                        ElementKind::Subclassification,
                        "subclassifier",
                        "superclassifier",
                    )
                } else if self.model.kind(base).is_a(ElementKind::Classifier) {
                    (ElementKind::FeatureTyping, "typedFeature", "type")
                } else {
                    (
                        ElementKind::Subsetting,
                        "subsettingFeature",
                        "subsettedFeature",
                    )
                };
                let relationship = self.model.create(kind);
                self.model.add_owned(elem, relationship);
                self.model.set(relationship, from, Value::Ref(elem));
                self.model.set(relationship, to, Value::Ref(base));
                self.model.set(relationship, "isImplied", Value::Bool(true));
                self.model.set(elem, "isImpliedIncluded", Value::Bool(true));
                written += 1;
            }
        }
        written
    }
    /// The redefinition a declared result parameter implies.
    ///
    /// The library writes no `redefines`, and the standard says it does not
    /// have to: a result parameter of a function that specializes another
    /// redefines that one's. Without it the specializing function has two
    /// result parameters, and "a function has exactly one" is true of none of
    /// the five hundred in the corpus that declare one.
    fn imply_return_redefinitions(&mut self) -> usize {
        let mut written = 0;
        for elem in self.model.ids().collect::<Vec<_>>() {
            let Some(result) = self.result_parameter(elem) else {
                continue;
            };
            // What it already says it redefines is what it redefines.
            if self
                .model
                .owned(result)
                .iter()
                .any(|&it| self.model.kind(it).is_a(ElementKind::Redefinition))
            {
                continue;
            }
            let Some(inherited) = self.inherited_result(elem, result) else {
                continue;
            };
            let redefinition = self.model.create(ElementKind::Redefinition);
            self.model.add_owned(result, redefinition);
            self.model
                .set(redefinition, "redefiningFeature", Value::Ref(result));
            self.model
                .set(redefinition, "redefinedFeature", Value::Ref(inherited));
            self.model.set(redefinition, "isImplied", Value::Bool(true));
            // `validateElementIsImpliedIncluded` -- what owns an implied
            // relationship says that it does
            self.model
                .set(result, "isImpliedIncluded", Value::Bool(true));
            written += 1;
        }
        written
    }
    /// The result parameter the nearest general type declares.
    ///
    /// A function may specialize one that declares no result of its own
    /// and inherits it in turn -- `LiteralBooleanEvaluation` through
    /// `BooleanEvaluation` -- so the walk carries on up rather than
    /// stopping where the first general type is silent.
    fn inherited_result(&mut self, elem: ElementId, mine: ElementId) -> Option<ElementId> {
        let mut queue = self.supertypes_of(elem);
        let mut seen = vec![elem];
        let mut at = 0;
        while at < queue.len() {
            let up = queue[at];
            at += 1;
            if seen.contains(&up) {
                continue;
            }
            seen.push(up);
            match self.result_parameter(up) {
                Some(result) if result != mine => return Some(result),
                _ => queue.extend(self.supertypes_of(up)),
            }
        }
        None
    }
    /// The one member a type declares with `return`, where it declares one.
    fn result_parameter(&self, elem: ElementId) -> Option<ElementId> {
        self.model
            .owned(elem)
            .iter()
            .copied()
            .find(|&it| self.model.member_role(it) == Some(Role::Return))
    }
}

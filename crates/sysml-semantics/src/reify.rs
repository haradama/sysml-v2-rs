//! Turning a resolved name into the relationship the model keeps.
//!
//! `part eng : Engine;` writes a typing, and the model holds it as an
//! owned `FeatureTyping` whose `type` is the element `Engine` resolved to.
//! Every declaration clause works that way, and so do the statements that
//! write their ends as plain names: `connect a to b`, `bind x = y`, `first
//! a then b`, `satisfy r by p`, `allocate one to other`.
//!
//! What is hard is not the reification but the ends: which operands are
//! ends at all, which the notation left for the neighbour to supply, and
//! what an end reached through a chain participates in.

use sysml_model::{ElementId, ElementKind, Value};
use sysml_syntax::{SyntaxKind, SyntaxNode};

use crate::syntax::*;
use crate::{Reached, Reference, ResolveStats, Workspace};

impl Workspace {
    /// Resolve the metadata definition an `@name { ... }` usage is typed
    /// by, and reify the typing so the settings inside the body resolve
    /// against the definition's attributes.
    pub(crate) fn resolve_metadata_typing(
        &mut self,
        usage: ElementId,
        node: &SyntaxNode,
        stats: &mut ResolveStats,
    ) {
        let Some(target) = metadata_target(node) else {
            return;
        };
        match self.resolve_from(usage, &target.segments) {
            Some(def) => {
                stats.resolved += 1;
                self.references.push(Reference {
                    file: self.elem_file.get(&usage).copied().unwrap_or(0),
                    range: target.range,
                    name_range: target.name_range,
                    target: def,
                });
                self.reify(usage, false, SyntaxKind::TYPING, def);
            }
            None => {
                self.record_miss(
                    self.elem_file.get(&usage).copied().unwrap_or(0),
                    target.range,
                    &target.segments,
                    stats,
                );
            }
        }
    }
    /// Put an element of `kind` with `props` under `owner`, taking the one an
    /// earlier pass made rather than making a second.
    ///
    /// Resolving the same file again is a thing callers do, and it does the
    /// same work over: made afresh each time, a feature would come to hold its
    /// type twice and everything reading the model would see it twice. What
    /// this pass has already taken is passed over, so `connect a to a`, which
    /// really does write one end twice, still gets two.
    pub(crate) fn reified(
        &mut self,
        owner: ElementId,
        kind: ElementKind,
        props: &[(&str, Value)],
    ) -> ElementId {
        let already = self.model.owned(owner).iter().copied().find(|&child| {
            self.model.kind(child) == kind
                && !self.claimed.contains(&child)
                && props
                    .iter()
                    .all(|(prop, value)| self.model.maybe(child, prop) == Some(value))
        });
        let made = match already {
            Some(child) => child,
            None => {
                let child = self.model.create(kind);
                self.model.add_owned(owner, child);
                for (prop, value) in props {
                    self.try_set(child, prop, value.clone());
                }
                child
            }
        };
        self.claimed.insert(made);
        made
    }
    /// Create the relationship element for one resolved target.
    pub(crate) fn reify(
        &mut self,
        elem: ElementId,
        is_definition: bool,
        part: SyntaxKind,
        target: ElementId,
    ) {
        let (kind, source_prop, target_prop) = match part {
            SyntaxKind::SUBSETTING if is_definition => (
                ElementKind::Subclassification,
                "subclassifier",
                "superclassifier",
            ),
            SyntaxKind::SUBSETTING => (
                ElementKind::Subsetting,
                "subsettingFeature",
                "subsettedFeature",
            ),
            SyntaxKind::REDEFINITION => (
                ElementKind::Redefinition,
                "redefiningFeature",
                "redefinedFeature",
            ),
            SyntaxKind::REFERENCES => (
                ElementKind::ReferenceSubsetting,
                "referencingFeature",
                "referencedFeature",
            ),
            // `datatype N :> V, A intersects V, A;` -- a type written as
            // the union, intersection or difference of others, and
            // `feature chain chains source.target` a feature written as
            // the chain through them
            SyntaxKind::UNIONS_KW => (ElementKind::Unioning, "typeUnioned", "unioningType"),
            SyntaxKind::INTERSECTS_KW => (
                ElementKind::Intersecting,
                "typeIntersected",
                "intersectingType",
            ),
            SyntaxKind::DIFFERENCES_KW => (
                ElementKind::Differencing,
                "typeDifferenced",
                "differencingType",
            ),
            SyntaxKind::CHAINS_KW => (
                ElementKind::FeatureChaining,
                "featureChained",
                "chainingFeature",
            ),
            // `feature h2 ... disjoint from h1;` -- what a feature is
            // declared not to overlap. Written this way there is no
            // statement to hang it off, and until it was read the name
            // after `from` was looked at by nothing: a typo in one was a
            // model this toolchain called sound.
            SyntaxKind::DISJOINT_KW => (ElementKind::Disjoining, "typeDisjoined", "disjoiningType"),
            // A cross subsetting is a subsetting, so the end that
            // declares it is its `subsettingFeature` like any other;
            // what it crosses to is the narrower name. Its
            // `crossingFeature` is derived from which feature owns it,
            // so nothing is stored for that.
            SyntaxKind::CROSSES_KW => (
                ElementKind::CrossSubsetting,
                "subsettingFeature",
                "crossedFeature",
            ),
            // `class B conjugates A;` -- the conjugation is owned by the
            // type that is conjugated, which is what tells it from
            // `conjugation c conjugate B conjugates A;`, where the
            // namespace owns it and B is not itself a conjugated type.
            SyntaxKind::CONJUGATES_KW => {
                (ElementKind::Conjugation, "conjugatedType", "originalType")
            }
            // `member feature inCart : ShoppingCart featured by
            // Product_Account;` -- what features a feature, written
            // beside the declaration rather than as a statement of its
            // own. Without it the feature is featured by whatever owns
            // it, which is what it says the feature is *not*.
            SyntaxKind::FEATURED_KW => {
                (ElementKind::TypeFeaturing, "featureOfType", "featuringType")
            }
            // relationship_parts only yields the kinds above plus TYPING
            _ => (ElementKind::FeatureTyping, "typedFeature", "type"),
        };
        self.reified(
            elem,
            kind,
            &[
                (source_prop, Value::Ref(elem)),
                (target_prop, Value::Ref(target)),
            ],
        );
    }
    /// What a dotted operand names: the chain, not the feature at the end.
    ///
    /// The published abstract syntax stands a `Feature` of its own for it,
    /// carrying each step as a `FeatureChaining` -- `Occurrences.kermlx` does
    /// exactly that -- because `b.f` is `f` of that `b` and not `f` wherever
    /// it is found. What features it is read off the chain, which is what
    /// makes the two ends of such a relationship differ at all.
    ///
    /// A single name is no chain: that is the feature the name reached.
    pub(crate) fn chained(
        &mut self,
        owner: ElementId,
        segments: &[String],
        steps: &[usize],
        target: ElementId,
    ) -> ElementId {
        if steps.len() < 2 {
            return target;
        }
        // the whole path already resolved, so every prefix of it does too
        let chain: Vec<ElementId> = steps
            .iter()
            .filter_map(|&depth| self.resolve_from(owner, &segments[..depth]))
            .collect();
        self.reified(
            owner,
            ElementKind::Feature,
            &[("chainingFeature", Value::RefList(chain))],
        )
    }
    /// The two types a relationship written as its own statement relates.
    ///
    /// `feature g :> f;` reifies a `Subsetting` under `g` and gives it both
    /// ends from the declaration it hangs off. `subset g subsets f;` says the
    /// same with no declaration to hang off: the `Subsetting` is what was
    /// written, and it reaches the model with neither end until the name
    /// before the clause and the name inside it are read here.
    /// Resolve one operand a statement wrote and keep what it landed on under
    /// `property`, or record that it landed on nothing.
    pub(crate) fn resolve_operand_into(
        &mut self,
        id: ElementId,
        operand: &SyntaxNode,
        property: &str,
        stats: &mut ResolveStats,
        chains: bool,
    ) {
        let file = self.elem_file.get(&id).copied().unwrap_or(0);
        let segments = operand_segments(operand);
        let range = operand.text_range();
        match self.resolve_written(id, &segments, false, None) {
            Some(target) => {
                stats.resolved += 1;
                let name_range = last_name_range(operand);
                self.record(file, range, name_range, &operand_ranges(operand), target);
                let reached = match chains {
                    true => self.chained(id, &segments, &operand_chain_steps(operand), target),
                    false => target,
                };
                self.model.set(id, property, Value::Ref(reached));
            }
            None => self.record_miss(file, range, &segments, stats),
        }
    }
    pub(crate) fn resolve_relation_ends(
        &mut self,
        id: ElementId,
        node: &SyntaxNode,
        stats: &mut ResolveStats,
    ) {
        let Some((keyword, source_prop, target_prop)) = relation_ends(self.model.kind(id)) else {
            return;
        };
        // `subset a.b subsets c.d;` writes a chain on either side, and a
        // subsetting is the one relationship the abstract syntax stands
        // a chain feature under. A specialization or a typing relates
        // types, and the published abstract syntax carries no chain
        // beneath either.
        let chains = self.model.kind(id).is_a(ElementKind::Subsetting)
            || self.model.kind(id) == ElementKind::Disjoining;
        let file = self.elem_file.get(&id).copied().unwrap_or(0);
        if let Some(operand) = operand_after(node, keyword) {
            self.resolve_operand_into(id, &operand, source_prop, stats, chains);
        }
        for (part, targets) in relationship_parts(node) {
            // What stands before `from` is what is disjoined, and what
            // stands after it is what it is disjoined from. `disjoint A
            // from B;` writes both bare; `disjoining d disjoint A from
            // B;` names the relationship first, which leaves `disjoint
            // A` a part of its own -- so the part is the end this one
            // relationship reads as its source rather than its target.
            let prop = match part == SyntaxKind::DISJOINT_KW {
                true => source_prop,
                false => target_prop,
            };
            for t in targets {
                match self.resolve_written(id, &t.segments, false, None) {
                    Some(target) => {
                        stats.resolved += 1;
                        self.record(file, t.range, t.name_range, &t.at, target);
                        let reached = match chains {
                            true => self.chained(id, &t.segments, &t.chain, target),
                            false => target,
                        };
                        self.model.set(id, prop, Value::Ref(reached));
                    }
                    None => self.record_miss(file, t.range, &t.segments, stats),
                }
            }
        }
        // and the end after `from` has no part of its own in either
        // shape: only the keyword standing before it says where it is.
        if self.model.kind(id) == ElementKind::Disjoining {
            if let Some(operand) = operand_after(node, SyntaxKind::FROM_KW) {
                self.resolve_operand_into(id, &operand, target_prop, stats, chains);
            }
        }
    }
    /// Resolve the operands of a `connect`/`bind`/`allocate` statement and
    /// record what they point at as the connector's `relatedFeature`s, so a
    /// consumer can read the connected ends off the model.
    pub(crate) fn resolve_connector_ends(
        &mut self,
        id: ElementId,
        node: &SyntaxNode,
        stats: &mut ResolveStats,
    ) {
        let file = self.elem_file.get(&id).copied().unwrap_or(0);
        // What the connector relates, in the order it relates them, and
        // how each of them was reached. The ends are reified once that
        // order is settled: `connectorEnd->at(1)` is the one it runs
        // from and `->at(2)` the one it runs to, and the notation lets
        // the first go unwritten.
        let mut related = Vec::new();
        let mut reached: Vec<Reached> = Vec::new();
        for operand in end_operands(node, self.model.kind(id)) {
            // an operand with no identifiers resolves to nothing, which the
            // `None` arm below reports like any other unresolved end
            let segments = operand_segments(&operand);
            let range = operand.text_range();
            let name_range = last_name_range(&operand);
            match self.resolve_from(id, &segments) {
                Some(target) => {
                    stats.resolved += 1;
                    self.record(file, range, name_range, &operand_ranges(&operand), target);
                    related.push(target);
                    reached.push(Reached::Written(segments, operand_chain_steps(&operand)));
                }
                None => {
                    self.record_miss(file, range, &segments, stats);
                }
            }
        }
        // `then action b;` writes no operand at all: what it flows into
        // is the declaration it wraps. That declaration belongs to the
        // enclosing scope rather than to the succession, so nothing an
        // operand search looks at holds it -- and a succession that
        // relates nothing is a step the model cannot say follows.
        let mut beside = None;
        if related.is_empty() && self.model.kind(id).is_a(ElementKind::ConnectorAsUsage) {
            beside = self.declared_beside(id, node);
            if let Some(target) = self.wrapped_declaration(id, node).or(beside) {
                related.push(target);
                reached.push(Reached::Beside(target));
            }
        }
        // A succession relates two things and the notation lets one go unwritten.
        // `then b;` says where the flow goes and not where it comes from; `first
        // start;` says the other. The answer is its neighbour in the same body.
        // Without it the model says a step follows nothing, and
        // `validateConnectorRelatedFeatures` is the specification saying so.
        if related.len() == 1 && self.model.kind(id).is_a(ElementKind::SuccessionAsUsage) {
            let written = |wanted: &[SyntaxKind]| {
                node.children_with_tokens()
                    .filter_map(|it| it.into_token())
                    .any(|token| wanted.contains(&token.kind()))
            };
            // `then message m of T from a to b;` writes the succession's
            // own end with its leading keyword alone: the `from` and the
            // `to` say where the flow it declares runs, and are not this
            // relationship's to read.
            let ends = match beside {
                Some(_) => (
                    &[SyntaxKind::FIRST_KW][..],
                    &[SyntaxKind::THEN_KW, SyntaxKind::ELSE_KW][..],
                ),
                None => (
                    &[SyntaxKind::FIRST_KW, SyntaxKind::FROM_KW][..],
                    &[SyntaxKind::THEN_KW, SyntaxKind::TO_KW, SyntaxKind::ELSE_KW][..],
                ),
            };
            // Only the source can be the missing one. `first a;` says
            // which step comes first and writes no flow at all --
            // `InitialNodeMember` rather than a succession -- so a
            // succession that names one end names the one it runs to.
            if written(ends.1) && !written(ends.0) {
                if let Some(source) = self.step_before(id) {
                    related.insert(0, source);
                    reached.insert(0, Reached::Beside(source));
                }
            }
        }
        // `accept Go then s2;` is a transition out of the state it is
        // written in. It says where it goes and not where it comes from,
        // and where it comes from is the state around it -- without
        // that, the succession it owns relates one thing, which
        // `validateConnectorRelatedFeatures` says a concrete connector
        // cannot do.
        if related.len() == 1
            && self.model.kind(id).is_a(ElementKind::TransitionUsage)
            && !has_leading(node, SyntaxKind::FIRST_KW)
        {
            // the state it follows, as a succession takes the step
            // before it; failing that the one it is written inside
            let leaves = self.step_before(id).or_else(|| {
                self.model
                    .owner(id)
                    .filter(|&owner| self.model.kind(owner).is_a(ElementKind::Step))
            });
            if let Some(state) = leaves {
                related.insert(0, state);
                reached.insert(0, Reached::Beside(state));
            }
        }
        // A transition relates its source and target through the
        // `Succession` it owns rather than by being a connector itself,
        // so that is where the two ends belong.
        let holder = self
            .model
            .owned(id)
            .iter()
            .copied()
            .find(|&child| self.model.kind(child).is_a(ElementKind::SuccessionAsUsage))
            .filter(|_| self.model.kind(id).is_a(ElementKind::TransitionUsage))
            .unwrap_or(id);
        for step in reached {
            match step {
                Reached::Written(segments, steps) => self.reify_end(holder, &segments, &steps),
                Reached::Beside(target) => self.end_reaching(holder, vec![target]),
            }
        }
        if !related.is_empty() {
            self.try_set(holder, "relatedFeature", Value::RefList(related));
        }
        // How many things it relates says which library type it
        // specializes, and the ends were not there to be counted when
        // anything asked earlier.
        self.supertypes.remove(&id);
    }
    /// What a succession runs from, where the statement left it unwritten: the
    /// nearest member of the same body before it that a succession can join.
    ///
    /// A step or an occurrence, since a sequence model writes `event
    /// occurrence e; then f;`. Where the nearest one is another succession the
    /// answer is the end of it facing this one: `then a; then b;` runs a to b.
    fn step_before(&self, succession: ElementId) -> Option<ElementId> {
        let owner = self.model.owner(succession)?;
        let members = self.model.owned(owner);
        let at = members.iter().position(|&it| it == succession)?;
        for &member in members[..at].iter().rev() {
            let kind = self.model.kind(member);
            if kind.is_a(ElementKind::ConnectorAsUsage) {
                // the one before this went somewhere, and that is where
                // this one starts
                if let Some(Value::RefList(related)) = self.model.maybe(member, "relatedFeature") {
                    return related.last().copied();
                }
                continue;
            }
            // `first x;` says which step comes first without owning
            // it, so what it names is what a `then` after it follows
            if kind.is_a(ElementKind::Membership) {
                if let Some(Value::Ref(named)) = self.model.get(member, "memberElement") {
                    return Some(*named);
                }
                continue;
            }
            if kind.is_a(ElementKind::Step) || kind.is_a(ElementKind::OccurrenceUsage) {
                return Some(member);
            }
        }
        None
    }
    /// What a comment says it is about.
    ///
    /// `comment about A, B /* ... */` names what it annotates, and the
    /// standard reifies one `Annotation` per name -- without them the
    /// comment says something about nothing in particular.
    pub(crate) fn resolve_annotation(
        &mut self,
        id: ElementId,
        node: &SyntaxNode,
        stats: &mut ResolveStats,
    ) {
        let file = self.elem_file.get(&id).copied().unwrap_or(0);
        let Some(about) = node
            .children()
            .find(|child| child.kind() == SyntaxKind::ABOUT)
        else {
            return;
        };
        for operand in about
            .children()
            .filter(|child| child.kind() == SyntaxKind::TYPE_REF)
        {
            for qname in operand
                .children()
                .filter(|child| child.kind() == SyntaxKind::QUALIFIED_NAME)
            {
                let segments = name_segments(&qname);
                let range = operand.text_range();
                match self.resolve_from(id, &segments) {
                    Some(target) => {
                        stats.resolved += 1;
                        // the earlier steps of `a::b::c` are references too,
                        // and a rename has to reach every one of them
                        let at = segment_ranges(&qname);
                        self.record(file, range, last_name_range(&operand), &at, target);
                        self.reified(
                            id,
                            ElementKind::Annotation,
                            &[
                                ("annotatingElement", Value::Ref(id)),
                                ("annotatedElement", Value::Ref(target)),
                            ],
                        );
                    }
                    None => {
                        self.record_miss(file, range, &segments, stats);
                    }
                }
            }
        }
    }
    /// What a `#Safety` prefix is typed by.
    pub(crate) fn resolve_prefix_metadata(
        &mut self,
        id: ElementId,
        node: &SyntaxNode,
        stats: &mut ResolveStats,
    ) {
        for qname in node
            .children()
            .filter(|child| child.kind() == SyntaxKind::QUALIFIED_NAME)
        {
            let segments = name_segments(&qname);
            if let Some(target) = self.resolve_from(id, &segments) {
                stats.resolved += 1;
                let file = self.elem_file.get(&id).copied().unwrap_or(0);
                let range = qname.text_range();
                let at = segment_ranges(&qname);
                self.record(file, range, last_name_range(&qname), &at, target);
                self.reify(id, false, SyntaxKind::TYPING, target);
            }
        }
    }
    /// The clients and suppliers of a `dependency a, b to c;`.
    ///
    /// `Dependency = 'dependency' ( Identification? 'from' )? client +=
    /// [QualifiedName] ( ',' client )* 'to' supplier += [QualifiedName]
    /// ( ',' supplier )*` -- so `to` divides the two, and a name before
    /// `from` is the dependency's own, not a client.
    pub(crate) fn resolve_dependency(
        &mut self,
        id: ElementId,
        node: &SyntaxNode,
        stats: &mut ResolveStats,
    ) {
        let file = self.elem_file.get(&id).copied().unwrap_or(0);
        let (mut clients, mut suppliers) = (Vec::new(), Vec::new());
        let mut supplying = false;
        for part in node.children_with_tokens() {
            if part.kind() == SyntaxKind::TO_KW {
                supplying = true;
                continue;
            }
            // `dependency Use from A to B` names itself before `from`;
            // without `from` there is no such name, and `dependency Z to
            // A` starts with a client
            let Some(operand) = part
                .into_node()
                .filter(|child| {
                    matches!(
                        child.kind(),
                        SyntaxKind::NAME_REF | SyntaxKind::PATH_EXPR | SyntaxKind::QUALIFIED_NAME
                    )
                })
                .filter(|operand| !before_from(node, operand))
            else {
                continue;
            };
            let segments = operand_segments(&operand);
            let range = operand.text_range();
            match self.resolve_from(id, &segments) {
                Some(target) => {
                    stats.resolved += 1;
                    let name_range = last_name_range(&operand);
                    self.record(file, range, name_range, &operand_ranges(&operand), target);
                    if supplying {
                        &mut suppliers
                    } else {
                        &mut clients
                    }
                    .push(target);
                }
                None => {
                    self.record_miss(file, range, &segments, stats);
                }
            }
        }
        if !clients.is_empty() {
            self.try_set(id, "client", Value::RefList(clients));
        }
        if !suppliers.is_empty() {
            self.try_set(id, "supplier", Value::RefList(suppliers));
        }
    }
    /// The declaration a control statement wraps, as the element the
    /// build hoisted it to.
    ///
    /// A wrapper keeps the name in the enclosing scope rather than one
    /// level in, so the declaration is a sibling of the statement --
    /// found by the syntax it was built from, which is the only thing
    /// that still tells the two apart.
    fn wrapped_declaration(&self, connector: ElementId, node: &SyntaxNode) -> Option<ElementId> {
        let declared = node
            .children()
            .find(|child| matches!(child.kind(), SyntaxKind::DEFINITION | SyntaxKind::USAGE))?;
        let owner = self.model.owner(connector)?;
        self.model
            .owned(owner)
            .iter()
            .copied()
            .find(|member| self.source.get(member) == Some(&declared))
    }
    /// What a `send`, an `accept` or an `assign` names.
    ///
    /// `send new S() via displayPort to screen;` says which port the message
    /// leaves by and who receives it, and `assign v := 1;` which feature it
    /// sets. None of the three was looked up at all, so the names stood for
    /// nothing -- and were not reported either, which is worse.
    ///
    /// The standard keeps the first two as arguments of the action, in the
    /// input parameters the builder laid out, and the last as a membership the
    /// assignment does not own.
    pub(crate) fn resolve_action_arguments(
        &mut self,
        id: ElementId,
        node: &SyntaxNode,
        stats: &mut ResolveStats,
    ) {
        let kind = self.model.kind(id);
        let arguments: &[(usize, SyntaxKind)] = match kind {
            ElementKind::SendActionUsage => &[(1, SyntaxKind::VIA_KW), (2, SyntaxKind::TO_KW)],
            ElementKind::AcceptActionUsage => &[(1, SyntaxKind::VIA_KW)],
            ElementKind::AssignmentActionUsage => &[],
            _ => return,
        };
        let file = self.elem_file.get(&id).copied().unwrap_or(0);
        let mut resolve = |ws: &mut Self, operand: SyntaxNode| {
            let segments = operand_segments(&operand);
            let at = operand_ranges(&operand);
            let name_range = *at.last().expect("an operand spells a name");
            match ws.resolve_operand(id, &segments) {
                Some(target) => {
                    stats.resolved += 1;
                    ws.record(file, operand.text_range(), name_range, &at, target);
                    Some(target)
                }
                None => {
                    ws.record_miss(file, operand.text_range(), &segments, stats);
                    None
                }
            }
        };
        // `assign v := 1;` refers to `v` without owning it, which is the
        // one membership of an assignment that is not an owning one
        if kind == ElementKind::AssignmentActionUsage {
            if let Some(target) = operand_after(node, SyntaxKind::ASSIGN_KW)
                .and_then(|operand| resolve(self, operand))
            {
                self.reified(
                    id,
                    ElementKind::Membership,
                    &[("memberElement", Value::Ref(target))],
                );
            }
            return;
        }
        let parameters: Vec<ElementId> = self
            .model
            .owned(id)
            .iter()
            .copied()
            .filter(|&child| self.model.kind(child) == ElementKind::ReferenceUsage)
            .collect();
        for &(slot, keyword) in arguments {
            let Some(operand) = operand_after(node, keyword) else {
                continue;
            };
            let Some(target) = resolve(self, operand) else {
                continue;
            };
            // the expression the builder made of it stands for that
            // feature, the way `= ledPinNumber` does
            if let Some(reference) = parameters
                .get(slot)
                .and_then(|&p| self.reference_expression(p))
            {
                self.refers_to(reference, target);
            }
        }
    }
    /// Record what a `FeatureReferenceExpression` stands for.
    ///
    /// `= ledPinNumber` refers to a feature without owning it, and the
    /// standard reads the referent back off the membership that says so.
    /// Holding the answer and not the membership leaves the expression
    /// referring to something by a route the specification does not have.
    pub(crate) fn refers_to(&mut self, reference: ElementId, target: ElementId) {
        self.try_set(reference, "referent", Value::Ref(target));
        // The builder stood the membership there ahead of the text the
        // expression was written as, since the standard takes the first
        // one. What it relates is only known once the name is looked up.
        let standing = self
            .model
            .owned(reference)
            .iter()
            .copied()
            .find(|&child| self.model.kind(child).is_a(ElementKind::Membership));
        // `attribute simpleUnitSelf : SimpleUnit = self;` -- `self` names
        // the feature every occurrence has of itself, and this model
        // resolves it to the type it is written in because that is the
        // scope it means. Which feature it stands for is not something
        // this can say, so the membership is left relating nothing
        // rather than relating a type where the standard has a feature.
        if let Some(membership) =
            standing.filter(|_| self.model.kind(target).is_a(ElementKind::Feature))
        {
            self.try_set(membership, "memberElement", Value::Ref(target));
            self.results_in_what_it_names(reference, target);
        }
    }
    /// What a feature reference comes to is what it names.
    ///
    /// `checkFeatureReferenceExpressionResultSpecialization` --
    /// "result.owningType() = self and result.specializes(referent)". What the
    /// parameter the builder gives the expression stands for is only known
    /// once the name is looked up, and without it `[n]` says nothing about
    /// what kind of thing `n` counts.
    fn results_in_what_it_names(&mut self, reference: ElementId, referent: ElementId) {
        let result = self
            .model
            .owned(reference)
            .iter()
            .copied()
            .find(|&child| self.model.member_role(child) == Some(sysml_model::Role::Return))
            .expect("the builder gives every feature reference the result it comes to");
        self.reified(
            result,
            ElementKind::Subsetting,
            &[
                ("subsettingFeature", Value::Ref(result)),
                ("subsettedFeature", Value::Ref(referent)),
                ("isImplied", Value::Bool(true)),
            ],
        );
        self.model
            .set(result, "isImpliedIncluded", Value::Bool(true));
        // an index expression asks what its argument comes to before
        // the argument has been looked up, and the answer it got then
        // must not stand for this one
        self.supertypes.remove(&result);
    }
    /// What the statement a succession was built from declares.
    ///
    /// `then merge continue;` writes the node and the succession as one
    /// statement, so the two elements share the one syntax node and the
    /// declaration is the sibling that node also became. So what is looked for
    /// is what the statement declared, never the succession beside it.
    fn declared_beside(&self, succession: ElementId, node: &SyntaxNode) -> Option<ElementId> {
        let owner = self.model.owner(succession)?;
        self.model.owned(owner).iter().copied().find(|&member| {
            member != succession
                && !self.model.kind(member).is_a(ElementKind::SuccessionAsUsage)
                && self.source.get(&member) == Some(node)
        })
    }
    /// Reify one connector end as a `Feature` whose `chainingFeature` holds
    /// what each segment of the operand resolved to.
    ///
    /// The final target alone cannot say which part an end belongs to --
    /// `w1.hub` and `w2.hub` resolve to the same port of the same type --
    /// so the chain is what an interconnection view needs.
    fn reify_end(&mut self, connector: ElementId, segments: &[String], steps: &[usize]) {
        let mut chain = Vec::new();
        // the full path already resolved, so every prefix normally does too
        for &depth in steps {
            if let Some(step) = self.resolve_from(connector, &segments[..depth]) {
                chain.push(step);
            }
        }
        self.end_reaching(connector, chain);
    }
    /// Stand a `Feature` for one connector end, reaching what it names.
    ///
    /// One name is not a chain: `validateFeatureChainingFeatureNotOne`
    /// gives a feature either no chaining features or more than one, so
    /// an end naming a single feature refers to it through a subsetting
    /// instead. Read either way by [`sysml_model::end_reaches`].
    fn end_reaching(&mut self, connector: ElementId, chain: Vec<ElementId>) {
        // What a connector relates it relates through ends of its own:
        // `EndFeatureMembership` is how the standard owns one, which is also what
        // tells such a feature from a member the source wrote as a reference.
        // An end, and only an end: a connector may own a feature the source wrote
        // -- `connector ps : P ([1] myCart, ...)` -- and taking one of those for
        // an end would give it a second multiplicity and the connector a third
        // thing to relate.
        // A flow relates its ends through `FlowEnd`s, where the last step of what
        // one names is not part of the path but the thing that flows.
        let flowing = self.model.kind(connector).is_a(ElementKind::Flow);
        let end = self.reified(
            connector,
            match flowing {
                true => ElementKind::FlowEnd,
                false => ElementKind::Feature,
            },
            &[("isEnd", Value::Bool(true))],
        );
        self.counts_one(end);
        let chain = match flowing {
            // an end is reified only for an operand that resolved, so
            // there is always a last step to be the thing that flows
            true => {
                let (&flows, path) = chain.split_last().expect("an end names something");
                let path = path.to_vec();
                self.flowing_feature(end, flows);
                path
            }
            false => chain,
        };
        match chain.as_slice() {
            // One name is not a chain: the standard gives a feature
            // either no chaining features or more than one, so an end
            // naming a single feature refers to it instead.
            [only] => {
                let only = *only;
                self.reified(
                    end,
                    ElementKind::ReferenceSubsetting,
                    &[
                        ("referencingFeature", Value::Ref(end)),
                        ("referencedFeature", Value::Ref(only)),
                    ],
                );
            }
            _ => self.try_set(end, "chainingFeature", Value::RefList(chain)),
        }
    }
    /// Stand the last step of what a flow end names up as the feature that
    /// flows through it.
    ///
    /// `flow from tank.fuelOut to engine.fuelIn` runs from `tank`, and what
    /// flows is `fuelOut`. The standard keeps them apart -- the path is what
    /// the end refers to, the feature is the one thing it owns -- and
    /// [`sysml_model::end_reaches`] puts the two back together.
    fn flowing_feature(&mut self, end: ElementId, flows: ElementId) {
        let feature = self.reified(end, ElementKind::Feature, &[]);
        self.reified(
            feature,
            ElementKind::Redefinition,
            &[
                ("redefiningFeature", Value::Ref(feature)),
                ("redefinedFeature", Value::Ref(flows)),
            ],
        );
        // The standard says so in a note beside the grammar: "to ensure
        // that a FlowFeature passes the
        // validateRedefinitionDirectionConformance constraint, its
        // direction must be set to the direction of its
        // redefinedFeature". Written nowhere, `out fuelOut` flowed into
        // a feature with no direction at all, and the two are then not
        // the same way round.
        if let Some(direction) = self.model.maybe(flows, "direction").cloned() {
            self.try_set(feature, "direction", direction);
        }
    }
    /// An end is one thing.
    ///
    /// `validateFeatureEndMultiplicity` -- "if a Feature has isEnd = true,
    /// then it must have multiplicity 1..1" -- and the notation writes it
    /// nowhere. What stands before an end in `first [0..1] decide then [0..1]
    /// merge` is the cross multiplicity: how many at the far end go with one
    /// here. Without the range, the four constraints counting what a control
    /// node is joined by have nothing to read.
    fn counts_one(&mut self, end: ElementId) {
        let range = self.reified(end, ElementKind::MultiplicityRange, &[]);
        if self.model.get(range, "upperBound").is_none() {
            let mut one = || {
                let bound = self.model.create(ElementKind::LiteralInteger);
                self.model.add_owned(range, bound);
                self.model.set(bound, "value", Value::Int(1));
                bound
            };
            let (lower, upper) = (one(), one());
            self.model.set(range, "lowerBound", Value::Ref(lower));
            self.model.set(range, "upperBound", Value::Ref(upper));
        }
        self.try_set(end, "multiplicity", Value::Ref(range));
    }
    /// A transition's `accept x : T` writes a typing that belongs to the
    /// trigger it declares, not to the transition itself.
    pub(crate) fn resolve_trigger_type(
        &mut self,
        transition: ElementId,
        node: &SyntaxNode,
        stats: &mut ResolveStats,
    ) {
        let declared = match self.model.maybe(transition, "triggerAction") {
            Some(Value::RefList(triggers)) => triggers.first().copied(),
            // `accept cl : CallGiveItems do action { ... }` is an accept
            // node rather than a transition, so what it waits for is its
            // own payload parameter instead of a trigger. The type is
            // written after the payload's name and belongs to it either
            // way: without it `cl.itms` names nothing.
            _ if self
                .model
                .kind(transition)
                .is_a(ElementKind::AcceptActionUsage) =>
            {
                Some(transition)
            }
            _ => None,
        };
        // `deriveAcceptActionUsagePayloadParameter` -- "the
        // payloadParameter of an AcceptActionUsage is its first
        // parameter", and the type written after the payload's name
        // belongs to it rather than to the node that waits for it.
        let Some(trigger) = declared.and_then(|it| sysml_model::payload_parameter(&self.model, it))
        else {
            return;
        };
        let file = self.elem_file.get(&transition).copied().unwrap_or(0);
        let typings = relationship_parts(node)
            .into_iter()
            .filter(|(part, _)| *part == SyntaxKind::TYPING);
        for (_, targets) in typings {
            for t in targets {
                match self.resolve_written(transition, &t.segments, false, None) {
                    Some(target) => {
                        stats.resolved += 1;
                        self.record(file, t.range, t.name_range, &t.at, target);
                        self.reified(
                            trigger,
                            ElementKind::FeatureTyping,
                            &[
                                ("typedFeature", Value::Ref(trigger)),
                                ("type", Value::Ref(target)),
                            ],
                        );
                    }
                    None => {
                        self.record_miss(file, t.range, &t.segments, stats);
                    }
                }
            }
        }
    }
    /// Resolve what `satisfy r by p;` relates: the requirement named after
    /// `satisfy` and the feature named after `by`.
    ///
    /// `satisfy requirement r : R by p;` declares the requirement inline, so
    /// only the `by` side is a reference there.
    /// Resolve an operand that may be the implicit `self` or `that` rather
    /// than a declared name.
    pub(crate) fn resolve_satisfaction(
        &mut self,
        id: ElementId,
        node: &SyntaxNode,
        stats: &mut ResolveStats,
    ) {
        let file = self.elem_file.get(&id).copied().unwrap_or(0);
        for (keyword, property) in [
            (SyntaxKind::SATISFY_KW, "satisfiedRequirement"),
            (SyntaxKind::BY_KW, "satisfyingFeature"),
        ] {
            let Some(operand) = operand_after(node, keyword) else {
                // `satisfy requirement req1 : Req1 by system;` declares
                // the satisfaction rather than writing the requirement
                // after the keyword, and names what is satisfied by
                // typing it. That typing is reified by now, so the
                // answer is already in the model. Where the keyword is
                // there but the typing is not, nothing is satisfied.
                if property == "satisfiedRequirement" {
                    if let Some(typed) = self.model.type_of(id) {
                        self.try_set(id, property, Value::Ref(typed));
                    }
                }
                continue;
            };
            let segments = operand_segments(&operand);
            let range = operand.text_range();
            match self.resolve_operand(id, &segments) {
                Some(target) => {
                    stats.resolved += 1;
                    self.record(file, range, range, &operand_ranges(&operand), target);
                    self.try_set(id, property, Value::Ref(target));
                }
                None => {
                    self.record_miss(file, range, &segments, stats);
                }
            }
        }
    }
    /// Resolve what `verify r;` names, and hang it on the case.
    ///
    /// The requirement a verification case answers for is written inside
    /// its objective, several levels down from the case itself, and
    /// `verifiedRequirement` is the case's own property. Recording it
    /// there is what lets a reader of the model ask what verifies a
    /// requirement without walking back down through the objective.
    pub(crate) fn resolve_verification(
        &mut self,
        id: ElementId,
        node: &SyntaxNode,
        stats: &mut ResolveStats,
    ) {
        let Some(operand) = operand_after(node, SyntaxKind::VERIFY_KW) else {
            return;
        };
        let file = self.elem_file.get(&id).copied().unwrap_or(0);
        let segments = operand_segments(&operand);
        let range = operand.text_range();
        let Some(target) = self.resolve_operand(id, &segments) else {
            self.record_miss(file, range, &segments, stats);
            return;
        };
        stats.resolved += 1;
        self.record(file, range, range, &operand_ranges(&operand), target);

        let mut scope = self.model.owner(id);
        while let Some(current) = scope {
            if matches!(
                self.model.kind(current),
                ElementKind::VerificationCaseDefinition | ElementKind::VerificationCaseUsage
            ) {
                let mut verified = match self.model.maybe(current, "verifiedRequirement") {
                    Some(Value::RefList(already)) => already.clone(),
                    _ => Vec::new(),
                };
                if !verified.contains(&target) {
                    verified.push(target);
                }
                self.try_set(current, "verifiedRequirement", Value::RefList(verified));
                return;
            }
            scope = self.model.owner(current);
        }
    }
    pub(crate) fn try_set(&mut self, id: ElementId, prop: &str, value: Value) {
        if self.model.kind(id).feature(prop).is_some() {
            self.model.set(id, prop, value);
        }
    }
}

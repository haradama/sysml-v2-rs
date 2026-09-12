//! What gets resolved, in what order, and what is thrown away first.
//!
//! One pass over the elements asked for: each declaration's clauses are
//! looked up and reified, then the passes that need the whole model
//! standing -- the names inside expressions, the ends a statement named,
//! the relationships the notation implies.
//!
//! The order is not arbitrary. A feature chain cannot be walked until the
//! typings it steps through are reified; an end cannot redefine the one
//! its association declares until the association's supertypes are known.
//! And resolving one file again must forget what that file said last time
//! without forgetting its neighbours, or a workspace an editor keeps open
//! grows a second copy of every relationship.

use std::collections::HashSet;

use sysml_model::{ElementId, ElementKind, Role, Value};
use sysml_syntax::SyntaxKind;

use crate::syntax::*;
use crate::{Clear, ResolveStats, Workspace};

impl Workspace {
    /// Forget what was worked out from names that were not there.
    ///
    /// A workspace grows a file at a time: an editor opens a buffer over a
    /// project already loaded, a project loads its library after the file
    /// being edited. A lookup that failed before the file arrived is no
    /// evidence about the workspace now, and remembering it is how a language
    /// server underlines a name the model does resolve. What was found stands
    /// -- a file only adds names.
    pub(crate) fn forget_failures(&mut self) {
        self.imports.retain(|_, target| target.is_some());
        self.aliases.retain(|_, target| target.is_some());
        for id in std::mem::take(&mut self.incomplete) {
            self.supertypes.remove(&id);
            self.semantic_bases.remove(&id);
        }
        // The new file's members are members of the root namespace, and
        // its own namespaces have none indexed yet. An index holds a
        // namespace's own members and nothing else -- imports and
        // inheritance are looked through at lookup time -- so the root's
        // is the only one the file made stale, and every other index
        // dropped here was rebuilt, unchanged, under the next lookup.
        self.members.remove(&self.root);
        // And a name that answered to nothing from the root answered
        // against the files there were then. The one just added may be
        // the file it lives in -- which is how a workspace that asked
        // `Base::Anything` before its library arrived went on saying it
        // had no standard library afterwards, and so never put the
        // specification's constraints to a model that could have
        // answered them.
        self.globals.retain(|_, found| found.is_some());
    }
    /// Resolve every explicit relationship target in the workspace and
    /// reify the relationship elements.
    pub fn resolve_all(&mut self) -> ResolveStats {
        let ids: Vec<ElementId> = self.model.ids().collect();
        self.resolve_ids(&ids, Clear::TheseFiles)
    }
    /// Resolve `files`, and then whatever they turned out to reach, until
    /// nothing new is reached.
    ///
    /// A reader of the model -- a drawing, a generator -- follows the
    /// relationships resolution reifies, so a type that was never resolved has
    /// no members to show and no supertype to inherit from. Resolving every
    /// loaded file answers that by doing far more work than the question
    /// needs: a standard library is thousands of references, of which a model
    /// uses a handful.
    pub fn resolve_reached(&mut self, files: &[usize]) -> ResolveStats {
        let mut stats = self.resolve_files(files);
        let mut done: HashSet<ElementId> = self.elements_in(files).into_iter().collect();
        loop {
            let mut fresh = Vec::new();
            for reference in &self.references {
                // reached from what this call resolved, and not into a
                // file already resolved whole: the language server
                // stands its documents on a resolved library, and a
                // reach that re-entered it resolved most of it again
                // and recorded every reference in it a second time
                if !done.contains(&reference.from) || done.contains(&reference.target) {
                    continue;
                }
                let elsewhere = self.elem_file.get(&reference.target).copied();
                if elsewhere.is_some_and(|file| self.settled.contains(&file)) {
                    continue;
                }
                // A package is a place to look names up, not something
                // a model is made of: nothing is typed by one, and a
                // qualified name records every namespace it passed
                // through on the way. What is actually used out of a
                // package records itself, so following the package
                // would be reading a library to find two words in it.
                if self.model.kind(reference.target).is_a(ElementKind::Package) {
                    continue;
                }
                // what a name landed on is of no use without what it
                // holds: the members a box would show, and the types
                // those are declared with
                fresh.extend(self.model.descendants(reference.target));
            }
            fresh.retain(|id| done.insert(*id));
            if fresh.is_empty() {
                // once more over everything reached: what a round implied
                // about an element was implied before the round after it
                // resolved the supertypes that implication reads
                let reached: Vec<ElementId> = done.into_iter().collect();
                self.imply_for(&reached);
                return stats;
            }
            // every element is resolved once, so nothing said about one
            // round is there to be replaced by the next
            let round = self.resolve_ids(&fresh, Clear::Nothing);
            stats.resolved += round.resolved;
            stats.unresolved += round.unresolved;
            stats.lookups += round.lookups;
        }
    }
    /// What resolution implies about `ids`: the ends carried down from
    /// what they redefine, the named ends related, the redefinitions,
    /// participations and cross-subsettings the specification says
    /// follow. Over the elements just resolved and no more -- each of
    /// these walked every element of the workspace on every call, which
    /// made the two references of a document being typed into cost a
    /// walk over the whole library it stood on.
    pub(crate) fn imply_for(&mut self, ids: &[ElementId]) {
        let ids = self.with_what_they_own(ids);
        self.carry_ends(&ids);
        self.count_with_what_is_named(&ids);
        self.relate_named_ends(&ids);
        self.imply_end_redefinitions(&ids);
        self.imply_end_participation(&ids);
        self.imply_cross_subsettings(&ids);
    }
    /// `ids` and everything under them. Resolving an element reifies
    /// what its text implies -- a transition's ends, a typing -- as
    /// elements it owns, which no file lists and which want what follows
    /// resolution as much as what was written down. Walked from the
    /// elements that own nothing else in `ids`, so a whole workspace is
    /// one walk rather than one per element.
    fn with_what_they_own(&self, ids: &[ElementId]) -> Vec<ElementId> {
        let given: HashSet<ElementId> = ids.iter().copied().collect();
        let mut stack: Vec<ElementId> = ids
            .iter()
            .copied()
            .filter(|&id| {
                self.model
                    .owner(id)
                    .is_none_or(|owner| !given.contains(&owner))
            })
            .collect();
        let mut seen: HashSet<ElementId> = HashSet::new();
        let mut all = Vec::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            all.push(id);
            stack.extend(self.model.owned(id).iter().copied());
        }
        all
    }
    /// Every element of `files`, from each file's own list rather than
    /// by filtering every element of the workspace by its file.
    fn elements_in(&self, files: &[usize]) -> Vec<ElementId> {
        files
            .iter()
            .flat_map(|&file| self.files[file].elements.iter().copied())
            .collect()
    }
    /// Resolve everything in `files`, forgetting what those files said
    /// last time and leaving what their neighbours said standing.
    ///
    /// This is what an editor calls when a buffer changes: the library
    /// underneath is resolved once and never again.
    pub fn resolve_files(&mut self, files: &[usize]) -> ResolveStats {
        let ids = self.elements_in(files);
        self.resolve_ids(&ids, Clear::TheseFiles)
    }
    fn resolve_ids(&mut self, ids: &[ElementId], clear: Clear) -> ResolveStats {
        // Resolving the same elements again replaces what was found
        // about them rather than adding to it: asking twice is a thing
        // callers do, and it should not double every finding. A caller
        // that resolves each element exactly once says so instead, so
        // that one round does not wipe what the last one found.
        if clear == Clear::TheseFiles {
            let touched: HashSet<usize> = ids
                .iter()
                .filter_map(|id| self.elem_file.get(id).copied())
                .collect();
            self.unresolved.retain(|u| !touched.contains(&u.file));
            self.references.retain(|r| !touched.contains(&r.file));
            // every element of these files is about to be resolved
            self.settled.extend(touched);
        }

        let mut stats = ResolveStats::default();
        let began = self.lookups;
        self.claimed.clear();
        for &id in ids {
            let Some(node) = self.source.get(&id).cloned() else {
                continue;
            };
            // `first x;` names which step comes first: a membership
            // whose member is written elsewhere, the way an alias's is.
            if node.kind() == SyntaxKind::CONTROL_STMT
                && self.model.kind(id).is_a(ElementKind::Membership)
            {
                if let Some(operand) = operand_after(&node, SyntaxKind::FIRST_KW) {
                    self.resolve_operand_into(id, &operand, "memberElement", &mut stats, false);
                }
                continue;
            }
            if matches!(
                node.kind(),
                SyntaxKind::CONNECTOR_STMT | SyntaxKind::CONTROL_STMT
            ) {
                self.resolve_connector_ends(id, &node, &mut stats);
                self.resolve_trigger_type(id, &node, &mut stats);
                self.resolve_action_arguments(id, &node, &mut stats);
                continue;
            }
            // `comment about A, B /* ... */` and `metadata m : M about
            // A` both say what they are about, and the names they say it
            // about are references like any other
            self.resolve_annotation(id, &node, &mut stats);
            if matches!(
                node.kind(),
                SyntaxKind::COMMENT_ELEM | SyntaxKind::DOCUMENTATION | SyntaxKind::REP
            ) {
                continue;
            }
            // `#Safety part def Boiler;` -- the prefix is a metadata
            // usage typed by what it names, and the typing is written as
            // a bare qualified name rather than a typing clause
            if node.kind() == SyntaxKind::PREFIX_METADATA {
                self.resolve_prefix_metadata(id, &node, &mut stats);
                continue;
            }
            // `dependency use from A to B;` is a statement of its own,
            // and the names on either side of `to` are references like
            // any other
            if node.kind() == SyntaxKind::DEPENDENCY {
                self.resolve_dependency(id, &node, &mut stats);
                continue;
            }
            // `@code { ... }` types the metadata usage by its metadata
            // definition; resolving it is what lets the `:>> attribute`
            // settings inside reach the definition's attributes
            if node.kind() == SyntaxKind::METADATA_ANNOTATION {
                self.resolve_metadata_typing(id, &node, &mut stats);
                continue;
            }
            // `import P1::*;` names P1, and an editor renaming P1 has to
            // be told so -- otherwise the rename leaves the import
            // pointing at a package that is no longer there.
            if matches!(node.kind(), SyntaxKind::IMPORT | SyntaxKind::EXPOSE) {
                self.record_import(id, &node, &mut stats);
                continue;
            }
            // `alias Q for P;` is a membership whose member is the
            // element it renames. Nothing else asks for it -- a name
            // reached through the alias resolves to what it stands for
            // and forgets the way in -- so a reader of the model alone
            // would find an alias that names nothing.
            if node.kind() == SyntaxKind::ALIAS {
                self.record_alias(id, &node, &mut stats);
                continue;
            }
            // `mass * speed` ending a calculation body, or the body of
            // `require constraint { ... }`: an expression standing on
            // its own, whose names are as much references as a typing's
            if node.kind() == SyntaxKind::EXPR_STMT {
                if let Some(written) = node.children().next() {
                    self.resolve_expression(id, &written, &mut stats);
                }
                continue;
            }
            // `subset g.g subsets b.f.a;` writes as a statement of its
            // own what `feature g :> f` writes as a clause. Written as a
            // clause the declaration is the element and the
            // relationship is reified under it; written as a statement
            // the relationship *is* the element, and both of the things
            // it relates are names on it that nothing else reads.
            if node.kind() == SyntaxKind::RELATION_STMT {
                self.resolve_relation_ends(id, &node, &mut stats);
                continue;
            }
            // a payload carries a typing of its own -- `flow f of Fuel`
            // -- and is an element the builder made, so it resolves like
            // any declaration
            if !matches!(
                node.kind(),
                SyntaxKind::DEFINITION | SyntaxKind::USAGE | SyntaxKind::PAYLOAD
            ) {
                continue;
            }
            // `action initialization assign index := 1;` writes the
            // action's own name before the keyword, so the statement
            // parses as a usage rather than as a control statement --
            // and what it assigns is a name to look up either way
            self.resolve_action_arguments(id, &node, &mut stats);
            // `attribute pin : PinNumber = ledPinNumber;` -- the value is
            // an expression like any other, and the name in it is a
            // reference like any other. `binding a = b;` writes the same
            // `=` and means something else by it: what follows is the
            // second end, which the ends are read from instead.
            for clause in node
                .children()
                .filter(|_| !binds_an_end(&node))
                .filter(|child| child.kind() == SyntaxKind::VALUE)
            {
                for written in clause
                    .children()
                    .filter(|child| child.kind() != SyntaxKind::BODY)
                {
                    self.resolve_expression(id, &written, &mut stats);
                }
            }
            let is_definition = node.kind() == SyntaxKind::DEFINITION;
            for (part_kind, targets) in relationship_parts(&node) {
                for t in targets {
                    // `member step merge ... featured by
                    // TakePicture_snapshots { member feature
                    // TakePicture_snapshots ... }` -- what features a
                    // feature may be declared inside it, and the
                    // featuring is written inside it too, so the name is
                    // read from there before it is read from around.
                    let found = match part_kind {
                        SyntaxKind::FEATURED_KW => self.resolve_inside(id, &t.segments),
                        _ => None,
                    };
                    let redefining = std::mem::replace(
                        &mut self.redefining,
                        part_kind == SyntaxKind::REDEFINITION,
                    );
                    let found = found.or_else(|| {
                        self.resolve_written(
                            id,
                            &t.segments,
                            may_name_itself(part_kind, is_definition),
                            expected_kind(part_kind, is_definition),
                        )
                    });
                    self.redefining = redefining;
                    match found {
                        Some(target) => {
                            stats.resolved += 1;
                            let file = self.elem_file.get(&id).copied().unwrap_or(0);
                            self.record(id, file, t.range, t.name_range, &t.at, target);
                            // `chains source.target` names the steps of
                            // one chain, and each step is a chaining of
                            // its own: read as a single relationship the
                            // feature comes to have one chaining
                            // feature, which the standard does not
                            // allow it.
                            if part_kind == SyntaxKind::CHAINS_KW {
                                for depth in 1..=t.segments.len() {
                                    if let Some(step) = self.resolve_from(id, &t.segments[..depth])
                                    {
                                        self.reify(id, is_definition, part_kind, step);
                                    }
                                }
                                continue;
                            }
                            // A dotted operand names a chain, not the feature at the end of it. The
                            // published abstract syntax makes a `Feature` of its own of it, carrying
                            // the steps as `FeatureChaining` -- `Occurrences.kermlx` does exactly
                            // that. Read as the last step alone, the operand is the feature that step
                            // names anywhere rather than the one this path reaches, and what features
                            // it is read off the wrong element -- which is the whole of what
                            // `validateSubsettingFeaturingTypes` and
                            // `validateRedefinitionFeaturingTypes` ask.
                            //
                            // A reference subsetting is one too -- 106 of them under the standard
                            // library alone -- and a connector end written `lcp ::> w.lcp` reaches
                            // what it relates through the chain. For a cross subsetting there is
                            // more: `deriveFeatureCrossFeature` reads
                            // `crossedFeature.chainingFeature->at(2)`, and
                            // `validateCrossSubsettingCrossedFeature` holds the first step to being
                            // the other end of the association.
                            let names_a_chain = matches!(
                                part_kind,
                                SyntaxKind::CROSSES_KW
                                    | SyntaxKind::REDEFINITION
                                    | SyntaxKind::REFERENCES
                                    | SyntaxKind::DISJOINT_KW
                            ) || part_kind == SyntaxKind::SUBSETTING
                                && !is_definition;
                            let reached = match names_a_chain {
                                true => self.chained(id, &t.segments, &t.chain, target),
                                false => target,
                            };
                            self.reify(id, is_definition, part_kind, reached);
                        }
                        None => {
                            let file = self.elem_file.get(&id).copied().unwrap_or(0);
                            self.record_miss(file, t.range, &t.segments, &mut stats);
                        }
                    }
                }
            }
            // `perform w;`, `exhibit s;`, `assert c;`, `include u;` -- the reference
            // is what the usage is *about*, and without it the model says only that
            // something is performed.
            //
            // A name that does not resolve is left alone rather than reported:
            // `satisfy requirement viewpointConformance by that;` writes the same
            // shape and *declares* that name, so a finding here would be false.
            //
            // `satisfy r by p;` and `verify r;` have resolvers of their own that
            // record the same operand; recording it here too would give a rename two
            // edits over one name.
            let handled = self
                .model
                .kind(id)
                .is_a(ElementKind::SatisfyRequirementUsage)
                || self.model.member_role(id) == Some(Role::Verify);
            if let Some(operand) = adapter_target(&node).filter(|_| !handled) {
                if let Some(target) = self.resolve_from(id, &operand_segments(&operand)) {
                    stats.resolved += 1;
                    let file = self.elem_file.get(&id).copied().unwrap_or(0);
                    let range = operand.text_range();
                    let name_range = last_name_range(&operand);
                    self.record(
                        id,
                        file,
                        range,
                        name_range,
                        &operand_ranges(&operand),
                        target,
                    );
                    self.reify(id, false, SyntaxKind::REFERENCES, target);
                }
            }
            // `connection c : L connect a to b;` is written as a usage, so
            // its ends arrive here rather than through a connector
            // statement -- and so is KerML's `connector c from a to b;`,
            // which is a `Connector` and not a usage at all.
            if self.model.kind(id).is_a(ElementKind::Connector) {
                self.resolve_connector_ends(id, &node, &mut stats);
            }
            if self
                .model
                .kind(id)
                .is_a(ElementKind::SatisfyRequirementUsage)
            {
                self.resolve_satisfaction(id, &node, &mut stats);
            }
            if self.model.member_role(id) == Some(Role::Verify) {
                self.resolve_verification(id, &node, &mut stats);
            }
        }
        self.resolve_expression_tree(ids);
        self.imply_for(ids);
        stats.lookups = self.lookups - began;
        stats
    }
    /// What a connector relates, where it wrote its ends as declarations of
    /// their own.
    ///
    /// `interface i : WHI connect [1] lugNutPort ::> wheel.lugNutPort to [1]
    /// shankPort ::> hub.shankPort;` writes the ends and not the things they
    /// stand for, and `relatedFeature =
    /// connectorEnd.ownedReferenceSubsetting.subsettedFeature` says which of
    /// them the connector relates. What an end refers to is resolved with the
    /// end, so this waits until both are.
    ///
    /// A dotted reference is a chain, and what the connector relates is the
    /// feature it ends at: a consumer asking what is connected to what wants
    /// the port, not the path to it.
    fn relate_named_ends(&mut self, ids: &[ElementId]) {
        for &elem in ids {
            if !self.model.kind(elem).is_a(ElementKind::Connector)
                || self.model.maybe(elem, "relatedFeature").is_some()
            {
                continue;
            }
            let related: Vec<ElementId> = self
                .model
                .owned(elem)
                .iter()
                .filter(|&&it| self.model.flag(it, "isEnd"))
                .filter_map(|&end| {
                    self.model
                        .owned(end)
                        .iter()
                        .copied()
                        .find(|&it| self.model.kind(it) == ElementKind::ReferenceSubsetting)
                        .and_then(|it| self.model.referenced_feature(it))
                        .map(|reached| {
                            self.model
                                .chaining_feature(reached)
                                .last()
                                .copied()
                                .unwrap_or(reached)
                        })
                })
                .collect();
            if !related.is_empty() {
                self.try_set(elem, "relatedFeature", Value::RefList(related));
            }
        }
    }
    /// Drop what the guard's refusals shaped, once the reference that
    /// prompted them has been answered. An element is an import, an
    /// alias or a type, so at most one of these has anything under it.
    pub(crate) fn forget_provisional(&mut self) {
        if self.depth == 0 && self.resolving.is_empty() {
            for id in std::mem::take(&mut self.provisional) {
                self.imports.remove(&id);
                self.aliases.remove(&id);
                self.supertypes.remove(&id);
                self.semantic_bases.remove(&id);
            }
        }
    }
}

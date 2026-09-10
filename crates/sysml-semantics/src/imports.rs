//! What an `import` brings into a namespace, and what an `alias` stands
//! for.
//!
//! `import A::B;` takes one member, `A::*` takes the namespace's own,
//! `A::**` takes those and everything nested below them, and each of
//! those may be filtered: `import A::*[@Safety];` admits only what the
//! condition holds of, which means classifying every candidate before
//! it is known whether the name is there at all. A re-export chain --
//! `public import` of a namespace that itself publicly imports -- is
//! walked the same way, with a guard, since a model may write a cycle.
//!
//! An alias is here because it is the same question asked of one name:
//! what does this stand for, once whatever it stands for has been
//! worked out.

use std::collections::HashSet;

use sysml_model::{ElementId, ElementKind};
use sysml_syntax::{SyntaxKind, SyntaxNode};

use crate::syntax::*;
use crate::{Access, ImportScope, ImportTarget, Workspace};

impl Workspace {
    /// All references resolving to `target`.
    /// Whether `elem` is an `alias X for Y;`. A name reached through one
    /// resolves to what it stands for, so nothing records that the alias
    /// was the way in -- which is why renaming one cannot be offered.
    pub fn is_alias(&self, elem: ElementId) -> bool {
        self.source
            .get(&elem)
            .is_some_and(|node| node.kind() == SyntaxKind::ALIAS)
    }
    /// What an `alias X for Y;` stands for, once it has been resolved.
    ///
    /// Read back off the model rather than out of the resolver's
    /// memory, so it costs nothing and answers for a workspace that is
    /// only being read. A model written out carries the alias as an
    /// element of its own, and an alias that says nothing about what it
    /// names is a name given to nothing.
    pub fn alias_target(&self, alias: ElementId) -> Option<ElementId> {
        self.model.member_element(alias)
    }
    /// What one import element resolved to: the member a
    /// `import A::B;` names, or the namespace whose members a
    /// `import A::*;` or `import A::**;` exposes.
    pub fn import_of(&mut self, import: ElementId) -> Option<ElementId> {
        self.import_target(import).map(|target| target.target)
    }
    /// The members the imports of `ns` bring into it, resolved and
    /// visibility-filtered, in import order: what the standard's derived
    /// `importedMembership` reaches beyond the owned members.
    ///
    /// A member import contributes the member itself; a namespace import
    /// contributes the target's visible members (all of them under
    /// `import all`); a recursive import adds the visible members of every
    /// namespace below the target as well.
    pub fn imported_members(&mut self, ns: ElementId) -> Vec<ElementId> {
        let mut out = Vec::new();
        let mut seen: HashSet<ElementId> = self.model.owned(ns).iter().copied().collect();
        for import in self.imports_of(ns) {
            let Some(imp) = self.import_target(import) else {
                continue;
            };
            let access = if imp.all {
                Access::Internal
            } else {
                Access::External
            };
            if imp.scope.brings_the_member()
                && self.admits(imp.target, &imp.filters)
                && seen.insert(imp.target)
            {
                out.push(imp.target);
            }
            if imp.scope.brings_its_members() {
                self.visible_members_into(imp.target, access, &imp.filters, &mut out, &mut seen);
                if imp.scope.reaches_below() {
                    for below in self.nested_visible(imp.target, access) {
                        if self.model.kind(below).is_a(ElementKind::Namespace) {
                            self.visible_members_into(
                                below,
                                access,
                                &imp.filters,
                                &mut out,
                                &mut seen,
                            );
                        }
                    }
                }
            }
        }
        out
    }
    /// Append the members of `ns` that `access` can see, each once.
    fn visible_members_into(
        &mut self,
        ns: ElementId,
        access: Access,
        filters: &[(SyntaxNode, ElementId)],
        out: &mut Vec<ElementId>,
        seen: &mut HashSet<ElementId>,
    ) {
        for child in self.model.owned(ns).to_vec() {
            if self.model.kind(child).is_a(ElementKind::Import)
                || self.model.kind(child).is_a(ElementKind::Relationship)
            {
                continue;
            }
            if self.visible(child, access) && self.admits(child, filters) && seen.insert(child) {
                out.push(child);
            }
        }
    }
    pub(crate) fn imports_of(&self, ns: ElementId) -> Vec<ElementId> {
        self.model
            .owned(ns)
            .iter()
            .copied()
            .filter(|c| self.model.kind(*c).is_a(ElementKind::Import))
            .collect()
    }
    /// The filter conditions an import has to satisfy, each with the
    /// element the names in it are resolved from.
    ///
    /// The language writes them in two places -- `import A::*[@Safety];`
    /// on the import itself and `filter @Safety;` beside it in the
    /// package -- and means the same by both: the grammar makes the
    /// bracketed form a `FilterPackage` owning the import, so either way
    /// the conditions belong to the namespace the names arrive in. Only
    /// the second is an element of the model, which is why both are read
    /// off the syntax here rather than one of each.
    fn filters_of(&self, import: ElementId) -> Vec<(SyntaxNode, ElementId)> {
        let mut out = Vec::new();
        let condition_of = |node: &SyntaxNode, at: ElementId, out: &mut Vec<_>| {
            if let Some(written) = node.children().find(|it| it.kind() != SyntaxKind::BODY) {
                out.push((written, at));
            }
        };
        let bracketed = self
            .source
            .get(&import)
            .into_iter()
            .flat_map(|node| node.children())
            .filter(|it| it.kind() == SyntaxKind::FILTER);
        for filter in bracketed {
            condition_of(&filter, import, &mut out);
        }
        let beside = self
            .model
            .owner(import)
            .map(|ns| self.model.owned(ns).to_vec())
            .unwrap_or_default();
        for member in beside {
            if !self
                .model
                .kind(member)
                .is_a(ElementKind::ElementFilterMembership)
            {
                continue;
            }
            if let Some(node) = self.source.get(&member) {
                condition_of(&node.clone(), member, &mut out);
            }
        }
        out
    }
    /// Whether an import may bring `member` in.
    ///
    /// "All filterConditions are checked against every Membership that
    /// would otherwise be imported into the Package if it had no
    /// filterConditions. A Membership shall be imported into the Package
    /// if and only if every filterCondition evaluates to true either
    /// with no target Element, or with any MetadataFeature of the
    /// memberElement of the Membership as the target Element" (KerML
    /// 8.4.4.14).
    ///
    /// A condition past what is evaluated here answers yes. A filter
    /// says which of the names already there to keep, so one that is not
    /// understood must not take a name the model does declare and leave
    /// it resolving to nothing -- the same three-answer reading the
    /// constraint checker gives an OCL body it cannot evaluate.
    pub(crate) fn admits(
        &mut self,
        member: ElementId,
        filters: &[(SyntaxNode, ElementId)],
    ) -> bool {
        filters
            .iter()
            .all(|(condition, at)| self.condition_holds(condition, member, *at) != Some(false))
    }
    /// What one filter condition comes to for one candidate member, or
    /// `None` where the condition reaches past the operators that can be
    /// answered from the model alone.
    fn condition_holds(
        &mut self,
        condition: &SyntaxNode,
        member: ElementId,
        at: ElementId,
    ) -> Option<bool> {
        let operator = condition
            .children_with_tokens()
            .filter_map(|part| part.into_token())
            .map(|token| token.kind())
            .find(|kind| !kind.is_trivia());
        let mut operands = condition
            .children()
            .filter(|child| child.kind() != SyntaxKind::QUALIFIED_NAME);
        match condition.kind() {
            SyntaxKind::PAREN_EXPR => {
                let inner = operands.next()?;
                self.condition_holds(&inner, member, at)
            }
            // `@Safety` asks what the member is annotated with, and it
            // is the whole of what the published models filter by.
            SyntaxKind::UNARY_EXPR if operator == Some(SyntaxKind::AT) => {
                let name = condition
                    .children()
                    .find(|child| child.kind() == SyntaxKind::QUALIFIED_NAME)?;
                let wanted = self.resolve_from(at, &name_segments(&name))?;
                Some(self.classified_by(member, wanted))
            }
            SyntaxKind::UNARY_EXPR if operator == Some(SyntaxKind::NOT_KW) => {
                let inner = operands.next()?;
                Some(!self.condition_holds(&inner, member, at)?)
            }
            SyntaxKind::BINARY_EXPR => {
                // The operator is read before the operands because one
                // this cannot answer -- `==` and `>` are both in the
                // corpus -- settles the condition whatever they are.
                let combine: fn(bool, bool) -> bool = match operator? {
                    SyntaxKind::AND_KW | SyntaxKind::AMP => |left, right| left && right,
                    SyntaxKind::OR_KW | SyntaxKind::PIPE => |left, right| left || right,
                    SyntaxKind::XOR_KW => |left, right| left != right,
                    _ => return None,
                };
                let left = operands.next()?;
                let right = operands.next()?;
                // Both sides are answered whichever operator it is: a
                // condition half of which cannot be evaluated is not
                // evaluated, rather than answered from the half that can.
                let left = self.condition_holds(&left, member, at)?;
                let right = self.condition_holds(&right, member, at)?;
                Some(combine(left, right))
            }
            _ => None,
        }
    }
    /// Is `member` classified by `wanted`?
    ///
    /// Two ways it can be. What has been said about it: `@Safety;`
    /// inside a body and `#Safety` in front of a declaration both leave
    /// a metadata feature owned by what they annotate, typed by the
    /// metadata definition they name. And what it *is*: the standard
    /// libraries carry a reflective model of the abstract syntax --
    /// `metaclass Structure specializes Class` in `KerML`, `metadata def
    /// PartUsage` in `SysML` -- so `@Structure` and `@SysML::PartUsage`
    /// filter by the metaclass rather than by any annotation. The corpus
    /// writes both, in the same file.
    fn classified_by(&mut self, member: ElementId, wanted: ElementId) -> bool {
        for child in self.model.owned(member).to_vec() {
            if !self.model.kind(child).is_a(ElementKind::MetadataFeature) {
                continue;
            }
            if self.reaches(child, wanted) {
                return true;
            }
        }
        let names_a_metaclass = matches!(
            self.model.kind(wanted),
            ElementKind::Metaclass | ElementKind::MetadataDefinition
        );
        names_a_metaclass
            && self
                .model
                .name(wanted)
                .and_then(ElementKind::from_name)
                .is_some_and(|kind| self.model.kind(member).is_a(kind))
    }
    pub(crate) fn import_target(&mut self, import: ElementId) -> Option<ImportTarget> {
        if let Some(cached) = self.imports.get(&import) {
            return cached.clone();
        }
        if !self.in_progress.insert(import) {
            if self.resolving.last() != Some(&import) {
                self.blocked += 1;
            }
            return None; // import cycle
        }
        self.resolving.push(import);
        self.lookups += 1;
        let cut = self.blocked;
        let result = (|| {
            let node = self.source.get(&import)?.clone();
            let qname = node
                .children()
                .find(|c| c.kind() == SyntaxKind::QUALIFIED_NAME)?;
            let mut segments = name_segments(&qname);
            let scope = match segments.last().map(String::as_str) {
                Some("**") => {
                    segments.pop();
                    if segments.last().map(String::as_str) == Some("*") {
                        segments.pop();
                        ImportScope::Recursive
                    } else {
                        ImportScope::RecursiveMember
                    }
                }
                Some("*") => {
                    segments.pop();
                    ImportScope::Members
                }
                _ => ImportScope::Member,
            };
            // `expose P::*;` writes no `all` and means it: "An Expose
            // always imports all Elements, regardless of visibility
            // (isImportAll = true)". A view shows what it is pointed
            // at, and what a package keeps to itself is still part of
            // what it is.
            let all = node.kind() == SyntaxKind::EXPOSE
                || node
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .any(|t| t.kind() == SyntaxKind::ALL_KW);
            let target = self.resolve_from(import, &segments)?;
            let leaf = if scope.brings_the_member() {
                segments.last().cloned()
            } else {
                None
            };
            Some(ImportTarget {
                target,
                scope,
                leaf,
                all,
                filters: self.filters_of(import),
            })
        })();
        self.resolving.pop();
        self.in_progress.remove(&import);
        // Resolving one import can ask for another -- `import A::B;` then
        // `import B::c;` -- and the guard above answers `None` for
        // whichever is already under way. That `None` says nothing about
        // the import, so remembering it would leave the import dead for
        // the rest of the session. Every other failure is the real
        // answer and must be remembered: a name that is genuinely absent
        // is asked for once per reference, and re-searching every scope
        // each time costs seconds on a forty-line file.
        self.imports.insert(import, result.clone());
        if result.is_none() && self.blocked != cut {
            self.provisional.insert(import);
        }
        result
    }
    pub(crate) fn resolve_alias(&mut self, alias: ElementId) -> Option<ElementId> {
        if let Some(cached) = self.aliases.get(&alias) {
            return *cached;
        }
        if !self.in_progress.insert(alias) {
            if self.resolving.last() != Some(&alias) {
                self.blocked += 1;
            }
            return None;
        }
        self.resolving.push(alias);
        self.lookups += 1;
        let cut = self.blocked;
        let result = (|| {
            let node = self.source.get(&alias)?.clone();
            let qname = node
                .children()
                .find(|c| c.kind() == SyntaxKind::QUALIFIED_NAME)?;
            let segments = name_segments(&qname);
            self.resolve_from(alias, &segments)
        })();
        self.resolving.pop();
        self.in_progress.remove(&alias);
        self.aliases.insert(alias, result);
        if result.is_none() && self.blocked != cut {
            self.provisional.insert(alias);
        }
        result
    }
}

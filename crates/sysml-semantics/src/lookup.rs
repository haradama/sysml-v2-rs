//! Working a written name out to the element it names.
//!
//! A name is looked up from where it was written, outward: the members of
//! the enclosing namespace, then what it inherits, then what it imports,
//! then the namespace that owns it, and so on to the root. Visibility
//! narrows what each step may answer with, and a qualified name walks that
//! whole search once per segment.
//!
//! What a lookup *finds* is here; what is then done with it is in `reify`.

use std::collections::{HashMap, HashSet};

use sysml_model::{ElementId, ElementKind, Value, Vis};
use sysml_syntax::{SyntaxKind, SyntaxNode, TextRange};

use crate::syntax::*;
use crate::{reaches, Access, Reference, ResolveStats, Unresolved, Workspace, MAX_INHERITANCE};

/// How far a lookup may reach past a namespace's own members.
///
/// These four travelled as loose arguments through a chain of three
/// methods, two of them adjacent booleans: `lookup_guarded(sup, name,
/// sub_access, true, false, exclude, guard)` said which was which only
/// in the reader's memory, and swapping them typechecks. Carried as one
/// thing and named where each is built, they cannot be.
#[derive(Clone, Copy)]
pub(crate) struct Reach {
    /// Which members the search may see. A private member answers a
    /// name written inside the namespace and not one written outside.
    access: Access,
    /// Whether what the namespace inherits is searched.
    inherited: bool,
    /// Whether what it imports is searched. A supertype's members are
    /// inherited; what that supertype itself imports is not, so the
    /// walk upward turns this off.
    imports: bool,
    /// A member the answer must not be -- the declaration whose own
    /// clause is being resolved, which would otherwise answer itself.
    exclude: Option<ElementId>,
}

impl Reach {
    /// Everything a name written in a namespace may reach: what the
    /// namespace has, what it inherits and what it imports.
    pub(crate) fn all(access: Access) -> Reach {
        Reach {
            access,
            inherited: true,
            imports: true,
            exclude: None,
        }
    }

    /// ... but never this member.
    pub(crate) fn excluding(self, exclude: Option<ElementId>) -> Reach {
        Reach { exclude, ..self }
    }

    /// ... and the inherited members only where `inherited`.
    pub(crate) fn inheriting(self, inherited: bool) -> Reach {
        Reach { inherited, ..self }
    }

    /// ... and seeing only what `access` allows. The walk narrows this
    /// as it climbs: a private member of a supertype answers a name
    /// written in the subtype, and a private member of what that
    /// supertype imports does not.
    pub(crate) fn narrowed_to(self, access: Access) -> Reach {
        Reach { access, ..self }
    }

    /// ... and not what the namespace imports.
    pub(crate) fn without_imports(self) -> Reach {
        Reach {
            imports: false,
            ..self
        }
    }
}

impl Workspace {
    /// The element a whole name answers to, read from the root.
    ///
    /// Visibility has no part in it: this is the index a tool searches, not a
    /// name a model wrote, and `Vault::Sealed` names the same element whether
    /// or not the file asking could have written it.
    /// [`resolve_from`](Self::resolve_from) is the other question.
    ///
    /// `Namespace::resolveGlobal` is one of the four the metamodel writes as
    /// prose rather than OCL. The last segment names the candidates and the
    /// whole name picks one out -- a scan of every name in the workspace,
    /// which is why the answers are kept.
    ///
    /// The candidates are gathered rather than searched for: ranking them the
    /// way [`search_names`](Self::search_names) does is work for a person
    /// choosing between near misses, and taking the best few hundred first
    /// only meant that enough elements called `x` could push `SomePackage::x`
    /// out of its own answer.
    pub fn named_globally(&mut self, qualified: &str) -> Option<ElementId> {
        if let Some(&found) = self.globals.get(qualified) {
            return found;
        }
        let declared = qualified.rsplit("::").next().unwrap_or(qualified);
        let candidates: Vec<ElementId> = self
            .named_elements()
            .filter(|&(_, name)| name == declared)
            .map(|(id, _)| id)
            .collect();
        let found = candidates
            .into_iter()
            .find(|&it| self.qualified_name_of(it) == qualified);
        self.globals.insert(qualified.to_string(), found);
        found
    }
    /// Resolve a qualified name starting from the scope that contains
    /// `elem`. `elem` itself is excluded from name matches: a feature's own
    /// (effective) name must not shadow the inherited feature it redefines.
    pub fn resolve_from(&mut self, elem: ElementId, segments: &[String]) -> Option<ElementId> {
        self.resolve_written(elem, segments, true, None)
    }
    /// As `resolve_from`, saying whether the declaration the name is
    /// written on may answer with itself.
    pub(crate) fn resolve_written(
        &mut self,
        elem: ElementId,
        segments: &[String],
        may_name_itself: bool,
        expected: Option<ElementKind>,
    ) -> Option<ElementId> {
        if segments.is_empty() {
            return None;
        }
        self.depth += 1;
        // Kept for the whole walk and put back afterwards: an import
        // resolving its own path is a reference of its own, written
        // where the import is rather than where the name that woke it
        // was.
        let outer = std::mem::replace(&mut self.origin, elem);
        let exclude = Some(elem);
        let found = self.resolve_segments(elem, segments, exclude).or_else(|| {
            // The declaration is kept out of the first walk so it cannot answer for
            // itself -- but that is about where the walk *ends*, and the name may
            // pass through the declaration on the way: `classifier C specializes
            // C::D { classifier D; }` reaches a member of its own, and the first
            // segment of that path is the name being declared. So the walk is made
            // again with the declaration allowed to answer, and what it landed on
            // decides.
            let hit = self.resolve_segments(elem, segments, None)?;
            if hit != elem {
                return Some(hit);
            }
            // A self-reference (`part p4 :> p4;`) is a legal name where
            // the language allows one. But a feature with no name of its
            // own answers to the name of what it redefines, and that is
            // the very thing being looked up here. Letting it match
            // itself would make the answer its own premise: `attribute
            // :>> nothingHere;` would resolve, and a model naming
            // something that exists nowhere would be reported as sound.
            let names_itself = self.model.name(elem) == segments.last().map(String::as_str);
            (may_name_itself && names_itself).then_some(hit)
        });
        // "The metaclass of the memberElement must conform to the
        // expected metaclass in the context of the name resolution ...
        // if the resulting Element does not have the proper type for
        // its context, then the qualified name has no resolution"
        // (KerML 8.2.3.5.1). `feature aa subsets non;` names nothing
        // when `non` is a classifier, however plainly the name is
        // there: what a feature subsets is a feature.
        let found =
            found.filter(|&hit| expected.is_none_or(|kind| self.model.kind(hit).is_a(kind)));
        self.depth -= 1;
        self.origin = outer;
        if found.is_none() {
            self.misses += 1;
            // A walk that found nothing named nothing. The segments a
            // walk started from inside this one recorded are not this
            // one's, and left behind they are handed to whatever
            // reference is recorded next -- which is how a rename comes
            // to rewrite a token the name never touched.
            self.chain.clear();
        }
        self.forget_provisional();
        found
    }
    /// Where an import's path was written, and what each part of it names. The
    /// wildcard at the end names nothing.
    ///
    /// An import that names nothing is a finding like any other: `import
    /// Vehicles::*;` where the package is spelled `Vehicle` brings in nothing,
    /// and every name the file expected then fails somewhere else -- or, in a
    /// file that only re-exports, goes quietly missing. Saying so at the
    /// import is saying it once, where the typo is.
    pub(crate) fn record_import(
        &mut self,
        import: ElementId,
        node: &SyntaxNode,
        stats: &mut ResolveStats,
    ) {
        let (mut segments, mut at) = node
            .children()
            .find(|c| c.kind() == SyntaxKind::QUALIFIED_NAME)
            .map(|qname| (name_segments(&qname), segment_ranges(&qname)))
            .unwrap_or_default();
        while matches!(segments.last().map(String::as_str), Some("*" | "**")) {
            segments.pop();
            at.pop();
        }
        let (Some(first), Some(last), false) =
            (at.first().copied(), at.last().copied(), segments.is_empty())
        else {
            return;
        };
        let file = self.elem_file.get(&import).copied().unwrap_or(0);
        match self.resolve_from(import, &segments) {
            Some(target) => {
                stats.resolved += 1;
                self.record(file, last, last, &at, target);
            }
            None => {
                let whole = TextRange::new(first.start(), last.end());
                self.record_miss(file, whole, &segments, stats);
            }
        }
    }
    /// Resolve what an alias names, put it on the membership, and say
    /// where it was written -- renaming the element has to reach the
    /// alias too.
    pub(crate) fn record_alias(
        &mut self,
        alias: ElementId,
        node: &SyntaxNode,
        stats: &mut ResolveStats,
    ) {
        let file = self.elem_file.get(&alias).copied().unwrap_or(0);
        // An alias writes exactly one name -- the parser puts one there
        // even where the text does not, empty rather than missing.
        for qname in node
            .children()
            .filter(|c| c.kind() == SyntaxKind::QUALIFIED_NAME)
        {
            let at = segment_ranges(&qname);
            let range = qname.text_range();
            match self.resolve_alias(alias) {
                Some(target) => {
                    stats.resolved += 1;
                    self.try_set(alias, "memberElement", Value::Ref(target));
                    self.record(file, range, last_name_range(&qname), &at, target);
                }
                None => self.record_miss(file, range, &name_segments(&qname), stats),
            }
        }
    }
    /// Say that a name resolved to nothing, and count it.
    pub(crate) fn record_miss(
        &mut self,
        file: usize,
        range: TextRange,
        segments: &[String],
        stats: &mut ResolveStats,
    ) {
        stats.unresolved += 1;
        self.unresolved.push(Unresolved {
            file,
            range,
            name: Self::spell(segments),
        });
    }
    /// Record a resolved reference, and with it every earlier segment of
    /// the qualified name that got there. `Classes::A` names the package
    /// as well as the class, and an editor renaming the package has to
    /// be told where it was named -- otherwise it rewrites the
    /// declaration and leaves the mentions of it behind.
    pub(crate) fn record(
        &mut self,
        file: usize,
        range: TextRange,
        name_range: TextRange,
        at: &[TextRange],
        target: ElementId,
    ) {
        let walked = std::mem::take(&mut self.chain);
        self.references.push(Reference {
            file,
            range,
            name_range,
            target,
        });
        // the last segment is the reference just recorded
        let earlier = walked.len().saturating_sub(1);
        for (&range, &element) in at.iter().zip(walked.iter()).take(earlier) {
            self.references.push(Reference {
                file,
                range,
                name_range: range,
                target: element,
            });
        }
    }
    fn resolve_segments(
        &mut self,
        elem: ElementId,
        segments: &[String],
        exclude: Option<ElementId>,
    ) -> Option<ElementId> {
        let (mut current, rest) = if segments[0].is_empty() {
            (self.root, &segments[1..])
        } else {
            let first = self.resolve_first_segment(elem, &segments[0], exclude);
            (first?, &segments[1..])
        };
        // Kept per segment, and only handed over once the whole name has
        // resolved: a walk that gives up halfway named nothing, and one
        // started from inside this one (an import resolving its own
        // path) must not be mistaken for it.
        let mut walked = vec![current];
        for seg in rest {
            current = self.lookup(
                current,
                seg,
                Reach::all(Access::External).excluding(exclude),
            )?;
            walked.push(current);
        }
        self.chain = walked;
        Some(current)
    }
    /// A name read from inside `elem`, which is where a relationship
    /// written in its declaration sits.
    pub(crate) fn resolve_inside(
        &mut self,
        elem: ElementId,
        segments: &[String],
    ) -> Option<ElementId> {
        let first = segments.first().filter(|it| !it.is_empty())?;
        let mut current = self.lookup(elem, first, Reach::all(Access::Internal))?;
        for seg in &segments[1..] {
            current = self.lookup(current, seg, Reach::all(Access::External))?;
        }
        Some(current)
    }
    fn resolve_first_segment(
        &mut self,
        elem: ElementId,
        name: &str,
        exclude: Option<ElementId>,
    ) -> Option<ElementId> {
        // Connector/association ends resolve against the connector's own
        // ends, then the types those ends relate, then the enclosing scope;
        // members inherited through the container's typing come last (so a
        // connector usage's ends prefer its featuring scope over its type).
        if let Some(container) = self.end_context(elem) {
            if let Some(hit) = self.lookup(
                container,
                name,
                Reach::all(Access::Internal)
                    .inheriting(self.redefining)
                    .excluding(exclude),
            ) {
                return Some(hit);
            }
            for end in self.model.owned(container).to_vec() {
                if !self.is_end_member(end) {
                    continue;
                }
                let mut candidates = self.supertypes_of(end);
                for nested in self.model.owned(end).to_vec() {
                    candidates.extend(self.supertypes_of(nested));
                }
                for candidate in candidates {
                    if let Some(hit) = self.lookup(
                        candidate,
                        name,
                        Reach::all(Access::Inherited).excluding(exclude),
                    ) {
                        return Some(hit);
                    }
                }
            }
            let mut scope = self.model.owner(container);
            while let Some(ns) = scope {
                if let Some(hit) =
                    self.lookup(ns, name, Reach::all(Access::Internal).excluding(exclude))
                {
                    return Some(hit);
                }
                scope = self.model.owner(ns);
            }
            return self.lookup(
                container,
                name,
                Reach::all(Access::Internal).excluding(exclude),
            );
        }
        let mut scope = self.model.owner(elem);
        while let Some(ns) = scope {
            if let Some(hit) =
                self.lookup(ns, name, Reach::all(Access::Internal).excluding(exclude))
            {
                return Some(hit);
            }
            scope = self.model.owner(ns);
        }
        None
    }
    /// The connector/association owning the nearest enclosing `end` member,
    /// if `elem` lives inside one.
    fn end_context(&mut self, elem: ElementId) -> Option<ElementId> {
        let mut current = elem;
        loop {
            if self.is_end_member(current) {
                return self.model.owner(current);
            }
            current = self.model.owner(current)?;
        }
    }
    /// Look up `name` as a member of `ns`.
    pub(crate) fn lookup(&mut self, ns: ElementId, name: &str, reach: Reach) -> Option<ElementId> {
        let mut guard = HashSet::new();
        self.lookup_guarded(ns, name, reach, &mut guard)
    }

    fn lookup_guarded(
        &mut self,
        ns: ElementId,
        name: &str,
        reach: Reach,
        guard: &mut HashSet<ElementId>,
    ) -> Option<ElementId> {
        if !guard.insert(ns) {
            return None;
        }
        // A namespace inherits from a namespace that inherits from a
        // namespace: the walk goes as deep as the model specializes,
        // and a model can specialize deeper than a stack goes. Past
        // this the walk stops and the name is reported unresolved,
        // which is a finding a reader can act on rather than a crash.
        if self.walking >= MAX_INHERITANCE {
            return None;
        }
        self.walking += 1;
        let found = self.lookup_walk(ns, name, reach, guard);
        self.walking -= 1;
        found
    }

    fn lookup_walk(
        &mut self,
        ns: ElementId,
        name: &str,
        reach: Reach,
        guard: &mut HashSet<ElementId>,
    ) -> Option<ElementId> {
        // Direct members and aliases, in the order they are written.
        let mut candidates = self.members_named(ns, name);
        if ns == self.root && candidates.len() > 1 {
            // Two files may declare a root package of the same name --
            // a model's own `Requirements` and the standard library's.
            // Which one is meant is settled by where the name was
            // written rather than by the order the files were loaded:
            // the library resolves within the library, a model within
            // its own files. `findings` reports the collision.
            let side = self.in_library(self.origin);
            candidates.sort_by_key(|&member| self.in_library(member) != side);
        }
        for child in candidates {
            if Some(child) == reach.exclude || !self.visible(child, reach.access) {
                continue;
            }
            // an import is a member of the namespace but answers to no
            // name of its own, so the index never files one
            if self.model.kind(child) == ElementKind::Membership {
                if let Some(target) = self.resolve_alias(child) {
                    return Some(target);
                }
            } else {
                return Some(child);
            }
        }
        // inherited members through specializations/typings. Private members
        // are not inherited; through an external path only public ones are
        // accessible.
        if reach.inherited {
            let sub_access = match reach.access {
                Access::Internal | Access::Inherited => Access::Inherited,
                Access::External => Access::External,
            };
            // Two supertypes may both answer to the name, and one of
            // their answers may be a refinement of the other's --
            // `classifier C specializes A, B` where `B` redefines
            // `A::f`. The refinement is the member, whichever order the
            // supertypes happen to be written in, so the answer is the
            // candidate no other candidate specializes.
            let mut hits = Vec::new();
            for sup in self.supertypes_of(ns) {
                if let Some(hit) = self.lookup_guarded(
                    sup,
                    name,
                    reach.without_imports().narrowed_to(sub_access),
                    guard,
                ) {
                    if !hits.contains(&hit) {
                        hits.push(hit);
                    }
                }
            }
            if let Some(&most) = hits
                .iter()
                .find(|&&hit| {
                    !hits
                        .iter()
                        .any(|&other| other != hit && reaches(&self.model, other, hit))
                })
                .or(hits.first())
            {
                return Some(most);
            }
        }
        // imported members: all imports apply inside the namespace itself,
        // only `public import`s re-export
        if reach.imports && reach.access != Access::Inherited {
            for import in self.imports_of(ns) {
                if reach.access == Access::External && self.visibility(import) != Vis::Public {
                    continue;
                }
                let Some(imp) = self.import_target(import) else {
                    continue;
                };
                // `import all` overrides target-side visibility
                let target_access = if imp.all {
                    Access::Internal
                } else {
                    Access::External
                };
                if imp.scope.brings_the_member() {
                    // `import A::B;` makes the member visible under
                    // the name the import wrote, and where that is
                    // the member's own its short name answers too.
                    // Where it is an alias's -- `import A::Alias;`
                    // -- the member's own name was not imported,
                    // and offering it as well would let a name the
                    // importing file never wrote resolve.
                    let wrote = imp.leaf.as_deref();
                    let by_its_own_name =
                        wrote.is_some_and(|leaf| self.member_name_matches(imp.target, leaf));
                    if (wrote == Some(name)
                        || (by_its_own_name && self.member_name_matches(imp.target, name)))
                        && self.admits(imp.target, &imp.filters)
                    {
                        return Some(imp.target);
                    }
                }
                if imp.scope.brings_its_members() {
                    let inherited = self.inherits_into_imports(imp.target);
                    if let Some(hit) = self.lookup_guarded(
                        imp.target,
                        name,
                        reach.narrowed_to(target_access).inheriting(inherited),
                        guard,
                    ) {
                        if self.admits(hit, &imp.filters) {
                            return Some(hit);
                        }
                    }
                    if imp.scope.reaches_below() {
                        for desc in self.nested_visible(imp.target, target_access) {
                            if Some(desc) != reach.exclude
                                && self.member_name_matches(desc, name)
                                && self.admits(desc, &imp.filters)
                            {
                                return Some(desc);
                            }
                        }
                    }
                }
            }
        }
        None
    }
    /// The members of `ns` that answer to `name`, in the order a walk
    /// of the namespace would have met them.
    fn members_named(&mut self, ns: ElementId, name: &str) -> Vec<ElementId> {
        if !self.members.contains_key(&ns) {
            let index = self.member_index(ns);
            self.members.insert(ns, index);
        }
        self.members[&ns].get(name).cloned().unwrap_or_default()
    }
    /// Every member of `ns`, filed under each name it answers to.
    ///
    /// Kept until a file arrives, because what resolution itself adds to
    /// a namespace is relationships and connector ends, and neither has
    /// a name to be found under.
    fn member_index(&self, ns: ElementId) -> HashMap<String, Vec<ElementId>> {
        // An `end` member's body can nest further -- an association
        // whose end holds a feature which itself holds the one being
        // named -- and the whole of it belongs to the connector's
        // scope, so a subtype naming it inherits the lot.
        let mut members = self.model.owned(ns).to_vec();
        for &child in self.model.owned(ns) {
            if self.is_end_member(child) {
                members.extend(self.model.descendants(child));
            }
        }
        let mut index: HashMap<String, Vec<ElementId>> = HashMap::new();
        for member in members {
            for name in self.member_names(member) {
                index.entry(name).or_default().push(member);
            }
        }
        index
    }
    /// Does `import T::*` of this namespace bring in what it inherits?
    ///
    /// A package's members are the ones written in it, but a type's
    /// are its own and every public one it inherits -- KerML puts them
    /// both in `Type::visibleMemberships` -- so importing a type
    /// imports what its supertypes give it.
    fn inherits_into_imports(&self, ns: ElementId) -> bool {
        self.model.kind(ns).is_a(ElementKind::Type)
    }
    /// Which side of the library boundary an element is written on:
    /// whether the root package holding it is a `library package`.
    pub(crate) fn in_library(&self, elem: ElementId) -> bool {
        let mut at = elem;
        while let Some(owner) = self.model.owner(at) {
            if owner == self.root {
                return self.model.kind(at).is_a(ElementKind::LibraryPackage);
            }
            at = owner;
        }
        false
    }
    /// Everything an `import N::**` reaches below `ns`: the members
    /// `access` can see, then the members of those, and so on down.
    ///
    /// The walk stops at a member it cannot see rather than stepping
    /// over it. KerML's `visibleMemberships` recurses only into member
    /// namespaces that are themselves visible, so a `private package`
    /// hides what is nested in it however public each of those is.
    pub(crate) fn nested_visible(&mut self, ns: ElementId, access: Access) -> Vec<ElementId> {
        let mut out = Vec::new();
        let mut stack: Vec<ElementId> = self.model.owned(ns).iter().rev().copied().collect();
        while let Some(at) = stack.pop() {
            if self.model.kind(at).is_a(ElementKind::Relationship) || !self.visible(at, access) {
                continue;
            }
            out.push(at);
            stack.extend(self.model.owned(at).iter().rev().copied());
        }
        out
    }
    /// Declared visibility of a member (imports default to private, other
    /// members to public).
    pub(crate) fn visibility(&mut self, elem: ElementId) -> Vis {
        if let Some(cached) = self.visibilities.get(&elem) {
            return *cached;
        }
        // what the member was declared with, read from the model rather
        // than worked out from the syntax a second time
        let vis = self.model.member_visibility(elem).unwrap_or_else(|| {
            // nothing written: an import keeps to itself, a member does not
            if self.model.kind(elem).is_a(ElementKind::Import) {
                Vis::Private
            } else {
                Vis::Public
            }
        });
        self.visibilities.insert(elem, vis);
        vis
    }
    pub(crate) fn visible(&mut self, elem: ElementId, access: Access) -> bool {
        match access {
            Access::Internal => true,
            Access::Inherited => self.visibility(elem) != Vis::Private,
            Access::External => self.visibility(elem) == Vis::Public,
        }
    }
    /// Is this an end of the connector or association that owns it?
    ///
    /// What an end relates is reached through the types of the other
    /// ends, so an end is where a name lookup carries on from.
    fn is_end_member(&self, elem: ElementId) -> bool {
        self.model.flag(elem, "isEnd")
    }
    pub(crate) fn member_name_matches(&self, elem: ElementId, name: &str) -> bool {
        self.member_names(elem).iter().any(|known| known == name)
    }
    /// Every name a member answers to: the one it was declared with or,
    /// where it declared none, the one it borrows from what it
    /// redefines (`attribute :>> mass = 10.0;` is found as `mass`), and
    /// its short name.
    fn member_names(&self, elem: ElementId) -> Vec<String> {
        let mut names = Vec::new();
        match self.model.name(elem) {
            Some(name) => names.push(name.to_string()),
            None => names.extend(self.effective_name(elem)),
        }
        names.extend(
            self.model
                .get(elem, "declaredShortName")
                .and_then(Value::as_str)
                .map(String::from),
        );
        names
    }
    /// Effective name of an unnamed feature from its first redefinition (or
    /// reference-subsetting) target's last segment. An unnamed `return`
    /// parameter is implicitly named `result` (KerML function semantics).
    fn effective_name(&self, elem: ElementId) -> Option<String> {
        let Some(node) = self.source.get(&elem) else {
            // A parameter the standard implies has no syntax to read.
            // `ReferenceUsage::namingFeature` -- "if this ReferenceUsage is the
            // payload parameter of a TransitionUsage, then its naming Feature is the
            // payloadParameter of the triggerAction" -- and what it subsets is that
            // parameter, which is how `bind payload = aState.aTransition.apayload;`
            // names it from the transition.
            return self
                .model
                .owned(elem)
                .iter()
                .find(|&&it| self.model.kind(it) == ElementKind::Subsetting)
                .and_then(|&it| self.model.subsetted_feature(it))
                .and_then(|named| self.model.name(named))
                .map(String::from);
        };
        if node.kind() != SyntaxKind::USAGE {
            return None;
        }
        for (part, targets) in relationship_parts(node) {
            if matches!(part, SyntaxKind::REDEFINITION | SyntaxKind::REFERENCES) {
                if let Some(target) = targets.first() {
                    return target.segments.last().cloned();
                }
            }
        }
        // `perform providePower.generateTorque;` subsets the performed
        // feature, so the usage answers to `generateTorque` -- the same
        // target `supertypes_of` already inherits members through.
        if let Some(segments) = adapter_target_segments(node) {
            return segments.last().cloned();
        }
        let leads_with_return = node
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| !t.kind().is_trivia())
            .is_some_and(|t| t.kind() == SyntaxKind::RETURN_KW);
        if leads_with_return {
            return Some("result".to_string());
        }
        None
    }
}

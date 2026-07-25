//! Name resolution and relationship reification.
//!
//! A [`Workspace`] holds any number of parsed files built into one shared
//! [`Model`], all owned by a synthetic root namespace (the KerML global
//! namespace, addressable as `$`). [`Workspace::resolve_all`] then resolves
//! every explicit typing (`: T`), specialization (`:>`), redefinition
//! (`:>>`) and reference subsetting (`::>`) target and reifies the
//! corresponding relationship elements ([`ElementKind::FeatureTyping`],
//! [`ElementKind::Subclassification`], [`ElementKind::Subsetting`],
//! [`ElementKind::Redefinition`], [`ElementKind::ReferenceSubsetting`]) into
//! the model, with resolved element references as properties.
//!
//! Lookup handles: member names and short names, ownership-scope walking,
//! visibility (members default public, imports default private; only
//! `public import` re-exports; `import all` overrides), imports (`A::B`,
//! `A::*`, `A::**`, re-exports through import chains), aliases, inherited
//! members through resolved specializations and typings (which also makes
//! feature chains like `engine.mass` work), implicit semantic-library
//! specializations (`part def` → `Parts::Part`, ...), user-defined keywords
//! via SemanticMetadata (`#cause x` specializes the keyword's `baseType`),
//! connector-end scoping, implicit `result` parameters, effective names of
//! unnamed redefining features, and `$`-rooted qualified names.
//!
//! With the official standard library loaded, every reference in the
//! library and in all official example models resolves (regression-tested).

use std::collections::{HashMap, HashSet};

use sysml_model::{build_into, ElementId, ElementKind, Model, Value};
use sysml_syntax::{parse_dialect, Dialect, Parse, SyntaxKind, SyntaxNode, TextRange};

/// An unresolved reference, for reporting.
#[derive(Clone, Debug)]
pub struct Unresolved {
    /// Index of the file (in insertion order) the reference appears in.
    pub file: usize,
    pub range: TextRange,
    pub name: String,
}

/// A successfully resolved reference (for go-to-definition etc.).
#[derive(Clone, Copy, Debug)]
pub struct Reference {
    pub file: usize,
    /// whole qualified-name range
    pub range: TextRange,
    /// final segment only (what a rename replaces)
    pub name_range: TextRange,
    pub target: ElementId,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResolveStats {
    pub resolved: usize,
    pub unresolved: usize,
}

#[derive(Clone)]
struct File {
    name: String,
    parse: Parse,
    roots: Vec<ElementId>,
}

/// How a namespace's members are being accessed during lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Access {
    /// From inside the namespace (or a nested scope): everything visible.
    Internal,
    /// Through specialization: public and protected members.
    Inherited,
    /// Through a qualified path or an import: public members only.
    External,
}

/// Declared visibility of a member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Vis {
    Public,
    Protected,
    Private,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ImportTarget {
    target: ElementId,
    scope: ImportScope,
    /// leaf name for member imports (`import A::Alias;`)
    leaf: Option<String>,
    /// `import all ...` also exposes non-public members
    all: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportScope {
    /// `import A::B;` — one member
    Member,
    /// `import A::*;`
    Members,
    /// `import A::**;`
    Recursive,
}

#[derive(Clone)]
pub struct Workspace {
    model: Model,
    root: ElementId,
    files: Vec<File>,
    /// element -> syntax node it was built from
    source: HashMap<ElementId, SyntaxNode>,
    /// element -> file index
    elem_file: HashMap<ElementId, usize>,
    // caches
    supertypes: HashMap<ElementId, Vec<ElementId>>,
    in_progress: HashSet<ElementId>,
    imports: HashMap<ElementId, Option<ImportTarget>>,
    aliases: HashMap<ElementId, Option<ElementId>>,
    visibilities: HashMap<ElementId, Vis>,
    semantic_bases: HashMap<ElementId, Option<ElementId>>,
    unresolved: Vec<Unresolved>,
    references: Vec<Reference>,
}

impl Default for Workspace {
    fn default() -> Self {
        Workspace::new()
    }
}

impl Workspace {
    pub fn new() -> Workspace {
        let mut model = Model::new();
        let root = model.create(ElementKind::Namespace);
        Workspace {
            model,
            root,
            files: Vec::new(),
            source: HashMap::new(),
            elem_file: HashMap::new(),
            supertypes: HashMap::new(),
            in_progress: HashSet::new(),
            imports: HashMap::new(),
            aliases: HashMap::new(),
            visibilities: HashMap::new(),
            semantic_bases: HashMap::new(),
            unresolved: Vec::new(),
            references: Vec::new(),
        }
    }

    /// Parse `text` (dialect chosen from the file name's extension) and add
    /// it to the workspace. Returns the file index.
    pub fn add_file(&mut self, name: impl Into<String>, text: &str) -> usize {
        let name = name.into();
        let dialect = if name.ends_with(".kerml") {
            Dialect::KerML
        } else {
            Dialect::SysML
        };
        let parse = parse_dialect(text, dialect);
        let built = build_into(&mut self.model, &parse);
        let file_idx = self.files.len();
        for root in &built.roots {
            self.model.add_owned(self.root, *root);
        }
        for (id, node) in built.source {
            self.source.insert(id, node);
            self.elem_file.insert(id, file_idx);
        }
        self.files.push(File {
            name,
            parse,
            roots: built.roots,
        });
        file_idx
    }

    /// Recursively load every `.sysml`/`.kerml` file under `dir`.
    pub fn load_dir(&mut self, dir: &std::path::Path) -> std::io::Result<usize> {
        let mut paths = Vec::new();
        collect_files(dir, &mut paths);
        paths.sort();
        let count = paths.len();
        for path in paths {
            let text = std::fs::read_to_string(&path)?;
            self.add_file(path.to_string_lossy(), &text);
        }
        Ok(count)
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    pub fn root(&self) -> ElementId {
        self.root
    }

    pub fn file_name(&self, file: usize) -> &str {
        &self.files[file].name
    }

    pub fn file_roots(&self, file: usize) -> &[ElementId] {
        &self.files[file].roots
    }

    pub fn unresolved(&self) -> &[Unresolved] {
        &self.unresolved
    }

    /// All resolved references (target locations for IDE queries).
    pub fn references(&self) -> &[Reference] {
        &self.references
    }

    /// The resolved reference covering `offset` in `file`, if any.
    pub fn reference_at(&self, file: usize, offset: sysml_syntax::TextSize) -> Option<&Reference> {
        self.references
            .iter()
            .filter(|r| r.file == file && r.range.contains_inclusive(offset))
            .min_by_key(|r| u32::from(r.range.len()))
    }

    /// All references resolving to `target`.
    pub fn references_to(&self, target: ElementId) -> impl Iterator<Item = &Reference> {
        self.references.iter().filter(move |r| r.target == target)
    }

    /// The element whose declared-name range covers `offset` in `file`
    /// (for rename/find-references started on a declaration).
    pub fn definition_at(&self, file: usize, offset: sysml_syntax::TextSize) -> Option<ElementId> {
        self.model
            .ids()
            .filter(|id| self.elem_file.get(id) == Some(&file))
            .filter_map(|id| {
                let node = self.source.get(&id)?;
                let name = node.children().find(|c| c.kind() == SyntaxKind::NAME)?;
                name.text_range()
                    .contains_inclusive(offset)
                    .then_some((id, name.text_range().len()))
            })
            .min_by_key(|(_, len)| u32::from(*len))
            .map(|(id, _)| id)
    }

    /// Names visible at `offset` in `file` (for completion): members of the
    /// enclosing scopes, inherited members, and imported names.
    /// The innermost model element whose syntax covers `offset` in `file`
    /// (the workspace root when none does).
    pub fn innermost_element(&self, file: usize, offset: sysml_syntax::TextSize) -> ElementId {
        self.model
            .ids()
            .filter(|id| self.elem_file.get(id) == Some(&file))
            .filter_map(|id| {
                let range = self.source.get(&id)?.text_range();
                range
                    .contains_inclusive(offset)
                    .then_some((id, range.len()))
            })
            .min_by_key(|(_, len)| u32::from(*len))
            .map(|(id, _)| id)
            .unwrap_or(self.root)
    }

    /// The call around `offset` (`f(a, |)`): the resolved callee and the
    /// zero-based index of the active argument.
    pub fn callable_at(
        &mut self,
        file: usize,
        offset: sysml_syntax::TextSize,
    ) -> Option<(ElementId, u32)> {
        let syntax = self.files.get(file)?.parse.syntax();
        let token = match syntax.token_at_offset(offset) {
            sysml_syntax::TokenAtOffset::Single(t) => t,
            sysml_syntax::TokenAtOffset::Between(l, _) => l,
            sysml_syntax::TokenAtOffset::None => return None,
        };
        let arg_list = token
            .parent_ancestors()
            .find(|n| n.kind() == SyntaxKind::ARG_LIST)?;
        let call = arg_list.parent()?;
        if call.kind() != SyntaxKind::CALL_EXPR {
            return None;
        }
        let callee = call
            .children()
            .find(|c| matches!(c.kind(), SyntaxKind::NAME_REF | SyntaxKind::PATH_EXPR))?;
        let segments = operand_segments(&callee);
        let active = arg_list
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == SyntaxKind::COMMA && t.text_range().end() <= offset)
            .count() as u32;
        let scope = self.innermost_element(file, offset);
        let target = self.resolve_from(scope, &segments)?;
        Some((target, active))
    }

    /// Parameter labels of a callable element (`in a : Real`), rendered
    /// from the members declared with a direction.
    pub fn parameters_of(&self, elem: ElementId) -> Vec<String> {
        let mut params = Vec::new();
        for child in self.model.owned(elem) {
            let Some(node) = self.source.get(child) else {
                continue;
            };
            if node.kind() != SyntaxKind::USAGE {
                continue;
            }
            let direction = node
                .children_with_tokens()
                .filter_map(|e| e.into_token())
                .find(|t| {
                    matches!(
                        t.kind(),
                        SyntaxKind::IN_KW | SyntaxKind::OUT_KW | SyntaxKind::INOUT_KW
                    )
                });
            let Some(direction) = direction else { continue };
            let mut label = direction.text().to_string();
            if let Some(name) = self.model.name(*child) {
                label.push(' ');
                label.push_str(name);
            }
            if let Some((_, targets)) = relationship_parts(node)
                .into_iter()
                .find(|(kind, _)| *kind == SyntaxKind::TYPING)
            {
                if let Some(target) = targets.first() {
                    label.push_str(" : ");
                    label.push_str(&target.segments.join("::"));
                }
            }
            params.push(label);
        }
        params
    }

    /// Every named element with its name (workspace-wide symbol search).
    pub fn named_elements(&self) -> impl Iterator<Item = (ElementId, &str)> {
        self.model
            .ids()
            .filter_map(|id| self.model.name(id).map(|n| (id, n)))
    }

    pub fn visible_names(
        &mut self,
        file: usize,
        offset: sysml_syntax::TextSize,
    ) -> Vec<(String, ElementKind)> {
        let mut scope = self.innermost_element(file, offset);

        let mut out = Vec::new();
        let mut seen = HashSet::new();
        loop {
            self.collect_visible(scope, Access::Internal, &mut out, &mut seen, 0);
            match self.model.owner(scope) {
                Some(owner) => scope = owner,
                None => break,
            }
        }
        // first (innermost) occurrence of a name wins
        let mut names = HashSet::new();
        out.retain(|(name, _)| names.insert(name.clone()));
        out
    }

    fn collect_visible(
        &mut self,
        ns: ElementId,
        access: Access,
        out: &mut Vec<(String, ElementKind)>,
        seen: &mut HashSet<ElementId>,
        depth: usize,
    ) {
        if depth > 16 || !seen.insert(ns) {
            return;
        }
        for child in self.model.owned(ns).to_vec() {
            if !self.visible(child, access) {
                continue;
            }
            let kind = self.model.kind(child);
            if kind.is_a(ElementKind::Import) {
                continue;
            }
            if let Some(name) = self.model.name(child) {
                out.push((name.to_string(), kind));
            }
            if self.is_end_member(child) {
                for nested in self.model.owned(child).to_vec() {
                    if let Some(name) = self.model.name(nested) {
                        out.push((name.to_string(), self.model.kind(nested)));
                    }
                }
            }
        }
        let sub_access = if access == Access::Internal {
            Access::Inherited
        } else {
            access
        };
        for sup in self.supertypes_of(ns) {
            self.collect_visible(sup, sub_access, out, seen, depth + 1);
        }
        if access != Access::Inherited {
            for import in self.imports_of(ns) {
                let Some(imp) = self.import_target(import) else {
                    continue;
                };
                match imp.scope {
                    ImportScope::Member => {
                        if let Some(name) = imp
                            .leaf
                            .clone()
                            .or_else(|| self.model.name(imp.target).map(String::from))
                        {
                            out.push((name, self.model.kind(imp.target)));
                        }
                    }
                    ImportScope::Members | ImportScope::Recursive => {
                        let target_access = if imp.all {
                            Access::Internal
                        } else {
                            Access::External
                        };
                        self.collect_visible(imp.target, target_access, out, seen, depth + 1);
                    }
                }
            }
        }
    }

    /// File a model element was built from.
    pub fn element_file(&self, elem: ElementId) -> Option<usize> {
        self.elem_file.get(&elem).copied()
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn file_parse(&self, file: usize) -> &Parse {
        &self.files[file].parse
    }

    /// Full node range and declared-name range of an element.
    pub fn element_ranges(&self, elem: ElementId) -> Option<(TextRange, TextRange)> {
        let node = self.source.get(&elem)?;
        let name = node
            .children()
            .find(|c| c.kind() == SyntaxKind::NAME)
            .map(|n| n.text_range())
            .unwrap_or_else(|| node.text_range());
        Some((node.text_range(), name))
    }

    /// Owner-path qualified name of an element (for hovers).
    pub fn qualified_name_of(&self, elem: ElementId) -> String {
        let mut segments = Vec::new();
        let mut current = Some(elem);
        while let Some(e) = current {
            if e == self.root {
                break;
            }
            segments.push(self.model.name(e).unwrap_or("?").to_string());
            current = self.model.owner(e);
        }
        segments.reverse();
        segments.join("::")
    }

    /// The `doc` body attached to an element, if any.
    pub fn documentation_of(&self, elem: ElementId) -> Option<String> {
        self.model
            .owned(elem)
            .iter()
            .find(|c| self.model.kind(**c) == ElementKind::Documentation)
            .and_then(|d| self.model.get(*d, "body"))
            .and_then(Value::as_str)
            .map(String::from)
    }

    /// Resolve every explicit relationship target in the workspace and
    /// reify the relationship elements.
    pub fn resolve_all(&mut self) -> ResolveStats {
        let ids: Vec<ElementId> = self.model.ids().collect();
        self.resolve_ids(&ids)
    }

    /// Resolve only elements belonging to the given files (imports,
    /// supertypes etc. from other files are still resolved on demand).
    pub fn resolve_files(&mut self, files: &[usize]) -> ResolveStats {
        let ids: Vec<ElementId> = self
            .model
            .ids()
            .filter(|id| self.elem_file.get(id).is_some_and(|f| files.contains(f)))
            .collect();
        self.resolve_ids(&ids)
    }

    fn resolve_ids(&mut self, ids: &[ElementId]) -> ResolveStats {
        let mut stats = ResolveStats::default();
        for &id in ids {
            let Some(node) = self.source.get(&id).cloned() else {
                continue;
            };
            if matches!(
                node.kind(),
                SyntaxKind::CONNECTOR_STMT | SyntaxKind::CONTROL_STMT
            ) {
                self.resolve_connector_ends(id, &node, &mut stats);
                continue;
            }
            if !matches!(node.kind(), SyntaxKind::DEFINITION | SyntaxKind::USAGE) {
                continue;
            }
            let is_definition = node.kind() == SyntaxKind::DEFINITION;
            for (part_kind, targets) in relationship_parts(&node) {
                for t in targets {
                    match self.resolve_from(id, &t.segments) {
                        Some(target) => {
                            stats.resolved += 1;
                            self.references.push(Reference {
                                file: self.elem_file.get(&id).copied().unwrap_or(0),
                                range: t.range,
                                name_range: t.name_range,
                                target,
                            });
                            self.reify(id, is_definition, part_kind, target);
                        }
                        None => {
                            stats.unresolved += 1;
                            self.unresolved.push(Unresolved {
                                file: self.elem_file.get(&id).copied().unwrap_or(0),
                                range: t.range,
                                name: t.segments.join("::"),
                            });
                        }
                    }
                }
            }
        }
        stats
    }

    /// Resolve a qualified name starting from the scope that contains
    /// `elem`. `elem` itself is excluded from name matches: a feature's own
    /// (effective) name must not shadow the inherited feature it redefines.
    pub fn resolve_from(&mut self, elem: ElementId, segments: &[String]) -> Option<ElementId> {
        if segments.is_empty() {
            return None;
        }
        let exclude = Some(elem);
        if let Some(hit) = self.resolve_segments(elem, segments, exclude) {
            return Some(hit);
        }
        // self-references (`part p4 :> p4;`) are legal names even though a
        // declaration cannot shadow the feature it redefines
        self.resolve_segments(elem, segments, None)
    }

    fn resolve_segments(
        &mut self,
        elem: ElementId,
        segments: &[String],
        exclude: Option<ElementId>,
    ) -> Option<ElementId> {
        let (mut current, rest) = if segments[0] == "$" {
            (self.root, &segments[1..])
        } else {
            let first = self.resolve_first_segment(elem, &segments[0], exclude);
            (first?, &segments[1..])
        };
        for seg in rest {
            current = self.lookup(current, seg, Access::External, true, exclude)?;
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
            if let Some(hit) = self.lookup(container, name, Access::Internal, false, exclude) {
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
                    if let Some(hit) =
                        self.lookup(candidate, name, Access::Inherited, true, exclude)
                    {
                        return Some(hit);
                    }
                }
            }
            let mut scope = self.model.owner(container);
            while let Some(ns) = scope {
                if let Some(hit) = self.lookup(ns, name, Access::Internal, true, exclude) {
                    return Some(hit);
                }
                scope = self.model.owner(ns);
            }
            return self.lookup(container, name, Access::Internal, true, exclude);
        }
        let mut scope = self.model.owner(elem);
        while let Some(ns) = scope {
            if let Some(hit) = self.lookup(ns, name, Access::Internal, true, exclude) {
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
    fn lookup(
        &mut self,
        ns: ElementId,
        name: &str,
        access: Access,
        allow_inherited: bool,
        exclude: Option<ElementId>,
    ) -> Option<ElementId> {
        let mut guard = HashSet::new();
        self.lookup_guarded(ns, name, access, allow_inherited, true, exclude, &mut guard)
    }

    #[allow(clippy::too_many_arguments)]
    fn lookup_guarded(
        &mut self,
        ns: ElementId,
        name: &str,
        access: Access,
        allow_inherited: bool,
        allow_imports: bool,
        exclude: Option<ElementId>,
        guard: &mut HashSet<ElementId>,
    ) -> Option<ElementId> {
        if !guard.insert(ns) {
            return None;
        }
        // direct members (incl. aliases and features nested in `end` members)
        let mut candidates = self.model.owned(ns).to_vec();
        for child in self.model.owned(ns).to_vec() {
            if self.is_end_member(child) {
                candidates.extend_from_slice(self.model.owned(child));
            }
        }
        for child in candidates {
            if Some(child) == exclude || !self.visible(child, access) {
                continue;
            }
            let kind = self.model.kind(child);
            if kind.is_a(ElementKind::Import) {
                continue;
            }
            if self.member_name_matches(child, name) {
                if kind == ElementKind::Membership {
                    if let Some(target) = self.alias_target(child) {
                        return Some(target);
                    }
                } else {
                    return Some(child);
                }
            }
        }
        // inherited members through specializations/typings. Private members
        // are not inherited; through an external path only public ones are
        // accessible.
        if allow_inherited {
            let sub_access = match access {
                Access::Internal | Access::Inherited => Access::Inherited,
                Access::External => Access::External,
            };
            for sup in self.supertypes_of(ns) {
                if let Some(hit) =
                    self.lookup_guarded(sup, name, sub_access, true, false, exclude, guard)
                {
                    return Some(hit);
                }
            }
        }
        // imported members: all imports apply inside the namespace itself,
        // only `public import`s re-export
        if allow_imports && access != Access::Inherited {
            for import in self.imports_of(ns) {
                if access == Access::External && self.visibility(import) != Vis::Public {
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
                match imp.scope {
                    ImportScope::Member => {
                        // `import A::Alias;` makes the member visible under
                        // the imported (possibly alias) name
                        if imp.leaf.as_deref() == Some(name)
                            || self.member_name_matches(imp.target, name)
                        {
                            return Some(imp.target);
                        }
                    }
                    ImportScope::Members => {
                        if let Some(hit) = self.lookup_guarded(
                            imp.target,
                            name,
                            target_access,
                            false,
                            true,
                            exclude,
                            guard,
                        ) {
                            return Some(hit);
                        }
                    }
                    ImportScope::Recursive => {
                        if let Some(hit) = self.lookup_guarded(
                            imp.target,
                            name,
                            target_access,
                            false,
                            true,
                            exclude,
                            guard,
                        ) {
                            return Some(hit);
                        }
                        for desc in self.model.descendants(imp.target) {
                            if Some(desc) != exclude
                                && self.visible(desc, target_access)
                                && self.member_name_matches(desc, name)
                                && !self.model.kind(desc).is_a(ElementKind::Relationship)
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

    /// Declared visibility of a member (imports default to private, other
    /// members to public).
    fn visibility(&mut self, elem: ElementId) -> Vis {
        if let Some(cached) = self.visibilities.get(&elem) {
            return *cached;
        }
        let vis = (|| {
            let node = self.source.get(&elem)?;
            for token in node
                .children_with_tokens()
                .filter_map(|e| e.into_token())
                .take(4)
            {
                match token.kind() {
                    SyntaxKind::PRIVATE_KW => return Some(Vis::Private),
                    SyntaxKind::PROTECTED_KW => return Some(Vis::Protected),
                    SyntaxKind::PUBLIC_KW => return Some(Vis::Public),
                    _ => {}
                }
            }
            None
        })()
        .unwrap_or_else(|| {
            if self.model.kind(elem).is_a(ElementKind::Import) {
                Vis::Private
            } else {
                Vis::Public
            }
        });
        self.visibilities.insert(elem, vis);
        vis
    }

    fn visible(&mut self, elem: ElementId, access: Access) -> bool {
        match access {
            Access::Internal => true,
            Access::Inherited => self.visibility(elem) != Vis::Private,
            Access::External => self.visibility(elem) == Vis::Public,
        }
    }

    /// Is this a connector-end member (`end e feature f : T;`) whose nested
    /// features are visible from the enclosing type?
    fn is_end_member(&self, elem: ElementId) -> bool {
        self.source.get(&elem).is_some_and(|node| {
            node.kind() == SyntaxKind::USAGE
                && node
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .any(|t| t.kind() == SyntaxKind::END_KW)
        })
    }

    fn member_name_matches(&self, elem: ElementId, name: &str) -> bool {
        if self.model.name(elem) == Some(name)
            || self
                .model
                .get(elem, "declaredShortName")
                .and_then(Value::as_str)
                == Some(name)
        {
            return true;
        }
        // An unnamed redefining feature takes the name of the feature it
        // redefines: `attribute :>> mass = 10.0;` is found as `mass`.
        self.model.name(elem).is_none() && self.effective_name(elem).as_deref() == Some(name)
    }

    /// Effective name of an unnamed feature from its first redefinition (or
    /// reference-subsetting) target's last segment. An unnamed `return`
    /// parameter is implicitly named `result` (KerML function semantics).
    fn effective_name(&self, elem: ElementId) -> Option<String> {
        let node = self.source.get(&elem)?;
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

    /// Supertypes of an element for inherited-member lookup: resolved
    /// targets of its typings, specializations, subsettings, redefinitions,
    /// plus the implicit base type from the semantic libraries.
    fn supertypes_of(&mut self, elem: ElementId) -> Vec<ElementId> {
        if let Some(cached) = self.supertypes.get(&elem) {
            return cached.clone();
        }
        if !self.in_progress.insert(elem) {
            // this element's supertypes are already being computed —
            // break the specialization cycle
            return Vec::new();
        }
        let mut supers = Vec::new();
        if let Some(node) = self.source.get(&elem).cloned() {
            for (_, targets) in relationship_parts(&node) {
                for t in targets {
                    if let Some(target) = self.resolve_from(elem, &t.segments) {
                        push_supertype(&mut supers, elem, target);
                    }
                }
            }
            // `perform vehicleMassTest.collectData { :>> param }` — the
            // performed/exhibited/included target contributes its members
            if let Some(segments) = adapter_target_segments(&node) {
                if let Some(target) = self.resolve_from(elem, &segments) {
                    push_supertype(&mut supers, elem, target);
                }
            }
            // `#cause 'battery old' { ... }` — a user-defined keyword makes
            // the element specialize the keyword's SemanticMetadata baseType
            for segments in prefix_metadata_segments(&node) {
                if let Some(meta_def) = self.resolve_from(elem, &segments) {
                    if let Some(base) = self.semantic_base(meta_def) {
                        push_supertype(&mut supers, elem, base);
                    }
                }
            }
        }
        let kind = self.model.kind(elem);
        let mut implicit: Vec<&str> = implicit_supertype(kind).to_vec();
        // every feature also (implicitly) subsets the top-level `things`
        if kind.is_a(ElementKind::Feature) && !implicit.contains(&"Base::things") {
            implicit.push("Base::things");
        }
        for path in implicit {
            let segments: Vec<String> = path.split("::").map(String::from).collect();
            if let Some(target) = self.resolve_from(elem, &segments) {
                if target != elem && !supers.contains(&target) {
                    supers.push(target);
                }
            }
        }
        self.in_progress.remove(&elem);
        self.supertypes.insert(elem, supers.clone());
        supers
    }

    /// The base type referenced by a SemanticMetadata definition's
    /// `:>> baseType = <ref> meta ...` member, if any.
    fn semantic_base(&mut self, meta_def: ElementId) -> Option<ElementId> {
        if let Some(cached) = self.semantic_bases.get(&meta_def) {
            return *cached;
        }
        if !self.in_progress.insert(meta_def) {
            return None;
        }
        let result = (|| {
            for child in self.model.owned(meta_def).to_vec() {
                if !self.member_name_matches(child, "baseType") {
                    continue;
                }
                let node = self.source.get(&child)?.clone();
                let value = node.children().find(|c| c.kind() == SyntaxKind::VALUE)?;
                let operand = value
                    .descendants()
                    .find(|c| matches!(c.kind(), SyntaxKind::NAME_REF | SyntaxKind::PATH_EXPR))?;
                let segments = operand_segments(&operand);
                if segments.is_empty() {
                    return None;
                }
                return self.resolve_from(child, &segments);
            }
            None
        })();
        self.in_progress.remove(&meta_def);
        self.semantic_bases.insert(meta_def, result);
        result
    }

    fn imports_of(&self, ns: ElementId) -> Vec<ElementId> {
        self.model
            .owned(ns)
            .iter()
            .copied()
            .filter(|c| self.model.kind(*c).is_a(ElementKind::Import))
            .collect()
    }

    fn import_target(&mut self, import: ElementId) -> Option<ImportTarget> {
        if let Some(cached) = self.imports.get(&import) {
            return cached.clone();
        }
        if !self.in_progress.insert(import) {
            return None; // import cycle
        }
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
                    }
                    ImportScope::Recursive
                }
                Some("*") => {
                    segments.pop();
                    ImportScope::Members
                }
                _ => ImportScope::Member,
            };
            let all = node
                .children_with_tokens()
                .filter_map(|e| e.into_token())
                .any(|t| t.kind() == SyntaxKind::ALL_KW);
            let target = self.resolve_from(import, &segments)?;
            let leaf = if scope == ImportScope::Member {
                segments.last().cloned()
            } else {
                None
            };
            Some(ImportTarget {
                target,
                scope,
                leaf,
                all,
            })
        })();
        self.in_progress.remove(&import);
        self.imports.insert(import, result.clone());
        result
    }

    fn alias_target(&mut self, alias: ElementId) -> Option<ElementId> {
        if let Some(cached) = self.aliases.get(&alias) {
            return *cached;
        }
        if !self.in_progress.insert(alias) {
            return None;
        }
        let result = (|| {
            let node = self.source.get(&alias)?.clone();
            let qname = node
                .children()
                .find(|c| c.kind() == SyntaxKind::QUALIFIED_NAME)?;
            let segments = name_segments(&qname);
            self.resolve_from(alias, &segments)
        })();
        self.in_progress.remove(&alias);
        self.aliases.insert(alias, result);
        result
    }

    /// Create the relationship element for one resolved target.
    fn reify(&mut self, elem: ElementId, is_definition: bool, part: SyntaxKind, target: ElementId) {
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
            // relationship_parts only yields the four kinds above plus TYPING
            _ => (ElementKind::FeatureTyping, "typedFeature", "type"),
        };
        let rel = self.model.create(kind);
        self.model.add_owned(elem, rel);
        self.try_set(rel, source_prop, Value::Ref(elem));
        self.try_set(rel, target_prop, Value::Ref(target));
    }

    /// Resolve the operands of a `connect`/`bind`/`allocate` statement and
    /// record what they point at as the connector's `relatedFeature`s, so a
    /// consumer can read the connected ends off the model.
    fn resolve_connector_ends(
        &mut self,
        id: ElementId,
        node: &SyntaxNode,
        stats: &mut ResolveStats,
    ) {
        let file = self.elem_file.get(&id).copied().unwrap_or(0);
        let mut related = Vec::new();
        for operand in end_operands(node) {
            // an operand with no identifiers resolves to nothing, which the
            // `None` arm below reports like any other unresolved end
            let segments = operand_segments(&operand);
            let range = operand.text_range();
            let name_range = operand
                .descendants_with_tokens()
                .filter_map(|e| e.into_token())
                .filter(|t| matches!(t.kind(), SyntaxKind::IDENT | SyntaxKind::UNRESTRICTED_NAME))
                .last()
                .map(|t| t.text_range())
                .unwrap_or(range);
            match self.resolve_from(id, &segments) {
                Some(target) => {
                    stats.resolved += 1;
                    self.references.push(Reference {
                        file,
                        range,
                        name_range,
                        target,
                    });
                    related.push(target);
                    self.reify_end(id, &segments);
                }
                None => {
                    stats.unresolved += 1;
                    self.unresolved.push(Unresolved {
                        file,
                        range,
                        name: segments.join("::"),
                    });
                }
            }
        }
        if !related.is_empty() {
            self.try_set(id, "relatedFeature", Value::RefList(related));
        }
    }

    /// Reify one connector end as a `Feature` whose `chainingFeature` holds
    /// what each segment of the operand resolved to.
    ///
    /// The final target alone cannot say which part an end belongs to --
    /// `w1.hub` and `w2.hub` resolve to the same port of the same type --
    /// so the chain is what an interconnection view needs.
    fn reify_end(&mut self, connector: ElementId, segments: &[String]) {
        let mut chain = Vec::new();
        // the full path already resolved, so every prefix normally does too
        for depth in 1..=segments.len() {
            if let Some(step) = self.resolve_from(connector, &segments[..depth]) {
                chain.push(step);
            }
        }
        let end = self.model.create(ElementKind::Feature);
        self.model.add_owned(connector, end);
        self.try_set(end, "chainingFeature", Value::RefList(chain));
    }

    fn try_set(&mut self, id: ElementId, prop: &str, value: Value) {
        if self.model.kind(id).feature(prop).is_some() {
            self.model.set(id, prop, value);
        }
    }
}

/// The library types every definition/usage of a given metaclass implicitly
/// specializes (KerML §7 / SysML §9 semantic library mappings, abridged:
/// only what inherited-member lookup needs). Targets that are not loaded in
/// the workspace are silently skipped.
fn implicit_supertype(kind: ElementKind) -> &'static [&'static str] {
    use ElementKind::*;
    match kind {
        PartDefinition | PartUsage => &["Parts::Part"],
        ItemDefinition | ItemUsage => &["Items::Item"],
        AttributeDefinition | AttributeUsage | EnumerationDefinition | EnumerationUsage => {
            &["Base::DataValue"]
        }
        PortDefinition | PortUsage => &["Ports::Port"],
        ConnectionDefinition | ConnectionUsage => {
            &["Connections::BinaryConnection", "Connections::Connection"]
        }
        InterfaceDefinition | InterfaceUsage => &["Interfaces::BinaryInterface"],
        AllocationDefinition | AllocationUsage => &["Allocations::Allocation"],
        ActionDefinition | ActionUsage | PerformActionUsage => &["Actions::Action"],
        SendActionUsage => &["Actions::SendAction"],
        // the library calls TransitionAction "the base type of all
        // TransitionUsages"; it owns accepter and effect
        TransitionUsage => &["States::StateTransitionAction", "Actions::TransitionAction"],
        AcceptActionUsage => &["Actions::AcceptAction"],
        CalculationDefinition | CalculationUsage => &["Calculations::Calculation"],
        StateDefinition | StateUsage | ExhibitStateUsage => &["States::StateAction"],
        ConstraintDefinition | ConstraintUsage | AssertConstraintUsage => {
            &["Constraints::ConstraintCheck"]
        }
        RequirementDefinition | RequirementUsage | SatisfyRequirementUsage => {
            &["Requirements::RequirementCheck"]
        }
        ConcernDefinition | ConcernUsage => &["Requirements::ConcernCheck"],
        CaseDefinition | CaseUsage => &["Cases::Case"],
        AnalysisCaseDefinition | AnalysisCaseUsage => &["AnalysisCases::AnalysisCase"],
        VerificationCaseDefinition | VerificationCaseUsage => {
            &["VerificationCases::VerificationCase"]
        }
        UseCaseDefinition | UseCaseUsage | IncludeUseCaseUsage => &["UseCases::UseCase"],
        ViewDefinition | ViewUsage => &["Views::View"],
        ViewpointDefinition | ViewpointUsage => &["Views::Viewpoint"],
        RenderingDefinition | RenderingUsage => &["Views::Rendering"],
        MetadataDefinition | MetadataUsage => &["Metadata::MetadataItem"],
        OccurrenceDefinition | OccurrenceUsage | EventOccurrenceUsage => {
            &["Occurrences::Occurrence"]
        }
        FlowDefinition | FlowUsage => &["Flows::Flow", "Flows::MessageFlow"],
        SuccessionAsUsage | Succession => &["Occurrences::HappensBefore"],
        // KerML classifiers
        Classifier => &["Base::Anything"],
        DataType => &["Base::DataValue"],
        Class => &["Occurrences::Occurrence"],
        Structure => &["Objects::Object"],
        Association => &["Links::BinaryLink", "Links::Link"],
        AssociationStructure => &["Objects::BinaryLinkObject", "Objects::LinkObject"],
        Behavior => &["Performances::Performance"],
        Function => &["Performances::Evaluation"],
        Predicate => &["Performances::BooleanEvaluation"],
        Interaction => &["Transfers::Transfer"],
        Metaclass => &["Metaobjects::Metaobject"],
        // KerML features
        Feature | Usage => &["Base::things"],
        Step => &["Performances::performances"],
        Expression => &["Performances::evaluations"],
        BooleanExpression => &["Performances::booleanEvaluations"],
        Invariant => &["Performances::trueEvaluations"],
        Connector => &["Links::links"],
        BindingConnector => &["Links::selfLinks"],
        _ => &[],
    }
}

/// The `TYPING`/`SUBSETTING`/`REDEFINITION`/`REFERENCES` parts of a
/// definition or usage node, with the name segments and range of each target.
#[allow(clippy::type_complexity)]
struct Target {
    segments: Vec<String>,
    /// whole qualified-name range
    range: TextRange,
    /// range of the final name segment (what a rename must replace)
    name_range: TextRange,
}

fn relationship_parts(node: &SyntaxNode) -> Vec<(SyntaxKind, Vec<Target>)> {
    node.children()
        .filter(|c| {
            matches!(
                c.kind(),
                SyntaxKind::TYPING
                    | SyntaxKind::SUBSETTING
                    | SyntaxKind::REDEFINITION
                    | SyntaxKind::REFERENCES
            )
        })
        .map(|part| {
            let targets = part
                .children()
                .filter(|c| c.kind() == SyntaxKind::TYPE_REF)
                .filter_map(|type_ref| {
                    let qname = type_ref
                        .children()
                        .find(|c| c.kind() == SyntaxKind::QUALIFIED_NAME)?;
                    let name_range = qname
                        .children_with_tokens()
                        .filter_map(|e| e.into_token())
                        .filter(|t| {
                            matches!(t.kind(), SyntaxKind::IDENT | SyntaxKind::UNRESTRICTED_NAME)
                        })
                        .last()
                        .map(|t| t.text_range())
                        .unwrap_or_else(|| qname.text_range());
                    Some(Target {
                        segments: name_segments(&qname),
                        range: qname.text_range(),
                        name_range,
                    })
                })
                .collect();
            (part.kind(), targets)
        })
        .collect()
}

/// For a usage introduced by `perform`/`exhibit`/`event`/`include` with a
/// direct reference operand (`perform a.b;`), the segments of that operand.
fn adapter_target_segments(node: &SyntaxNode) -> Option<Vec<String>> {
    if node.kind() != SyntaxKind::USAGE {
        return None;
    }
    let leads_with_adapter = node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| !t.kind().is_trivia())
        .is_some_and(|t| {
            matches!(
                t.kind(),
                SyntaxKind::PERFORM_KW
                    | SyntaxKind::EXHIBIT_KW
                    | SyntaxKind::EVENT_KW
                    | SyntaxKind::INCLUDE_KW
                    | SyntaxKind::SATISFY_KW
                    | SyntaxKind::ASSERT_KW
                    | SyntaxKind::ASSUME_KW
                    | SyntaxKind::REQUIRE_KW
                    | SyntaxKind::VERIFY_KW
                    | SyntaxKind::FRAME_KW
                    | SyntaxKind::NOT_KW
            )
        });
    if !leads_with_adapter {
        return None;
    }
    let operand = node
        .children()
        .find(|c| matches!(c.kind(), SyntaxKind::NAME_REF | SyntaxKind::PATH_EXPR))?;
    let segments = operand_segments(&operand);
    (!segments.is_empty()).then_some(segments)
}

/// All identifier segments within a reference operand (`a.b`, `A::B.c`).
fn operand_segments(operand: &SyntaxNode) -> Vec<String> {
    operand
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| matches!(t.kind(), SyntaxKind::IDENT | SyntaxKind::UNRESTRICTED_NAME))
        .map(|t| {
            t.text()
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
                .unwrap_or(t.text())
                .to_string()
        })
        .collect()
}

/// Record `target` as a supertype of `elem`, ignoring a self-reference and
/// a target another clause already contributed.
fn push_supertype(supers: &mut Vec<ElementId>, elem: ElementId, target: ElementId) {
    if target != elem && !supers.contains(&target) {
        supers.push(target);
    }
}

/// The operands naming a connector's or transition's ends.
///
/// A connector relates every reference it holds. A transition writes an
/// optional name of its own first (`transition off_to_on first off then
/// on`), so only the references introduced by `first`/`then` are ends.
fn end_operands(node: &SyntaxNode) -> Vec<SyntaxNode> {
    let is_reference = |kind| matches!(kind, SyntaxKind::NAME_REF | SyntaxKind::PATH_EXPR);
    if node.kind() != SyntaxKind::CONTROL_STMT {
        return node.children().filter(|c| is_reference(c.kind())).collect();
    }
    let mut out = Vec::new();
    let mut after_keyword = false;
    for element in node.children_with_tokens() {
        match element.as_token() {
            Some(token) if token.kind().is_trivia() => {}
            // only a reference written directly after `first`/`then` is an
            // end. Any other keyword in between starts a declaration --
            // `then accept sig after ...`, `then timeslice bobDriving` --
            // whose name is not something to resolve.
            Some(token) => {
                after_keyword = matches!(token.kind(), SyntaxKind::FIRST_KW | SyntaxKind::THEN_KW);
            }
            None => {
                let child = element.into_node().expect("element is a node");
                if after_keyword && is_reference(child.kind()) {
                    out.push(child);
                }
                after_keyword = false;
            }
        }
    }
    out
}

/// Segments of each `#keyword` prefix on a definition/usage node.
fn prefix_metadata_segments(node: &SyntaxNode) -> Vec<Vec<String>> {
    node.children()
        .filter(|c| c.kind() == SyntaxKind::PREFIX_METADATA)
        .filter_map(|prefix| {
            let qname = prefix
                .children()
                .find(|c| c.kind() == SyntaxKind::QUALIFIED_NAME)?;
            let segments = name_segments(&qname);
            (!segments.is_empty()).then_some(segments)
        })
        .collect()
}

/// Name segments of a `QUALIFIED_NAME` node (quotes stripped; `$` and
/// wildcards kept as segments).
fn name_segments(qname: &SyntaxNode) -> Vec<String> {
    qname
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| {
            matches!(
                t.kind(),
                SyntaxKind::IDENT
                    | SyntaxKind::UNRESTRICTED_NAME
                    | SyntaxKind::DOLLAR
                    | SyntaxKind::STAR
                    | SyntaxKind::STAR_STAR
            )
        })
        .map(|t| {
            t.text()
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
                .unwrap_or(t.text())
                .to_string()
        })
        .collect()
}

fn collect_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("sysml" | "kerml")
        ) {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved_workspace(files: &[(&str, &str)]) -> (Workspace, ResolveStats) {
        let mut ws = Workspace::new();
        for (name, text) in files {
            ws.add_file(*name, text);
        }
        let stats = ws.resolve_all();
        (ws, stats)
    }

    #[test]
    fn resolves_within_and_across_packages() {
        let (ws, stats) = resolved_workspace(&[
            (
                "lib.sysml",
                "package Base { abstract part def Thing; attribute def Real; }",
            ),
            (
                "app.sysml",
                "package App {\n  import Base::*;\n  part def Vehicle :> Thing {\n    attribute mass : Real;\n  }\n  part car : Vehicle {\n    attribute :>> mass;\n  }\n}",
            ),
        ]);
        let report = format!("unresolved: {:?}", ws.unresolved());
        assert_eq!(
            stats,
            ResolveStats {
                resolved: 4,
                unresolved: 0
            },
            "{report}"
        );
        // relationships were reified
        let model = ws.model();
        let count = |k: ElementKind| model.ids().filter(|id| model.kind(*id) == k).count();
        assert_eq!(count(ElementKind::Subclassification), 1);
        assert_eq!(count(ElementKind::FeatureTyping), 2);
        assert_eq!(count(ElementKind::Redefinition), 1);
    }

    #[test]
    fn resolves_aliases_and_qualified_paths() {
        let (ws, stats) = resolved_workspace(&[(
            "m.sysml",
            "package P {\n  part def Engine;\n  alias Motor for Engine;\n}\npackage Q {\n  part e : P::Motor;\n  part f : $::P::Engine;\n}",
        )]);
        assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
        assert_eq!(stats.resolved, 2);
    }

    #[test]
    fn resolves_feature_chains_through_typing() {
        let (ws, stats) = resolved_workspace(&[(
            "m.sysml",
            "package P {\n  attribute def Real;\n  part def Engine { attribute mass : Real; }\n  part def Vehicle { part eng : Engine; }\n  part v : Vehicle {\n    attribute :>> eng.mass;\n  }\n}",
        )]);
        assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
    }

    #[test]
    fn resolves_reexports_through_public_import_chains() {
        let (ws, stats) = resolved_workspace(&[
            ("a.sysml", "package A { part def Widget; }"),
            ("b.sysml", "package B { public import A::*; }"),
            ("c.sysml", "package C { part w : B::Widget; }"),
        ]);
        assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
    }

    #[test]
    fn private_imports_do_not_reexport() {
        // imports are private by default: A::Widget is usable inside B but
        // not reachable as B::Widget
        let (_ws, stats) = resolved_workspace(&[
            ("a.sysml", "package A { part def Widget; }"),
            (
                "b.sysml",
                "package B { import A::*; part inside : Widget; }",
            ),
            ("c.sysml", "package C { part w : B::Widget; }"),
        ]);
        assert_eq!(stats.resolved, 1); // inside : Widget
        assert_eq!(stats.unresolved, 1); // B::Widget
    }

    #[test]
    fn private_members_are_hidden_externally_but_not_inherited() {
        let (ws, stats) = resolved_workspace(&[(
            "m.sysml",
            "package P {\n  part def Base { private attribute secret : Real; attribute open : Real; }\n  attribute def Real;\n}\npackage Q {\n  part x : P::Base { attribute :>> open; }\n  part y { attribute s : P::Base::secret; }\n}",
        )]);
        // secret is not reachable through the external qualified path
        assert_eq!(stats.unresolved, 1, "unresolved: {:?}", ws.unresolved());
        assert_eq!(ws.unresolved()[0].name, "P::Base::secret");
    }

    #[test]
    fn unnamed_return_is_result() {
        let (ws, stats) = resolved_workspace(&[(
            "k.kerml",
            "package K {\n  datatype Real { feature dimension : Real; }\n  function F { return : Real; }\n  feature d : F::result::dimension;\n}",
        )]);
        assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
    }

    #[test]
    fn reports_unresolved_names() {
        let (ws, stats) = resolved_workspace(&[("m.sysml", "package P { part x : NoSuchThing; }")]);
        assert_eq!(stats.unresolved, 1);
        assert_eq!(ws.unresolved()[0].name, "NoSuchThing");
    }

    #[test]
    fn semantic_metadata_user_keywords() {
        let (ws, stats) = resolved_workspace(&[
            (
                "lib.sysml",
                "library package Lib {\n  attribute def Real;\n  metadata def SemanticMetadata { attribute baseType; }\n  part def CauseBase { attribute probability : Real; }\n  part causes : CauseBase;\n  metadata def cause :> SemanticMetadata {\n    :>> baseType = causes meta SysML::Usage;\n  }\n}",
            ),
            (
                "m.sysml",
                "package M {\n  import Lib::*;\n  #cause 'battery old' {\n    :>> probability = 0.01;\n  }\n}",
            ),
        ]);
        assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
    }

    #[test]
    fn import_all_overrides_visibility() {
        let (ws, stats) = resolved_workspace(&[(
            "m.sysml",
            "package P { private part def Hidden; }\npackage Q {\n  public import all P::*;\n  part h : Hidden;\n}\npackage R { part h2 : Q::Hidden; }",
        )]);
        assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
    }

    #[test]
    fn perform_and_assert_targets_contribute_members() {
        let (ws, stats) = resolved_workspace(&[(
            "m.sysml",
            "package P {\n  attribute def Real;\n  action def Collect { in attribute sample : Real; }\n  action collectData : Collect;\n  part scale {\n    perform collectData {\n      in :>> sample;\n    }\n  }\n  constraint massLimit { attribute margin : Real; }\n  assert not massLimit { :>> margin = 1.0; }\n}",
        )]);
        assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
    }

    #[test]
    fn self_reference_resolves_to_self() {
        let (ws, stats) = resolved_workspace(&[("m.sysml", "package P { part p4 :> p4; }")]);
        assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
    }

    #[test]
    fn kerml_dialect_and_short_names() {
        let (ws, stats) = resolved_workspace(&[(
            "k.kerml",
            "package K {\n  classifier <B> Base;\n  classifier Derived :> B;\n  feature f : Derived;\n}",
        )]);
        assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
    }
}

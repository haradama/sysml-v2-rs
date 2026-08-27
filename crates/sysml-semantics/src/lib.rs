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
//! It resolves the names written inside expressions too -- the body of a
//! `require constraint { ... }`, the result of a `calc`, the value after
//! `=`. The model keeps an expression as the text the author wrote rather
//! than as a tree of elements, so those names are read off the syntax and
//! looked up from the element the expression belongs to, which is the
//! scope the language gives them.
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

use sysml_model::{build_into, ElementId, ElementKind, Model, Role, Value, Vis};
use sysml_syntax::{
    is_name_chain, parse_dialect, Dialect, Parse, SyntaxKind, SyntaxNode, TextRange,
};

/// Whether a resolution pass replaces what was found about the files it
/// touches, or adds to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Clear {
    TheseFiles,
    Nothing,
}

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

/// What is wrong with a model. The two are kept apart because a file
/// that does not parse has no names worth resolving: anything said about
/// them is about the tree the parser guessed at, not the one written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Findings {
    /// What the parser could not read.
    pub syntax: Vec<Finding>,
    /// References that resolve to nothing.
    pub names: Vec<Finding>,
}

/// One thing wrong, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// Index of the file (in insertion order) it is in.
    pub file: usize,
    pub range: TextRange,
    /// The parser's complaint, or the name that resolved to nothing.
    pub what: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResolveStats {
    pub resolved: usize,
    pub unresolved: usize,
    /// How many times an import or an alias had to be worked out from
    /// its path rather than recalled. A resolver that remembers does
    /// this about once per import; one that has stopped remembering does
    /// it once per import per reference, which is the difference between
    /// a millisecond and half a minute on a forty-line file.
    pub lookups: u64,
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
    /// The imports and aliases whose targets are being worked out, from
    /// the outermost in. An import naturally consults itself while
    /// resolving its own path, so a guard that turns away the innermost
    /// entry says nothing; one that turns away an outer entry does.
    resolving: Vec<ElementId>,
    /// Bumped when the `in_progress` guard turns away an import or an
    /// alias other than the one being worked out. Those two answer
    /// `None` for something they know nothing about yet, so a failure
    /// computed while this moved is an artifact of the recursion rather
    /// than the truth. The other guards settle for an incomplete answer
    /// and cache it, so what they return is at least the same every time.
    blocked: u64,
    /// Imports and aliases whose cached failure was reached through such
    /// a guard. The failure is remembered while the outermost lookup
    /// runs -- ten unresolved wildcard imports in one package consult
    /// each other, and without memory that search is exponential -- and
    /// forgotten once it ends, so a later lookup may still find them.
    provisional: HashSet<ElementId>,
    /// Counts what `ResolveStats::lookups` reports.
    lookups: u64,
    /// What each segment of the last resolved qualified name landed on.
    /// `Classes::A` names two things, and an editor asked to rename the
    /// first of them has to know that it was named here at all.
    chain: Vec<ElementId>,
    /// How many `resolve_from` calls are on the stack. One reference is
    /// one outermost call, and the provisional failures live exactly
    /// that long.
    depth: usize,
    imports: HashMap<ElementId, Option<ImportTarget>>,
    aliases: HashMap<ElementId, Option<ElementId>>,
    visibilities: HashMap<ElementId, Vis>,
    semantic_bases: HashMap<ElementId, Vec<ElementId>>,
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
            resolving: Vec::new(),
            blocked: 0,
            provisional: HashSet::new(),
            lookups: 0,
            chain: Vec::new(),
            depth: 0,
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
        let paths = model_files(dir);
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

    /// Everything wrong with `files` (all of them when empty), kept in
    /// the two kinds a reader wants apart.
    ///
    /// Three front ends ask this -- the command line, the language
    /// server and the MCP server -- and they used to work it out for
    /// themselves. One of them forgot the syntax half, so `sysml check`
    /// reported a file that does not parse as sound. Whether to go on to
    /// the names when the syntax is broken is a judgement each of them
    /// makes: an editor shows both while you type, a batch check stops.
    /// What must not differ is what there is to show.
    pub fn findings(&self, files: &[usize]) -> Findings {
        let wanted = |file: usize| files.is_empty() || files.contains(&file);
        let mut syntax = Vec::new();
        for file in 0..self.file_count() {
            if !wanted(file) {
                continue;
            }
            for error in self.file_parse(file).errors() {
                syntax.push(Finding {
                    file,
                    range: error.range,
                    what: error.message.clone(),
                });
            }
        }
        let names = self
            .unresolved()
            .iter()
            .filter(|u| wanted(u.file))
            .map(|u| Finding {
                file: u.file,
                range: u.range,
                what: u.name.clone(),
            })
            .collect();
        Findings { syntax, names }
    }

    /// All references resolving to `target`.
    /// Whether `elem` is an `alias X for Y;`. A name reached through one
    /// resolves to what it stands for, so nothing records that the alias
    /// was the way in -- which is why renaming one cannot be offered.
    pub fn is_alias(&self, elem: ElementId) -> bool {
        self.source
            .get(&elem)
            .is_some_and(|node| node.kind() == SyntaxKind::ALIAS)
    }

    /// Everything that answers to the same name as `elem` because it
    /// redefines (or references) it without declaring a name of its own:
    /// `part l : Logical { part :>> component; }` gives `component` a
    /// second home, and `l.component` names that one. A rename that
    /// stops at the declaration leaves those mentions behind.
    pub fn named_after(&self, elem: ElementId) -> Vec<ElementId> {
        let mut found = Vec::new();
        let mut queue = vec![elem];
        let mut seen: HashSet<ElementId> = std::iter::once(elem).collect();
        while let Some(at) = queue.pop() {
            // Asked of the redefining side, which owns the relationship:
            // that way there is no side of it to be missing.
            for heir in self.model.ids() {
                // one that named itself is its own name from here on
                if self.model.get(heir, "declaredName").is_some() {
                    continue;
                }
                let borrows = self.model.owned(heir).iter().any(|&rel| {
                    let to = match self.model.kind(rel) {
                        ElementKind::Redefinition => "redefinedFeature",
                        ElementKind::ReferenceSubsetting => "referencedFeature",
                        _ => return false,
                    };
                    self.model.get(rel, to) == Some(&Value::Ref(at))
                });
                if borrows && seen.insert(heir) {
                    found.push(heir);
                    queue.push(heir);
                }
            }
        }
        found
    }

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

    /// The named elements `query` finds, best first: what was asked for
    /// exactly, then what starts with it, then what merely contains it,
    /// and within each the shorter name before the longer. Searching for
    /// `Natural` and being handed two SI units before
    /// `ScalarValues::Natural` is the difference between a useful answer
    /// and one that has to be read through.
    ///
    /// An empty query finds everything, which is what a symbol picker
    /// opens with. Asked twice, it answers the same: names that tie are
    /// left in the order the model holds them.
    ///
    /// Both the language server's symbol search and the MCP server's
    /// library search are this; they used to sort differently, and only
    /// one of them put an exact match first.
    pub fn search_names(&self, query: &str, limit: usize) -> Vec<ElementId> {
        let needle = query.to_lowercase();
        let mut found: Vec<(u8, usize, ElementId)> = self
            .named_elements()
            .filter_map(|(id, name)| {
                let lowered = name.to_lowercase();
                let rank = if lowered == needle {
                    0
                } else if lowered.starts_with(&needle) {
                    1
                } else if lowered.contains(&needle) {
                    2
                } else {
                    return None;
                };
                Some((rank, name.len(), id))
            })
            .collect();
        found.sort_by_key(|&(rank, length, _)| (rank, length));
        found.truncate(limit);
        found.into_iter().map(|(_, _, id)| id).collect()
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
                        // `import Q::**` reaches what is nested in Q as
                        // well, which is how `class Z :> F;` finds
                        // `Q::Q2::F`. Offering only Q's own members left
                        // the modeller typing blind a name the model
                        // resolves -- lookup has always followed it there.
                        if imp.scope == ImportScope::Recursive {
                            for desc in self.model.descendants(imp.target) {
                                if self.visible(desc, target_access)
                                    && !self.model.kind(desc).is_a(ElementKind::Relationship)
                                {
                                    if let Some(name) = self.model.name(desc) {
                                        out.push((name.to_string(), self.model.kind(desc)));
                                    }
                                }
                            }
                        }
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

    /// A qualified name as it was written, for a message: the root is
    /// held as an empty segment, and reads as `$`.
    fn spell(segments: &[String]) -> String {
        segments
            .iter()
            .map(|s| if s.is_empty() { "$" } else { s.as_str() })
            .collect::<Vec<_>>()
            .join("::")
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
        self.resolve_ids(&ids, Clear::TheseFiles)
    }

    /// Resolve only elements belonging to the given files (imports,
    /// supertypes etc. from other files are still resolved on demand).
    /// Resolve `files`, and then whatever they turned out to reach,
    /// until nothing new is reached.
    ///
    /// A reader of the model -- a drawing, a generator -- follows the
    /// relationships resolution reifies, so a type that was never
    /// resolved has no members to show and no supertype to inherit
    /// from. Resolving every loaded file answers that by doing far more
    /// work than the question needs: a standard library is thousands of
    /// references, of which a model uses a handful. This resolves the
    /// files asked for, sees which files the answers landed in, and
    /// goes round again.
    pub fn resolve_reached(&mut self, files: &[usize]) -> ResolveStats {
        let mut stats = self.resolve_files(files);
        let mut done: HashSet<ElementId> = self
            .model
            .ids()
            .filter(|id| self.elem_file.get(id).is_some_and(|f| files.contains(f)))
            .collect();
        loop {
            let mut fresh = Vec::new();
            for reference in &self.references {
                if done.contains(&reference.target) {
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

    pub fn resolve_files(&mut self, files: &[usize]) -> ResolveStats {
        let ids: Vec<ElementId> = self
            .model
            .ids()
            .filter(|id| self.elem_file.get(id).is_some_and(|f| files.contains(f)))
            .collect();
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
        }

        let mut stats = ResolveStats::default();
        let began = self.lookups;
        for &id in ids {
            let Some(node) = self.source.get(&id).cloned() else {
                continue;
            };
            if matches!(
                node.kind(),
                SyntaxKind::CONNECTOR_STMT | SyntaxKind::CONTROL_STMT
            ) {
                self.resolve_connector_ends(id, &node, &mut stats);
                self.resolve_trigger_type(id, &node, &mut stats);
                continue;
            }
            // `dependency use from A to B;` is a statement of its own,
            // and the names on either side of `to` are references like
            // any other
            if node.kind() == SyntaxKind::DEPENDENCY {
                self.resolve_dependency(id, &node, &mut stats);
                continue;
            }
            // `@rust { ... }` types the metadata usage by its metadata
            // definition; resolving it is what lets the `:>> attribute`
            // settings inside reach the definition's attributes
            if node.kind() == SyntaxKind::METADATA_ANNOTATION {
                self.resolve_metadata_typing(id, &node, &mut stats);
                continue;
            }
            // `import P1::*;` names P1, and an editor renaming P1 has to
            // be told so -- otherwise the rename leaves the import
            // pointing at a package that is no longer there. Imports are
            // not counted among the resolved references, which are the
            // explicit relationship targets; this only says where they
            // were written.
            if matches!(node.kind(), SyntaxKind::IMPORT | SyntaxKind::EXPOSE) {
                self.record_import(id, &node);
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
            // a payload carries a typing of its own -- `flow f of Fuel`
            // -- and is an element the builder made, so it resolves like
            // any declaration
            if !matches!(
                node.kind(),
                SyntaxKind::DEFINITION | SyntaxKind::USAGE | SyntaxKind::PAYLOAD
            ) {
                continue;
            }
            // `attribute pin : PinNumber = ledPinNumber;` -- the value is
            // an expression like any other, and the name in it is a
            // reference like any other
            for clause in node
                .children()
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
                    match self.resolve_from(id, &t.segments) {
                        Some(target) => {
                            stats.resolved += 1;
                            let file = self.elem_file.get(&id).copied().unwrap_or(0);
                            self.record(file, t.range, t.name_range, &t.at, target);
                            self.reify(id, is_definition, part_kind, target);
                        }
                        None => {
                            stats.unresolved += 1;
                            self.unresolved.push(Unresolved {
                                file: self.elem_file.get(&id).copied().unwrap_or(0),
                                range: t.range,
                                name: Self::spell(&t.segments),
                            });
                        }
                    }
                }
            }
            // `perform w;`, `exhibit s;`, `assert c;`, `include u;` --
            // `PerformActionUsageDeclaration : PerformActionUsage = (
            // ownedRelationship += OwnedReferenceSubsetting ... )`. The
            // reference is what the usage is *about*, and without it the
            // model says only that something is performed.
            //
            // A name that does not resolve is left alone rather than
            // reported: `satisfy requirement viewpointConformance by
            // that;` writes the same shape and *declares* that name, so
            // a finding here would be a false one.
            //
            // `satisfy r by p;` and `verify r;` have resolvers of their
            // own below, which record the same operand: recording it here
            // too would give a rename two edits over the one name.
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
                    self.record(file, range, name_range, &operand_ranges(&operand), target);
                    self.reify(id, false, SyntaxKind::REFERENCES, target);
                }
            }
            // `connection c : L connect a to b;` is written as a usage, so
            // its ends arrive here rather than through a connector statement
            if self.model.kind(id).is_a(ElementKind::ConnectorAsUsage) {
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
        stats.lookups = self.lookups - began;
        stats
    }

    /// Resolve the metadata definition an `@name { ... }` usage is typed
    /// by, and reify the typing so the settings inside the body resolve
    /// against the definition's attributes.
    fn resolve_metadata_typing(
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
                stats.unresolved += 1;
                self.unresolved.push(Unresolved {
                    file: self.elem_file.get(&usage).copied().unwrap_or(0),
                    range: target.range,
                    name: Self::spell(&target.segments),
                });
            }
        }
    }

    /// Resolve a qualified name starting from the scope that contains
    /// `elem`. `elem` itself is excluded from name matches: a feature's own
    /// (effective) name must not shadow the inherited feature it redefines.
    pub fn resolve_from(&mut self, elem: ElementId, segments: &[String]) -> Option<ElementId> {
        if segments.is_empty() {
            return None;
        }
        self.depth += 1;
        let exclude = Some(elem);
        let found = self.resolve_segments(elem, segments, exclude).or_else(|| {
            // A self-reference (`part p4 :> p4;`) is a legal name even
            // though a declaration cannot shadow the feature it
            // redefines, so the name is looked up once more with the
            // declaration itself allowed to answer.
            let hit = self.resolve_segments(elem, segments, None)?;
            // But a feature with no name of its own answers to the name
            // of what it redefines, and that is the very thing being
            // looked up here. Letting it match itself would make the
            // answer its own premise: `attribute :>> nothingHere;` would
            // resolve, and a model naming something that exists nowhere
            // would be reported as sound.
            let names_itself = self.model.name(elem) == segments.last().map(String::as_str);
            (hit != elem || names_itself).then_some(hit)
        });
        self.depth -= 1;
        self.forget_provisional();
        found
    }

    /// Record a resolved reference, and with it every earlier segment of
    /// the qualified name that got there. `Classes::A` names the package
    /// as well as the class, and an editor renaming the package has to
    /// be told where it was named -- otherwise it rewrites the
    /// declaration and leaves the mentions of it behind.
    /// Where an import's path was written, and what each part of it
    /// names. The wildcard at the end names nothing.
    fn record_import(&mut self, import: ElementId, node: &SyntaxNode) {
        let (mut segments, mut at) = node
            .children()
            .find(|c| c.kind() == SyntaxKind::QUALIFIED_NAME)
            .map(|qname| (name_segments(&qname), segment_ranges(&qname)))
            .unwrap_or_default();
        while matches!(segments.last().map(String::as_str), Some("*" | "**")) {
            segments.pop();
            at.pop();
        }
        let (Some(last), false) = (at.last().copied(), segments.is_empty()) else {
            return;
        };
        if let Some(target) = self.resolve_from(import, &segments) {
            let file = self.elem_file.get(&import).copied().unwrap_or(0);
            self.record(file, last, last, &at, target);
        }
    }

    fn record(
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
            current = self.lookup(current, seg, Access::External, true, exclude)?;
            walked.push(current);
        }
        self.chain = walked;
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
        // Direct members, aliases, and everything nested under an `end`
        // member. An end's body can nest further -- an association whose
        // end holds a feature which itself holds the one being named --
        // and the whole of it belongs to the connector's scope, so a
        // subtype naming it inherits the lot.
        let mut candidates = self.model.owned(ns).to_vec();
        for child in self.model.owned(ns).to_vec() {
            if self.is_end_member(child) {
                candidates.extend(self.model.descendants(child));
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
            // Two supertypes may both answer to the name, and one of
            // their answers may be a refinement of the other's --
            // `classifier C specializes A, B` where `B` redefines
            // `A::f`. The refinement is the member, whichever order the
            // supertypes happen to be written in, so the answer is the
            // candidate no other candidate specializes.
            let mut hits = Vec::new();
            for sup in self.supertypes_of(ns) {
                if let Some(hit) =
                    self.lookup_guarded(sup, name, sub_access, true, false, exclude, guard)
                {
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
            // an `@name` metadata usage inherits the definition's members
            if let Some(target) = metadata_target(&node) {
                if let Some(def) = self.resolve_from(elem, &target.segments) {
                    push_supertype(&mut supers, elem, def);
                }
            }
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
                    for base in self.semantic_bases(meta_def) {
                        push_supertype(&mut supers, elem, base);
                    }
                }
            }
        }
        // A feature can redeclare an inherited one by naming it the
        // same: `in p { ... }` inside a specialization stands for the
        // `p` its supertype declares, and inherits what that one has.
        // Only where nothing else was written about it -- an explicit
        // typing or subsetting is the whole story.
        if supers.is_empty() {
            if let (Some(name), Some(owner)) = (
                self.model.name(elem).map(String::from),
                self.model.owner(elem),
            ) {
                for sup in self.supertypes_of(owner) {
                    for sibling in self.model.owned(sup).to_vec() {
                        if self.model.name(sibling) == Some(name.as_str()) {
                            push_supertype(&mut supers, elem, sibling);
                        }
                    }
                }
            }
        }
        // `variant manualTransmission;` names one of the usages the
        // model already has; `variant part v;` declares a new one. The
        // difference is whether a kind keyword was written, and the
        // reference form has to bring what it names along with it.
        if supers.is_empty() && self.model.member_role(elem) == Some(Role::Variant) {
            let bare = self.source.get(&elem).is_some_and(|node| {
                !node
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .any(|t| t.kind().is_def_kind_kw())
            });
            if let (true, Some(name)) = (bare, self.model.name(elem).map(String::from)) {
                if let Some(target) = self.resolve_from(elem, &[name]) {
                    push_supertype(&mut supers, elem, target);
                }
            }
        }
        // A usage has at most one `subject` and one `objective`, so the
        // one it writes stands for the one its type declares -- which is
        // what `objective { verify x :>> massRequirement; }` redefines a
        // member of. Nothing says so in the text; the roles do.
        if let Some(role) = self.model.member_role(elem) {
            if matches!(role, Role::Subject | Role::Objective) {
                let owners: Vec<ElementId> = self.model.owner(elem).into_iter().collect();
                for owner in owners {
                    for sup in self.supertypes_of(owner) {
                        for sibling in self.model.owned(sup).to_vec() {
                            if self.model.member_role(sibling) == Some(role) {
                                push_supertype(&mut supers, elem, sibling);
                            }
                        }
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
    fn semantic_bases(&mut self, meta_def: ElementId) -> Vec<ElementId> {
        if let Some(cached) = self.semantic_bases.get(&meta_def) {
            return cached.clone();
        }
        if !self.in_progress.insert(meta_def) {
            return Vec::new();
        }
        let mut found = Vec::new();
        for child in self.model.owned(meta_def).to_vec() {
            if !self.member_name_matches(child, "baseType") {
                continue;
            }
            // `= if p ? A meta T else B meta T` names two bases, and
            // which one a given element gets is decided by an expression
            // this resolver does not evaluate. Both are taken: a name
            // that either of them declares is one the model can mean.
            let operands: Vec<SyntaxNode> = self
                .source
                .get(&child)
                .cloned()
                .into_iter()
                .flat_map(|node| node.children())
                .filter(|c| c.kind() == SyntaxKind::VALUE)
                .flat_map(|value| value.descendants())
                .filter(|c| matches!(c.kind(), SyntaxKind::NAME_REF | SyntaxKind::PATH_EXPR))
                .collect();
            for operand in operands {
                let segments = operand_segments(&operand);
                if segments.is_empty() {
                    continue;
                }
                found.extend(self.resolve_from(child, &segments));
            }
            break;
        }
        self.in_progress.remove(&meta_def);
        self.semantic_bases.insert(meta_def, found.clone());
        found
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
            match imp.scope {
                ImportScope::Member => {
                    if seen.insert(imp.target) {
                        out.push(imp.target);
                    }
                }
                ImportScope::Members => {
                    self.visible_members_into(imp.target, access, &mut out, &mut seen);
                }
                ImportScope::Recursive => {
                    self.visible_members_into(imp.target, access, &mut out, &mut seen);
                    for below in self.model.descendants(imp.target) {
                        if self.model.kind(below).is_a(ElementKind::Namespace)
                            && !self.model.kind(below).is_a(ElementKind::Relationship)
                            && self.visible(below, access)
                        {
                            self.visible_members_into(below, access, &mut out, &mut seen);
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
        out: &mut Vec<ElementId>,
        seen: &mut HashSet<ElementId>,
    ) {
        for child in self.model.owned(ns).to_vec() {
            if self.model.kind(child).is_a(ElementKind::Import)
                || self.model.kind(child).is_a(ElementKind::Relationship)
            {
                continue;
            }
            if self.visible(child, access) && seen.insert(child) {
                out.push(child);
            }
        }
    }

    /// Reify the implied specializations resolution reasons with, as the
    /// relationship elements the standard stores.
    ///
    /// Every definition and usage inherits from a semantic-library base --
    /// a `part def` from `Parts::Part`, a feature from `Base::things`, an
    /// element under a user-defined `#keyword` from that keyword's base --
    /// and resolution has always used those bases without materializing
    /// them. This pass writes each one the model does not already reach
    /// explicitly as an owned `Subclassification` (classifiers) or
    /// `Subsetting` (features) with `isImplied` set, the way the standard
    /// interchanges them. Elements that gained one are marked
    /// `isImpliedIncluded`.
    ///
    /// Call after [`resolve_all`](Workspace::resolve_all); bases that do
    /// not resolve (no library loaded) are skipped. Running the pass again
    /// adds nothing: what the first run wrote is reachable now. Returns
    /// how many relationships were written.
    pub fn materialize_implied(&mut self) -> usize {
        let mut written = 0;
        for elem in self.model.ids().collect::<Vec<_>>() {
            let kind = self.model.kind(elem);
            // relationships do not specialize; only types inherit
            if kind.is_a(ElementKind::Relationship) || !kind.is_a(ElementKind::Type) {
                continue;
            }
            let mut implied: Vec<&str> = implicit_supertype(kind).to_vec();
            if kind.is_a(ElementKind::Feature) && !implied.contains(&"Base::things") {
                implied.push("Base::things");
            }
            let mut bases = Vec::new();
            for path in implied {
                let segments: Vec<String> = path.split("::").map(String::from).collect();
                if let Some(target) = self.resolve_from(elem, &segments) {
                    if target != elem && !bases.contains(&target) {
                        bases.push(target);
                    }
                }
            }
            // `#cause 'battery old' { ... }` implies the keyword's baseType
            if let Some(node) = self.source.get(&elem).cloned() {
                for segments in prefix_metadata_segments(&node) {
                    if let Some(meta_def) = self.resolve_from(elem, &segments) {
                        for base in self.semantic_bases(meta_def) {
                            if base != elem && !bases.contains(&base) {
                                bases.push(base);
                            }
                        }
                    }
                }
            }
            for base in bases {
                // implied only where nothing explicit -- or already
                // implied -- reaches the base; what this loop writes
                // counts for the bases after it
                if reaches(&self.model, elem, base) {
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

    /// Drop the provisional failures once the reference that prompted
    /// them has been answered.
    fn forget_provisional(&mut self) {
        if self.depth == 0 && self.resolving.is_empty() {
            for id in std::mem::take(&mut self.provisional) {
                self.imports.remove(&id);
                self.aliases.remove(&id);
            }
        }
    }

    fn alias_target(&mut self, alias: ElementId) -> Option<ElementId> {
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
                    self.record(file, range, name_range, &operand_ranges(&operand), target);
                    related.push(target);
                    self.reify_end(id, &segments);
                }
                None => {
                    stats.unresolved += 1;
                    self.unresolved.push(Unresolved {
                        file,
                        range,
                        name: Self::spell(&segments),
                    });
                }
            }
        }
        // `then action b;` writes no operand at all: what it flows into
        // is the declaration it wraps. That declaration belongs to the
        // enclosing scope rather than to the succession, so nothing an
        // operand search looks at holds it -- and a succession that
        // relates nothing is a step the model cannot say follows.
        if related.is_empty() && self.model.kind(id).is_a(ElementKind::ConnectorAsUsage) {
            if let Some(target) = self.wrapped_declaration(id, node) {
                let end = self.model.create(ElementKind::Feature);
                self.model.add_owned(id, end);
                self.try_set(end, "chainingFeature", Value::RefList(vec![target]));
                related.push(target);
            }
        }
        if !related.is_empty() {
            self.try_set(id, "relatedFeature", Value::RefList(related));
        }
    }

    /// The clients and suppliers of a `dependency a, b to c;`.
    ///
    /// `Dependency = 'dependency' ( Identification? 'from' )? client +=
    /// [QualifiedName] ( ',' client )* 'to' supplier += [QualifiedName]
    /// ( ',' supplier )*` -- so `to` divides the two, and a name before
    /// `from` is the dependency's own, not a client.
    fn resolve_dependency(&mut self, id: ElementId, node: &SyntaxNode, stats: &mut ResolveStats) {
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
                    stats.unresolved += 1;
                    self.unresolved.push(Unresolved {
                        file,
                        range,
                        name: Self::spell(&segments),
                    });
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

    /// A transition's `accept x : T` writes a typing that belongs to the
    /// trigger it declares, not to the transition itself.
    fn resolve_trigger_type(
        &mut self,
        transition: ElementId,
        node: &SyntaxNode,
        stats: &mut ResolveStats,
    ) {
        let declared = match self.model.get(transition, "triggerAction") {
            Some(Value::RefList(triggers)) => triggers.first().copied(),
            _ => None,
        };
        let Some(trigger) = declared else {
            return;
        };
        let file = self.elem_file.get(&transition).copied().unwrap_or(0);
        let typings = relationship_parts(node)
            .into_iter()
            .filter(|(part, _)| *part == SyntaxKind::TYPING);
        for (_, targets) in typings {
            for t in targets {
                match self.resolve_from(transition, &t.segments) {
                    Some(target) => {
                        stats.resolved += 1;
                        self.record(file, t.range, t.name_range, &t.at, target);
                        let typing = self.model.create(ElementKind::FeatureTyping);
                        self.model.add_owned(trigger, typing);
                        self.try_set(typing, "typedFeature", Value::Ref(trigger));
                        self.try_set(typing, "type", Value::Ref(target));
                    }
                    None => {
                        stats.unresolved += 1;
                        self.unresolved.push(Unresolved {
                            file,
                            range: t.range,
                            name: Self::spell(&t.segments),
                        });
                    }
                }
            }
        }
    }

    /// Resolve what `satisfy r by p;` relates: the requirement named after
    /// `satisfy` and the feature named after `by`.
    ///
    /// `satisfy requirement r : R by p;` declares the requirement inline
    /// instead of naming one, so only the `by` side is a reference there --
    /// the usage is the requirement.
    fn resolve_satisfaction(&mut self, id: ElementId, node: &SyntaxNode, stats: &mut ResolveStats) {
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
                    stats.unresolved += 1;
                    self.unresolved.push(Unresolved {
                        file,
                        range,
                        name: Self::spell(&segments),
                    });
                }
            }
        }
    }

    /// Resolve an operand that may be the implicit `self` or `that` rather
    /// than a declared name -- `satisfy requirement r by that;` means the
    /// type the assertion is written in satisfies it.
    /// Resolve what `verify r;` names, and hang it on the case.
    ///
    /// The requirement a verification case answers for is written inside
    /// its objective, several levels down from the case itself, and
    /// `verifiedRequirement` is the case's own property. Recording it
    /// there is what lets a reader of the model ask what verifies a
    /// requirement without walking back down through the objective.
    fn resolve_verification(&mut self, id: ElementId, node: &SyntaxNode, stats: &mut ResolveStats) {
        let Some(operand) = operand_after(node, SyntaxKind::VERIFY_KW) else {
            return;
        };
        let file = self.elem_file.get(&id).copied().unwrap_or(0);
        let segments = operand_segments(&operand);
        let range = operand.text_range();
        let Some(target) = self.resolve_operand(id, &segments) else {
            stats.unresolved += 1;
            self.unresolved.push(Unresolved {
                file,
                range,
                name: Self::spell(&segments),
            });
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
                let mut verified = match self.model.get(current, "verifiedRequirement") {
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

    /// Resolve every name written in an expression, from `owner`.
    ///
    /// An expression is not reified as a tree of elements -- the model
    /// keeps it as the text the author wrote -- so the names in it are
    /// read off the syntax and looked up from the element the expression
    /// belongs to. That is the scope the language gives them: the
    /// constraint of a requirement sees the requirement's subject, the
    /// result of a `calc` sees its parameters.
    fn resolve_expression(
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
                    self.record(file, range, name_range, &at, target);
                    // where the whole expression is that one name, the
                    // model reified it as a reference to a feature and
                    // this is the feature
                    if chain == *expr {
                        if let Some(reference) = self.reference_expression(owner) {
                            self.try_set(reference, "referent", Value::Ref(target));
                        }
                    }
                }
                None => {
                    stats.unresolved += 1;
                    self.unresolved.push(Unresolved {
                        file,
                        range,
                        name: Self::spell(&segments),
                    });
                }
            }
        }
    }

    /// The `FeatureReferenceExpression` an element's value was built
    /// into, where the model made one -- which it does exactly when the
    /// whole value is a name.
    fn reference_expression(&self, owner: ElementId) -> Option<ElementId> {
        let membership = self
            .model
            .owned(owner)
            .iter()
            .copied()
            .find(|&child| self.model.kind(child) == ElementKind::FeatureValue)?;
        let expression = self.model.get(membership, "value")?.as_id()?;
        (self.model.kind(expression) == ElementKind::FeatureReferenceExpression)
            .then_some(expression)
    }

    fn resolve_operand(&mut self, elem: ElementId, segments: &[String]) -> Option<ElementId> {
        if !matches!(segments, [only] if only == "self" || only == "that") {
            return self.resolve_from(elem, segments);
        }
        let mut scope = self.model.owner(elem);
        while let Some(current) = scope {
            if self.model.kind(current).is_a(ElementKind::Type) {
                return Some(current);
            }
            scope = self.model.owner(current);
        }
        None
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
/// Does `elem` already specialize `base`, walking the reified
/// specialization relationships the model holds -- the implied ones a
/// materialization pass has written included?
fn reaches(model: &Model, elem: ElementId, base: ElementId) -> bool {
    let mut queue = vec![elem];
    let mut visited = HashSet::new();
    while let Some(current) = queue.pop() {
        if !visited.insert(current) {
            continue;
        }
        for &child in model.owned(current) {
            let target = match model.kind(child) {
                ElementKind::Subclassification => "superclassifier",
                ElementKind::Subsetting => "subsettedFeature",
                ElementKind::Redefinition => "redefinedFeature",
                ElementKind::FeatureTyping => "type",
                ElementKind::ReferenceSubsetting => "referencedFeature",
                _ => continue,
            };
            if let Some(Value::Ref(target)) = model.get(child, target) {
                if *target == base {
                    return true;
                }
                queue.push(*target);
            }
        }
    }
    false
}

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
        Feature | Usage | ReferenceUsage => &["Base::things"],
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
    /// range of each segment, in order -- the earlier ones name
    /// something too
    at: Vec<TextRange>,
}

/// The range of the last identifier in a reference operand -- what a
/// rename of the thing it names rewrites, as opposed to the whole `a.b`.
fn last_name_range(operand: &SyntaxNode) -> TextRange {
    operand
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| matches!(t.kind(), SyntaxKind::IDENT | SyntaxKind::UNRESTRICTED_NAME))
        .last()
        .map(|t| t.text_range())
        .unwrap_or_else(|| operand.text_range())
}

/// Is there a `from` in this statement that `operand` stands before?
/// That name is the dependency's own, not one of its clients.
fn before_from(node: &SyntaxNode, operand: &SyntaxNode) -> bool {
    node.children_with_tokens()
        .filter(|part| part.kind() == SyntaxKind::FROM_KW)
        .any(|from| operand.text_range().end() <= from.text_range().start())
}

/// The metadata definition an `@name`/`#name` annotation names: the
/// qualified name sitting directly under the annotation node.
fn metadata_target(node: &SyntaxNode) -> Option<Target> {
    if node.kind() != SyntaxKind::METADATA_ANNOTATION {
        return None;
    }
    let qname = node
        .children()
        .find(|c| c.kind() == SyntaxKind::QUALIFIED_NAME)?;
    let name_range = qname
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| matches!(t.kind(), SyntaxKind::IDENT | SyntaxKind::UNRESTRICTED_NAME))
        .last()
        .map(|t| t.text_range())
        .unwrap_or_else(|| qname.text_range());
    Some(Target {
        segments: name_segments(&qname),
        range: qname.text_range(),
        name_range,
        at: segment_ranges(&qname),
    })
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
                        at: segment_ranges(&qname),
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
    adapter_target(node).map(|operand| operand_segments(&operand))
}

/// The operand a `perform`/`exhibit`/`assert`/... usage adapts, when it
/// names one rather than declaring it.
fn adapter_target(node: &SyntaxNode) -> Option<SyntaxNode> {
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
                    | SyntaxKind::RENDER_KW
                    | SyntaxKind::NOT_KW
            )
        });
    if !leads_with_adapter {
        return None;
    }
    let operand = node
        .children()
        .find(|c| matches!(c.kind(), SyntaxKind::NAME_REF | SyntaxKind::PATH_EXPR))?;
    (!operand_segments(&operand).is_empty()).then_some(operand)
}

/// All identifier segments within a reference operand (`a.b`, `A::B.c`).
/// The names written in an expression, each as far as it can be
/// followed.
///
/// A dotted name is taken whole -- `a.b.c` looks `a` up in scope and
/// then each step among the members of the last one's type -- so a
/// `PATH_EXPR` counts only when what it walks from is itself a name.
/// `f(x).b` gives `f` and `x` and stops: nothing in the model says what
/// `f` returns, so there is no namespace for `b` to be a member of.
fn name_chains(node: &SyntaxNode, out: &mut Vec<SyntaxNode>) {
    match node.kind() {
        // `list->select { in i; i > 2 }` declares `i` inside a body that
        // is not built into the model, so the names there have a scope
        // nothing here can see. Reporting them would be a false alarm.
        SyntaxKind::BODY_EXPR => return,
        SyntaxKind::NAME_REF => {
            out.push(node.clone());
            return;
        }
        SyntaxKind::PATH_EXPR if is_name_chain(node) => {
            out.push(node.clone());
            return;
        }
        _ => {}
    }
    for child in node.children() {
        // `f(b = 1)` names a parameter of `f`, not anything in scope here
        if node.kind() == SyntaxKind::ARG_LIST && followed_by_eq(node, &child) {
            continue;
        }
        name_chains(&child, out);
    }
}

/// Whether `=` is the next thing after `child` -- the `b` of `f(b = 1)`.
fn followed_by_eq(parent: &SyntaxNode, child: &SyntaxNode) -> bool {
    let mut after = false;
    for element in parent.children_with_tokens() {
        if element.kind().is_trivia() {
            continue;
        }
        if after {
            return element.kind() == SyntaxKind::EQ;
        }
        after = element.as_node() == Some(child);
    }
    false
}

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

/// The reference written directly after `keyword`, if the next thing is one.
fn operand_after(node: &SyntaxNode, keyword: SyntaxKind) -> Option<SyntaxNode> {
    let mut seen = false;
    for element in node.children_with_tokens() {
        match element.as_token() {
            Some(token) if token.kind().is_trivia() => {}
            Some(token) => {
                if seen {
                    return None;
                }
                seen = token.kind() == keyword;
            }
            None => {
                let child = element.into_node().expect("checked for a token above");
                if seen {
                    return matches!(child.kind(), SyntaxKind::NAME_REF | SyntaxKind::PATH_EXPR)
                        .then_some(child);
                }
            }
        }
    }
    None
}

/// The operands naming a connector's or transition's ends.
///
/// A connector relates every reference it holds. A transition writes an
/// optional name of its own first (`transition off_to_on first off then
/// on`), so only the references introduced by `first`/`then` are ends.
fn end_operands(node: &SyntaxNode) -> Vec<SyntaxNode> {
    let is_reference = |kind| matches!(kind, SyntaxKind::NAME_REF | SyntaxKind::PATH_EXPR);
    let introduces_end = match node.kind() {
        // A connector statement relates every reference it holds --
        // `bind a.p = b.p;` among them, which writes its second end as
        // a value clause rather than as another operand.
        SyntaxKind::CONNECTOR_STMT => {
            return node
                .children()
                .flat_map(|child| match child.kind() {
                    kind if is_reference(kind) => vec![child],
                    SyntaxKind::VALUE => child
                        .children()
                        .filter(|c| is_reference(c.kind()))
                        .collect(),
                    _ => Vec::new(),
                })
                .collect()
        }
        // `transition t first a ... then b` writes a name of its own first
        SyntaxKind::CONTROL_STMT => &[SyntaxKind::FIRST_KW, SyntaxKind::THEN_KW][..],
        // `connection c : L connect a to b;` and `flow f of T from a to b;`
        // declare a name and a type before the ends arrive, and
        // `succession a then b;` writes its ends around the keyword
        _ => &[
            SyntaxKind::CONNECT_KW,
            SyntaxKind::TO_KW,
            SyntaxKind::FROM_KW,
            SyntaxKind::FIRST_KW,
            SyntaxKind::THEN_KW,
        ][..],
    };
    // `a then b` and `interface a.p to b.p;` say where they start
    // before the keyword, in the place a statement writing `first`,
    // `from` or `connect` puts a name and a type instead. Only the very
    // front of the statement is that place: a clause such as `accept rs
    // : T` takes it for itself, and what such a clause declares is a
    // name of its own rather than an end.
    let says_where_first = |kind| {
        matches!(
            kind,
            SyntaxKind::FIRST_KW | SyntaxKind::FROM_KW | SyntaxKind::CONNECT_KW
        )
    };
    let leading_source = introduces_end
        .iter()
        .any(|kind| matches!(kind, SyntaxKind::THEN_KW | SyntaxKind::TO_KW))
        && !node
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| says_where_first(t.kind()));
    let mut front = node
        .children_with_tokens()
        .take_while(|element| {
            element.as_token().map_or(true, |token| {
                token.kind().is_trivia()
                    || token.kind().is_modifier_kw()
                    || token.kind().is_visibility_kw()
                    || token.kind().is_def_kind_kw()
                    || matches!(
                        token.kind(),
                        SyntaxKind::SUCCESSION_KW | SyntaxKind::TRANSITION_KW
                    )
            })
        })
        .filter_map(|element| element.into_node())
        .find(|child| is_reference(child.kind()));
    let mut out = Vec::new();
    let mut after_keyword = false;
    for element in node.children_with_tokens() {
        match element.as_token() {
            Some(token) if token.kind().is_trivia() => {}
            // only a reference written directly after one of those keywords
            // is an end. Any other keyword in between starts a declaration --
            // `then accept sig after ...`, `flow of Fuel ...` -- whose name
            // is not something to resolve.
            Some(token) => {
                if leading_source && matches!(token.kind(), SyntaxKind::THEN_KW | SyntaxKind::TO_KW)
                {
                    out.extend(front.take());
                }
                after_keyword = introduces_end.contains(&token.kind());
            }
            None => {
                let child = element.into_node().expect("element is a node");
                // `connect [1] myCart to [1] products` and `first [1]
                // paint then [1] dry` count the end before naming it,
                // and the count is not what the keyword introduced
                if child.kind() == SyntaxKind::MULTIPLICITY {
                    continue;
                }
                if after_keyword && is_reference(child.kind()) {
                    out.push(child);
                } else if after_keyword && child.kind() == SyntaxKind::PAREN_EXPR {
                    // `connect (d1, d2, d3)` relates the whole list, and
                    // the parentheses hold it rather than the statement
                    out.extend(child.children().filter(|c| is_reference(c.kind())));
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
/// Where each segment of a qualified name is written, in order.
fn segment_ranges(qname: &SyntaxNode) -> Vec<TextRange> {
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
        .map(|t| t.text_range())
        .collect()
}

/// Where each identifier of an operand is written, in order.
fn operand_ranges(operand: &SyntaxNode) -> Vec<TextRange> {
    operand
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| matches!(t.kind(), SyntaxKind::IDENT | SyntaxKind::UNRESTRICTED_NAME))
        .map(|t| t.text_range())
        .collect()
}

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
            // `$` is the root, and `'$'` is a package someone named `$`.
            // Both unquote to the same three characters, so the root is
            // spelled as a segment a declared name cannot be: an empty
            // one. A NAME token always has text.
            if t.kind() == SyntaxKind::DOLLAR {
                return String::new();
            }
            t.text()
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
                .unwrap_or(t.text())
                .to_string()
        })
        .collect()
}

/// Every `.sysml`/`.kerml` file under `dir`, in a stable order.
///
/// Separate from [`Workspace::load_dir`] because an editor has to know
/// which files a project has before deciding which of them to load: the
/// ones open in a buffer are read from the buffer, not from disk.
pub fn model_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    collect_files(dir, &mut paths);
    paths.sort();
    paths
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

    #[test]
    fn a_relationship_without_a_target_does_not_reach() {
        // a foreign model may hold a specialization that never says what
        // it specializes; the walk passes it by
        let mut model = Model::new();
        let sub = model.create(ElementKind::PartDefinition);
        let sup = model.create(ElementKind::PartDefinition);
        let dangling = model.create(ElementKind::Subclassification);
        model.add_owned(sub, dangling);
        assert!(!reaches(&model, sub, sup));
    }

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
        assert_eq!((stats.resolved, stats.unresolved), (4, 0), "{report}");
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

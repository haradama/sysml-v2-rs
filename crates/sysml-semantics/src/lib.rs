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

mod ocl;
pub mod rules;

use std::collections::{HashMap, HashSet};

use sysml_model::{build_into, ElementId, ElementKind, Model, Role, Value, Vis};
use sysml_syntax::{
    is_name_chain, parse_dialect, Dialect, Parse, SyntaxKind, SyntaxNode, TextRange,
};

/// How many namespaces deep a lookup will walk through inherited
/// members before it gives up. The corpus, standard library included,
/// never goes past a few dozen; a model that goes thousands deep would
/// take the stack down with it, so it is told its name resolves to
/// nothing instead. The parser bounds its own nesting the same way.
const MAX_INHERITANCE: usize = 512;

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

/// What is wrong with a model, in the kinds a reader wants apart. A
/// file that does not parse has no names worth resolving -- anything
/// said about them is about the tree the parser guessed at, not the one
/// written -- and a collision is not a name that failed to resolve but
/// one that resolves to something other than it looks like.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Findings {
    /// What the parser could not read.
    pub syntax: Vec<Finding>,
    /// References that resolve to nothing.
    pub names: Vec<Finding>,
    /// Root packages of the model declared under a name the standard
    /// library has already taken.
    pub collisions: Vec<Finding>,
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
    /// Every element built from the file, in the order the model holds
    /// them. A question about a position is a question about one file,
    /// and answering it by walking the whole model walks the standard
    /// library -- forty thousand elements -- per keystroke.
    elements: Vec<ElementId>,
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
    /// How deep the walk through inherited members is. Nothing else
    /// bounds it: what a namespace specializes is written in the
    /// model, and the model can say it thousands of times over.
    walking: usize,
    /// What was worked out while that count moved: an import or an
    /// alias that failed, a supertype list that came out short. The
    /// answer is remembered while the outermost lookup runs -- ten
    /// unresolved wildcard imports in one package consult each other,
    /// and without memory that search is exponential -- and forgotten
    /// once it ends, so the next one works it out from what the model
    /// really holds.
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
    /// How many lookups have come back empty-handed. A conclusion
    /// drawn while this moved rests on a name that was not there, and
    /// a file added afterwards may be the file that name lives in.
    misses: u64,
    /// The elements whose cached supertypes or semantic bases were
    /// worked out while a name was missing, and so are only as
    /// complete as the workspace was at the time.
    incomplete: HashSet<ElementId>,
    /// The element a reference is written in, for the whole of the
    /// walk that resolves it. Two root packages may answer to one name
    /// -- a model's own and the standard library's -- and which one is
    /// meant depends on where the name was written.
    origin: ElementId,
    imports: HashMap<ElementId, Option<ImportTarget>>,
    aliases: HashMap<ElementId, Option<ElementId>>,
    visibilities: HashMap<ElementId, Vis>,
    semantic_bases: HashMap<ElementId, Vec<ElementId>>,
    /// Members of a namespace by the names they answer to, in the
    /// order a lookup would have walked them. Searching the list itself
    /// costs a scan of every member of the namespace per lookup, which
    /// on a package of ten thousand parts is what makes a project take
    /// minutes to open rather than seconds.
    members: HashMap<ElementId, HashMap<String, Vec<ElementId>>>,
    /// What the pass under way has already reified, so that a second
    /// pass over the same declaration takes what the first one made
    /// instead of making it again, while a declaration that really
    /// does write the same relationship twice still gets two.
    claimed: HashSet<ElementId>,
    unresolved: Vec<Unresolved>,
    references: Vec<Reference>,
}

impl Clone for Workspace {
    /// A copy is a workspace to ask questions of, not a resolution
    /// caught halfway. The guards, the walk in progress and the
    /// failures held back until the current reference is answered all
    /// belong to that reference; carried into a copy they are a cycle
    /// guard against elements nothing is looking at and a trail of
    /// segments no name walked.
    fn clone(&self) -> Workspace {
        Workspace {
            model: self.model.clone(),
            root: self.root,
            files: self.files.clone(),
            source: self.source.clone(),
            elem_file: self.elem_file.clone(),
            supertypes: self.supertypes.clone(),
            in_progress: HashSet::new(),
            resolving: Vec::new(),
            blocked: 0,
            walking: 0,
            provisional: HashSet::new(),
            lookups: self.lookups,
            chain: Vec::new(),
            depth: 0,
            misses: self.misses,
            incomplete: self.incomplete.clone(),
            origin: self.root,
            imports: self.imports.clone(),
            aliases: self.aliases.clone(),
            visibilities: self.visibilities.clone(),
            semantic_bases: self.semantic_bases.clone(),
            members: self.members.clone(),
            claimed: HashSet::new(),
            unresolved: self.unresolved.clone(),
            references: self.references.clone(),
        }
    }
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
            walking: 0,
            provisional: HashSet::new(),
            lookups: 0,
            chain: Vec::new(),
            depth: 0,
            misses: 0,
            incomplete: HashSet::new(),
            origin: root,
            imports: HashMap::new(),
            aliases: HashMap::new(),
            visibilities: HashMap::new(),
            semantic_bases: HashMap::new(),
            members: HashMap::new(),
            claimed: HashSet::new(),
            unresolved: Vec::new(),
            references: Vec::new(),
        }
    }

    /// Parse `text` (dialect chosen from the file name's extension) and add
    /// it to the workspace. Returns the file index.
    ///
    /// The same file read twice is one file. A caller reaching one file
    /// two ways -- through a link, or a URL spelled two ways -- would
    /// otherwise declare everything in it twice, and the second copy
    /// would lose every lookup to the first, silently, in a model that
    /// still resolves.
    ///
    /// A name added again over *different* text is a different
    /// question, and still adds a second file rather than replacing the
    /// first: a workspace hands element ids out to its callers, and
    /// taking a file back would leave every id from it pointing at
    /// nothing. A front end that reopens a file -- the language server,
    /// when a buffer replaces what is on disk -- builds the workspace
    /// again instead.
    pub fn add_file(&mut self, name: impl Into<String>, text: &str) -> usize {
        let name = name.into();
        if let Some(same) = self
            .files
            .iter()
            .position(|file| file.name == name && file.parse.syntax().text() == text)
        {
            return same;
        }
        let parse = parse_dialect(text, Dialect::from_path(&name));
        let built = build_into(&mut self.model, &parse);
        let file_idx = self.files.len();
        for root in &built.roots {
            self.model.add_owned(self.root, *root);
        }
        let mut elements = Vec::with_capacity(built.source.len());
        for (id, node) in built.source {
            elements.push(id);
            self.source.insert(id, node);
            self.elem_file.insert(id, file_idx);
        }
        // the builder hands them back in no order at all
        elements.sort_unstable();
        self.files.push(File {
            name,
            parse,
            roots: built.roots,
            elements,
        });
        self.forget_failures();
        file_idx
    }

    /// Forget what was worked out from names that were not there.
    ///
    /// A workspace grows a file at a time: an editor opens a buffer
    /// over a project already loaded, a project loads its library
    /// after the file being edited. A lookup that failed before the
    /// file arrived is no evidence about the workspace it is asked
    /// about now, and remembering it is how a language server comes to
    /// underline a name the model does resolve. What was found stands
    /// -- a file only adds names, and the ones already found are still
    /// where they were.
    fn forget_failures(&mut self) {
        self.imports.retain(|_, target| target.is_some());
        self.aliases.retain(|_, target| target.is_some());
        for id in std::mem::take(&mut self.incomplete) {
            self.supertypes.remove(&id);
            self.semantic_bases.remove(&id);
        }
        // the new file's members are members of the root namespace, and
        // its own namespaces have none indexed yet
        self.members.clear();
    }

    /// Recursively load every `.sysml`/`.kerml` file under `dir`.
    pub fn load_dir(&mut self, dir: &std::path::Path) -> std::io::Result<usize> {
        let paths = model_files(dir);
        let count = paths.len();
        for path in paths {
            // the error names the file, not just the directory the
            // caller asked about: one unreadable file in a corpus of
            // hundreds is otherwise a refusal with nothing to act on
            let text = std::fs::read_to_string(&path).map_err(|err| {
                std::io::Error::new(err.kind(), format!("{}: {err}", path.display()))
            })?;
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
        let collisions = self
            .library_collisions()
            .into_iter()
            .filter(|c| wanted(c.file))
            .collect();
        Findings {
            syntax,
            names,
            collisions,
        }
    }

    /// Root packages declared under a name the standard library has
    /// already taken.
    ///
    /// Every file's outermost packages are members of one shared root
    /// namespace, so a `package Requirements` of one's own and the
    /// library's `Requirements` are two members of it under one name.
    /// Resolution keeps both halves working by reading each name on the
    /// side of the library boundary it was written on -- but the name
    /// then means one thing in the model and another in the library,
    /// and nothing in the file says so. Two packages of one's own
    /// sharing a name are not reported: the official examples do it
    /// deliberately, writing the same model twice over.
    fn library_collisions(&self) -> Vec<Finding> {
        let named: Vec<(ElementId, &str)> = self
            .model
            .owned(self.root)
            .iter()
            .filter_map(|&member| Some((member, self.model.name(member)?)))
            .collect();
        let of_library =
            |member: ElementId| self.model.kind(member).is_a(ElementKind::LibraryPackage);
        let taken: HashSet<&str> = named
            .iter()
            .filter(|&&(member, _)| of_library(member))
            .map(|&(_, name)| name)
            .collect();
        named
            .iter()
            .filter(|&&(member, name)| !of_library(member) && taken.contains(name))
            .filter_map(|&(member, name)| {
                Some(Finding {
                    file: *self.elem_file.get(&member)?,
                    range: self.element_ranges(member)?.1,
                    what: format!("`{name}` is also a root package of the standard library"),
                })
            })
            .collect()
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
        self.elements_of(file)
            .iter()
            .copied()
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
        self.elements_of(file)
            .iter()
            .copied()
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
        // Every other query answers `None` for a cursor the file does
        // not have; asking the tree for a token there is a panic. An
        // editor that trails a stale position behind an edit asks this
        // one as readily as the rest.
        if !syntax.text_range().contains_inclusive(offset) {
            return None;
        }
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
            self.collect_visible(scope, Access::Internal, &mut out, &mut seen);
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
    ) {
        // Every namespace is walked once, which both ends the walk and
        // keeps it as long as it needs to be: a chain of twenty
        // re-exporting packages is a chain lookup follows to the end,
        // and a completion list that stopped short of it offered fewer
        // names than the model resolves.
        if !seen.insert(ns) {
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
        }
        let sub_access = if access == Access::Internal {
            Access::Inherited
        } else {
            access
        };
        for sup in self.supertypes_of(ns) {
            self.collect_visible(sup, sub_access, out, seen);
        }
        if access != Access::Inherited {
            for import in self.imports_of(ns) {
                // only a `public import` re-exports, so only one of
                // those is reached from outside the namespace. What is
                // offered as you type has to be what a lookup would
                // find, or the completion list is a list of names the
                // model will not resolve.
                if access == Access::External && self.visibility(import) != Vis::Public {
                    continue;
                }
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
                        self.collect_visible(imp.target, target_access, out, seen);
                        // `import Q::**` reaches what is nested in Q as
                        // well, which is how `class Z :> F;` finds
                        // `Q::Q2::F`. Offering only Q's own members left
                        // the modeller typing blind a name the model
                        // resolves -- lookup has always followed it there.
                        if imp.scope == ImportScope::Recursive {
                            for desc in self.nested_visible(imp.target, target_access) {
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

    /// Every element built from `file`, which is where a query about a
    /// position in it has to look.
    fn elements_of(&self, file: usize) -> &[ElementId] {
        self.files.get(file).map_or(&[], |f| &f.elements)
    }

    /// File a model element was built from.
    pub fn element_file(&self, elem: ElementId) -> Option<usize> {
        self.elem_file.get(&elem).copied()
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Every element built from one file, in the order the builder made
    /// them.
    pub fn file_elements(&self, file: usize) -> &[ElementId] {
        &self.files[file].elements
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
                    self.resolve_operand_into(id, &operand, "memberElement", &mut stats);
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
                    match self.resolve_written(
                        id,
                        &t.segments,
                        may_name_itself(part_kind, is_definition),
                    ) {
                        Some(target) => {
                            stats.resolved += 1;
                            let file = self.elem_file.get(&id).copied().unwrap_or(0);
                            self.record(file, t.range, t.name_range, &t.at, target);
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
                            // `crosses sameThing.self` names a chain,
                            // not the feature at the end of it:
                            // `deriveFeatureCrossFeature` reads
                            // `crossedFeature.chainingFeature->at(2)`,
                            // and `validateCrossSubsettingCrossedFeature`
                            // holds the first step to being the other
                            // end of the association. Read as the last
                            // step alone, what answers for the chain is
                            // whatever chaining that feature happens to
                            // have of its own.
                            if part_kind == SyntaxKind::CROSSES_KW && t.chain.len() > 1 {
                                let chain: Vec<ElementId> = t
                                    .chain
                                    .iter()
                                    .filter_map(|&depth| {
                                        self.resolve_from(id, &t.segments[..depth])
                                    })
                                    .collect();
                                let crossed = self.reified(
                                    id,
                                    ElementKind::Feature,
                                    &[("chainingFeature", Value::RefList(chain))],
                                );
                                self.reify(id, is_definition, part_kind, crossed);
                                continue;
                            }
                            self.reify(id, is_definition, part_kind, target);
                        }
                        None => {
                            let file = self.elem_file.get(&id).copied().unwrap_or(0);
                            self.record_miss(file, t.range, &t.segments, &mut stats);
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
        self.carry_ends();
        self.imply_end_redefinitions();
        self.imply_cross_subsettings();
        stats.lookups = self.lookups - began;
        stats
    }

    /// The redefinition an end declared beside a supertype's implies.
    ///
    /// "If a Feature has isEnd = true and an owningType that is not
    /// empty, then, for each direct supertype of its owningType, it
    /// must redefine the endFeature at the same position, if any."
    /// Almost nothing writes it: `connect a to b` names no end at all,
    /// and the binary connection it specializes is reached implicitly.
    /// Read without it every such connector has four ends -- the two it
    /// was written with and the two it inherits -- and "a connector
    /// specializing a binary one is binary" is true of none of the
    /// eight hundred in the corpus.
    fn imply_end_redefinitions(&mut self) {
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
    fn implied_bases_of(&mut self, elem: ElementId, above: &[ElementId]) -> Vec<&'static str> {
        let mut implied = implied_bases(self.model.kind(elem));
        if !implied.iter().any(|path| BINARY.contains(path)) {
            return implied;
        }
        // A usage declares no ends of its own -- `interface i :
        // WheelHubInterface;` -- and relates as many things as what it
        // is typed by. Only what it says it specializes counts: the
        // base being chosen here is not one of them yet.
        let mut ends = self.own_ends(elem).len();
        for &up in above {
            ends = ends.max(self.ends_of(up).len());
        }
        if ends > 2 {
            implied.retain(|path| !BINARY.contains(path));
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
                match self.model.get(owned, "redefinedFeature") {
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
            .filter(|&it| self.model.get(it, "isEnd") == Some(&Value::Bool(true)))
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

    /// The subsetting an owned cross feature implies.
    ///
    /// "If this Feature is the ownedCrossFeature of an end Feature,
    /// then, for any end Feature that is redefined by the owning end
    /// Feature of this Feature, this Feature must subset the
    /// crossFeature of the redefined end Feature, if this exists."
    /// Nothing writes it down: the association declares the cross
    /// feature and the redefinition and leaves what holds between them
    /// to the tool, and `validateFeatureCrossFeatureSpecialization` is
    /// the specification asking for it back.
    fn imply_cross_subsettings(&mut self) {
        for elem in self.model.ids().collect::<Vec<_>>() {
            let Some(mine) = self.owned_cross_feature(elem) else {
                continue;
            };
            let redefined: Vec<ElementId> = self
                .model
                .owned(elem)
                .iter()
                .filter_map(|&it| match self.model.get(it, "redefinedFeature") {
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
    /// `ownedCrossFeature()` is "the first ownedMember of the Feature
    /// that is a Feature, but not a Multiplicity or a MetadataFeature,
    /// and whose owningMembership is not a FeatureMembership". The
    /// notation writes that two ways, and both are read here from what
    /// was written: `member feature inCart;` inside the end, and `end
    /// inCart[0..1] feature cart : ShoppingCart;` in front of it.
    fn owned_cross_feature(&self, elem: ElementId) -> Option<ElementId> {
        if self.model.get(elem, "isEnd") != Some(&Value::Bool(true)) {
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

    /// A feature that redefines an end is an end, and an end is not
    /// composite.
    ///
    /// `end` is written once: the corpus writes it on the outer feature
    /// and nests redefinitions of it without repeating the keyword, and
    /// the standard says as much --
    /// `validateRedefinitionEndConformance` holds a feature redefining
    /// an end to being one, and
    /// `validateFeatureEndNotDerivedAbstractCompositeOrPortion` holds
    /// an end to not being composite. Redefinitions are resolved by
    /// now, so this is where the two can be said.
    fn carry_ends(&mut self) {
        loop {
            let mut carried = false;
            for elem in self.model.ids().collect::<Vec<_>>() {
                if self.model.get(elem, "isEnd") == Some(&Value::Bool(true)) {
                    continue;
                }
                let redefines_an_end = self.model.owned(elem).iter().any(|&owned| {
                    self.model.kind(owned).is_a(ElementKind::Redefinition)
                        && matches!(
                            self.model.get(owned, "redefinedFeature"),
                            Some(Value::Ref(up))
                                if self.model.get(*up, "isEnd") == Some(&Value::Bool(true))
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
                self.record_miss(
                    self.elem_file.get(&usage).copied().unwrap_or(0),
                    target.range,
                    &target.segments,
                    stats,
                );
            }
        }
    }

    /// Resolve a qualified name starting from the scope that contains
    /// `elem`. `elem` itself is excluded from name matches: a feature's own
    /// (effective) name must not shadow the inherited feature it redefines.
    pub fn resolve_from(&mut self, elem: ElementId, segments: &[String]) -> Option<ElementId> {
        self.resolve_written(elem, segments, true)
    }

    /// As `resolve_from`, saying whether the declaration the name is
    /// written on may answer with itself.
    fn resolve_written(
        &mut self,
        elem: ElementId,
        segments: &[String],
        may_name_itself: bool,
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
            // A self-reference (`part p4 :> p4;`) is a legal name even
            // though a declaration cannot shadow the feature it
            // redefines, so the name is looked up once more with the
            // declaration itself allowed to answer.
            if !may_name_itself {
                return None;
            }
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

    /// Resolve what an alias names, put it on the membership, and say
    /// where it was written -- renaming the element has to reach the
    /// alias too.
    fn record_alias(&mut self, alias: ElementId, node: &SyntaxNode, stats: &mut ResolveStats) {
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
    fn record_miss(
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
        // A namespace inherits from a namespace that inherits from a
        // namespace: the walk goes as deep as the model specializes,
        // and a model can specialize deeper than a stack goes. Past
        // this the walk stops and the name is reported unresolved,
        // which is a finding a reader can act on rather than a crash.
        if self.walking >= MAX_INHERITANCE {
            return None;
        }
        self.walking += 1;
        let found = self.lookup_walk(
            ns,
            name,
            access,
            allow_inherited,
            allow_imports,
            exclude,
            guard,
        );
        self.walking -= 1;
        found
    }

    #[allow(clippy::too_many_arguments)]
    fn lookup_walk(
        &mut self,
        ns: ElementId,
        name: &str,
        access: Access,
        allow_inherited: bool,
        allow_imports: bool,
        exclude: Option<ElementId>,
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
            if Some(child) == exclude || !self.visible(child, access) {
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
                        if wrote == Some(name)
                            || (by_its_own_name && self.member_name_matches(imp.target, name))
                        {
                            return Some(imp.target);
                        }
                    }
                    ImportScope::Members => {
                        let inherited = self.inherits_into_imports(imp.target);
                        if let Some(hit) = self.lookup_guarded(
                            imp.target,
                            name,
                            target_access,
                            inherited,
                            true,
                            exclude,
                            guard,
                        ) {
                            return Some(hit);
                        }
                    }
                    ImportScope::Recursive => {
                        let inherited = self.inherits_into_imports(imp.target);
                        if let Some(hit) = self.lookup_guarded(
                            imp.target,
                            name,
                            target_access,
                            inherited,
                            true,
                            exclude,
                            guard,
                        ) {
                            return Some(hit);
                        }
                        for desc in self.nested_visible(imp.target, target_access) {
                            if Some(desc) != exclude && self.member_name_matches(desc, name) {
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
    fn in_library(&self, elem: ElementId) -> bool {
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
    fn nested_visible(&mut self, ns: ElementId, access: Access) -> Vec<ElementId> {
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

    /// Is this an end of the connector or association that owns it?
    ///
    /// What an end relates is reached through the types of the other
    /// ends, so an end is where a name lookup carries on from.
    fn is_end_member(&self, elem: ElementId) -> bool {
        self.model.get(elem, "isEnd") == Some(&Value::Bool(true))
    }

    fn member_name_matches(&self, elem: ElementId, name: &str) -> bool {
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

    /// What an element specializes, as the resolver worked it out.
    ///
    /// The text of a model does not say this: a definition's shape is
    /// what it inherits as much as what it declares, and only a resolved
    /// workspace knows which is which.
    pub fn supertypes(&mut self, elem: ElementId) -> Vec<ElementId> {
        self.supertypes_of(elem)
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
        let cut = self.misses;
        let refused = self.blocked;
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
        // `port def P` defines its conjugate as well, and `~P` has
        // what `P` has: conjugating a type reverses the direction of
        // its features, not which features it has. The conjugate is
        // reified and has no syntax to read, so what it is the
        // conjugate of is read off the conjugation it owns -- and
        // without it `apsc.subscr` names nothing where `apsc` is a
        // port typed `~SubscriptionPort`.
        let conjugated: Vec<ElementId> = self
            .model
            .owned(elem)
            .iter()
            .copied()
            .filter(|&owned| self.model.kind(owned).is_a(ElementKind::Conjugation))
            .filter_map(|owned| match self.model.get(owned, "originalType") {
                Some(Value::Ref(target)) => Some(*target),
                _ => None,
            })
            .collect();
        for target in conjugated {
            push_supertype(&mut supers, elem, target);
        }
        // A relationship the standard implies is written into the model
        // and nowhere else, so the source cannot answer for it -- an
        // owned cross feature subsets the cross feature of the end its
        // owner redefines, and nothing in the notation says so. See
        // `imply_cross_subsettings`.
        let implied: Vec<ElementId> = self
            .model
            .owned(elem)
            .iter()
            .copied()
            .filter(|&owned| {
                self.model.kind(owned).is_a(ElementKind::Subsetting)
                    && self.model.get(owned, "isImplied") == Some(&Value::Bool(true))
            })
            .filter_map(|owned| {
                self.model
                    .redefined_feature(owned)
                    .or_else(|| self.model.subsetted_feature(owned))
            })
            .collect();
        for target in implied {
            push_supertype(&mut supers, elem, target);
        }
        // An element the builder reified has no syntax of its own. An
        // accept node's payload is one: `accept cl : Cmd` declares it on
        // the statement, and the typing written there is attached to the
        // payload once it resolves. Read back from the model it is a
        // supertype like any other, and without it `cl.itms` names
        // nothing.
        if supers.is_empty() {
            let typed: Vec<ElementId> = self
                .model
                .owned(elem)
                .iter()
                .copied()
                .filter(|&owned| self.model.kind(owned) == ElementKind::FeatureTyping)
                .filter_map(|owned| match self.model.get(owned, "type") {
                    Some(Value::Ref(target)) => Some(*target),
                    _ => None,
                })
                .collect();
            for target in typed {
                push_supertype(&mut supers, elem, target);
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
        // An enumeration value is a variant of its enumeration, and it
        // *declares* the value rather than naming one written
        // elsewhere: `enum def E1 { a; b; c; }` has no `a` anywhere
        // else to bring along, and looking for one reaches past the
        // enumeration to whatever else the workspace calls `a`.
        let enumerated = self.model.owner(elem).is_some_and(|owner| {
            self.model
                .kind(owner)
                .is_a(ElementKind::EnumerationDefinition)
        });
        if supers.is_empty() && !enumerated && self.model.member_role(elem) == Some(Role::Variant) {
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
        for path in self.implied_bases_of(elem, &supers.clone()) {
            // From the root: these are the standard library's own
            // names, and a model is free to declare a package called
            // `Requirements` of its own -- `SimpleVehicleModel` does --
            // which would otherwise stand in front of the library's and
            // leave the usage specializing nothing.
            let segments: Vec<String> = std::iter::once(String::new())
                .chain(path.split("::").map(String::from))
                .collect();
            if let Some(target) = self.resolve_from(elem, &segments) {
                if target != elem
                    && !supers.contains(&target)
                    && !reaches(&self.model, target, elem)
                {
                    supers.push(target);
                }
            }
        }
        self.in_progress.remove(&elem);
        if self.misses != cut {
            self.incomplete.insert(elem);
        }
        // A name this walk asked for was refused by the guard that
        // stops an import from resolving itself, so a supertype may be
        // missing from the list for no reason but the order things
        // were asked in. Kept for the reference under way and worked
        // out again for the next one.
        if self.blocked != refused {
            self.provisional.insert(elem);
        }
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
        let cut = self.misses;
        let refused = self.blocked;
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
        if self.misses != cut {
            self.incomplete.insert(meta_def);
        }
        if self.blocked != refused {
            self.provisional.insert(meta_def);
        }
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
                    for below in self.nested_visible(imp.target, access) {
                        if self.model.kind(below).is_a(ElementKind::Namespace) {
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
            let mut bases = Vec::new();
            let above = self.supertypes_of(elem);
            for path in self.implied_bases_of(elem, &above) {
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
        written += self.imply_return_redefinitions();
        written
    }

    /// The redefinition a declared result parameter implies.
    ///
    /// `abstract function LiteralEvaluation specializes Evaluation {
    /// return : ScalarValue[1]; }` -- the library writes no `redefines`,
    /// and the standard says it does not have to: a result parameter of
    /// a function that specializes another redefines that one's. Without
    /// the redefinition the specializing function has two result
    /// parameters, its own and the one it inherits, and "a function has
    /// exactly one" is true of none of the five hundred in the corpus
    /// that declare one.
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

    /// Drop what the guard's refusals shaped, once the reference that
    /// prompted them has been answered. An element is an import, an
    /// alias or a type, so at most one of these has anything under it.
    fn forget_provisional(&mut self) {
        if self.depth == 0 && self.resolving.is_empty() {
            for id in std::mem::take(&mut self.provisional) {
                self.imports.remove(&id);
                self.aliases.remove(&id);
                self.supertypes.remove(&id);
                self.semantic_bases.remove(&id);
            }
        }
    }

    fn resolve_alias(&mut self, alias: ElementId) -> Option<ElementId> {
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

    /// Put an element of `kind` with `props` under `owner`, taking the
    /// one an earlier pass made rather than making a second.
    ///
    /// Resolving the same file again is a thing callers do -- asking
    /// what is wrong with a model resolves it -- and it does the same
    /// work over again: the same declarations, the same targets, the
    /// same relationships. Made afresh each time, a feature would come
    /// to hold its type twice and everything reading the model would
    /// see it twice. What this pass has already taken is passed over,
    /// so `connect a to a`, which really does write one end twice,
    /// still gets two.
    fn reified(
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
                    .all(|(prop, value)| self.model.get(child, prop) == Some(value))
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
            // relationship_parts only yields the five kinds above plus TYPING
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

    /// The two types a relationship written as its own statement relates.
    ///
    /// `feature g :> f;` reifies a `Subsetting` under `g` and gives it
    /// both ends from the declaration it hangs off. `subset g subsets
    /// f;` says the same thing with no declaration to hang off: the
    /// `Subsetting` is what was written, and it reaches the model with
    /// neither end until the name before the clause and the name inside
    /// it are read here.
    /// Resolve one operand a statement wrote and keep what it landed on
    /// under `property`, or record that it landed on nothing.
    ///
    /// `first x;` and `specialization s subtype A :> B;` both write a
    /// name where the abstract syntax keeps a reference, and what has
    /// to happen either way is the same: resolve it, record it so a
    /// rename can find it, and set the property.
    fn resolve_operand_into(
        &mut self,
        id: ElementId,
        operand: &SyntaxNode,
        property: &str,
        stats: &mut ResolveStats,
    ) {
        let file = self.elem_file.get(&id).copied().unwrap_or(0);
        let segments = operand_segments(operand);
        let range = operand.text_range();
        match self.resolve_written(id, &segments, false) {
            Some(target) => {
                stats.resolved += 1;
                let name_range = last_name_range(operand);
                self.record(file, range, name_range, &operand_ranges(operand), target);
                self.model.set(id, property, Value::Ref(target));
            }
            None => self.record_miss(file, range, &segments, stats),
        }
    }

    fn resolve_relation_ends(
        &mut self,
        id: ElementId,
        node: &SyntaxNode,
        stats: &mut ResolveStats,
    ) {
        let Some((keyword, source_prop, target_prop)) = relation_ends(self.model.kind(id)) else {
            return;
        };
        let file = self.elem_file.get(&id).copied().unwrap_or(0);
        if let Some(operand) = operand_after(node, keyword) {
            self.resolve_operand_into(id, &operand, source_prop, stats);
        }
        for (_, targets) in relationship_parts(node) {
            for t in targets {
                match self.resolve_written(id, &t.segments, false) {
                    Some(target) => {
                        stats.resolved += 1;
                        self.record(file, t.range, t.name_range, &t.at, target);
                        self.model.set(id, target_prop, Value::Ref(target));
                    }
                    None => self.record_miss(file, t.range, &t.segments, stats),
                }
            }
        }
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
                    self.reify_end(id, &segments);
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
                self.end_reaching(id, vec![target]);
                related.push(target);
            }
        }
        // A succession relates two things and the notation lets one of
        // them go unwritten. `then b;` says where the flow goes and not
        // where it comes from; `first start;` says the other. Which one
        // is missing is what the statement wrote, and the answer is its
        // neighbour in the same body. Without it the model says a step
        // follows nothing, and `validateConnectorRelatedFeatures` -- "a
        // concrete Connector must have at least two relatedFeatures" --
        // is the specification saying so.
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
                Some(_) => (&[SyntaxKind::FIRST_KW][..], &[SyntaxKind::THEN_KW][..]),
                None => (
                    &[SyntaxKind::FIRST_KW, SyntaxKind::FROM_KW][..],
                    &[SyntaxKind::THEN_KW, SyntaxKind::TO_KW][..],
                ),
            };
            // Only the source can be the missing one. `first a;` says
            // which step comes first and writes no flow at all --
            // `InitialNodeMember` rather than a succession -- so a
            // succession that names one end names the one it runs to.
            if written(ends.1) && !written(ends.0) {
                if let Some(source) = self.step_before(id) {
                    related.insert(0, source);
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
            }
        }
        if !related.is_empty() {
            // A transition relates its source and target through the
            // `Succession` it owns rather than by being a connector
            // itself, so that is where the two ends belong.
            let holder = self
                .model
                .owned(id)
                .iter()
                .copied()
                .find(|&child| self.model.kind(child).is_a(ElementKind::SuccessionAsUsage))
                .filter(|_| self.model.kind(id).is_a(ElementKind::TransitionUsage))
                .unwrap_or(id);
            self.try_set(holder, "relatedFeature", Value::RefList(related));
        }
        // How many things it relates says which library type it
        // specializes, and the ends were not there to be counted when
        // anything asked earlier.
        self.supertypes.remove(&id);
    }

    /// What a succession runs from, where the statement left it
    /// unwritten: the nearest member of the same body before it that a
    /// succession can join.
    ///
    /// A step or an occurrence, since a sequence model writes `event
    /// occurrence e; then f;`. Where the nearest one is another
    /// succession the answer is the end of it facing this one: `then a;
    /// then b;` runs a to b, not the first succession to b.
    fn step_before(&self, succession: ElementId) -> Option<ElementId> {
        let owner = self.model.owner(succession)?;
        let members = self.model.owned(owner);
        let at = members.iter().position(|&it| it == succession)?;
        for &member in members[..at].iter().rev() {
            let kind = self.model.kind(member);
            if kind.is_a(ElementKind::ConnectorAsUsage) {
                // the one before this went somewhere, and that is where
                // this one starts
                if let Some(Value::RefList(related)) = self.model.get(member, "relatedFeature") {
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
    fn resolve_annotation(&mut self, id: ElementId, node: &SyntaxNode, stats: &mut ResolveStats) {
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
    fn resolve_prefix_metadata(
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
    /// `send new S() via displayPort to screen;` says which port the
    /// message leaves by and who receives it, and `assign v := 1;` which
    /// feature it sets. None of the three was being looked up at all, so
    /// the names stood for nothing -- and a name that stands for nothing
    /// was not reported either, which is worse than reporting it.
    ///
    /// The standard keeps the first two as arguments of the action, in
    /// the input parameters the builder laid out in the order the
    /// specification declares them, and the last as a membership the
    /// assignment does not own: `deriveAssignmentActionUsageReferent`
    /// reads back the first member of one, and
    /// `validateAssignmentActionUsageReferent` says there must be one.
    fn resolve_action_arguments(
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
    /// standard reads the referent back off the membership that says so
    /// -- `deriveFeatureReferenceExpressionReferent` takes the first
    /// owned membership that is not a parameter's. Holding the answer
    /// and not the membership leaves the expression referring to
    /// something by a route the specification does not have.
    fn refers_to(&mut self, reference: ElementId, target: ElementId) {
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
        }
    }

    /// What the statement a succession was built from declares.
    ///
    /// `then merge continue;` writes the node and the succession into it
    /// as one statement, so the two elements share the one syntax node
    /// and the declaration is the sibling that node also became. A
    /// `then message m of T;` writes a flow that way, which is a
    /// connector itself -- so what is looked for is what the statement
    /// declared, never the succession beside it.
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
    fn reify_end(&mut self, connector: ElementId, segments: &[String]) {
        let mut chain = Vec::new();
        // the full path already resolved, so every prefix normally does too
        for depth in 1..=segments.len() {
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
        // `EndFeatureMembership` is how the standard owns one, and
        // saying so is also what tells such a feature from a member the
        // source wrote as a reference.
        let end = self.reified(connector, ElementKind::Feature, &[]);
        self.try_set(end, "isEnd", Value::Bool(true));
        self.counts_one(end);
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

    /// An end is one thing.
    ///
    /// `validateFeatureEndMultiplicity` -- "if a Feature has isEnd =
    /// true, then it must have multiplicity 1..1" -- and the notation
    /// writes it nowhere. What stands before an end in `first [0..1]
    /// decide then [0..1] merge` is the cross multiplicity, how many
    /// things at the far end go with one at this one; the end itself is
    /// a participant, and there is one of it. Without the range, the
    /// four constraints that count what a control node is joined by
    /// have nothing to read.
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
    fn resolve_trigger_type(
        &mut self,
        transition: ElementId,
        node: &SyntaxNode,
        stats: &mut ResolveStats,
    ) {
        let declared = match self.model.get(transition, "triggerAction") {
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
                self.model
                    .owned(transition)
                    .iter()
                    .copied()
                    .find(|&child| self.model.kind(child) == ElementKind::AcceptActionUsage)
            }
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
                match self.resolve_written(transition, &t.segments, false) {
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
                    self.record_miss(file, range, &segments, stats);
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

/// Everything an element of this metaclass implicitly specializes: the
/// semantic-library types the standard maps the metaclass to, and --
/// for a feature -- the top-level `Base::things` every one of them
/// subsets.
fn implied_bases(kind: ElementKind) -> Vec<&'static str> {
    let mut implied: Vec<&str> = implicit_supertype(kind).to_vec();
    if kind.is_a(ElementKind::Feature) && !implied.contains(&"Base::things") {
        implied.push("Base::things");
    }
    implied
}

/// The library types that relate exactly two things, and the ones an
/// element reaches instead where it relates more.
///
/// `validateConnectorBinarySpecialization` -- "if a Connector has more
/// than two connectorEnds, then it must not specialize, directly or
/// indirectly, the Association BinaryLink" -- and
/// `validateAssociationBinarySpecialization` says the same of an
/// association. Each of these is listed in front of what it narrows, so
/// dropping it leaves the one an n-ary relationship reaches.
const BINARY: [&str; 4] = [
    "Links::BinaryLink",
    "Objects::BinaryLinkObject",
    "Connections::BinaryConnection",
    "Interfaces::BinaryInterface",
];

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
        InterfaceDefinition | InterfaceUsage => {
            &["Interfaces::BinaryInterface", "Interfaces::Interface"]
        }
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
        // the library's own words: "ViewpointCheck ... is the base type
        // of all ViewpointDefinitions". There is no `Views::Viewpoint`,
        // so a viewpoint was specializing nothing at all.
        ViewpointDefinition | ViewpointUsage => &["Views::ViewpointCheck"],
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
        // Each kind of literal has an evaluation of its own in the
        // library, and each of those declares the `return` that is the
        // literal's result. Without them a literal specializes nothing
        // and has no result at all, which is what nine tenths of the
        // constraints about a result parameter were tripping over.
        LiteralBoolean => &["Performances::literalBooleanEvaluations"],
        LiteralInteger => &["Performances::literalIntegerEvaluations"],
        LiteralRational => &["Performances::literalRationalEvaluations"],
        LiteralString => &["Performances::literalStringEvaluations"],
        // the library states no evaluation for an infinite literal, so
        // it is a literal evaluation and nothing narrower
        LiteralInfinity => &["Performances::literalEvaluations"],
        FeatureReferenceExpression => &["Performances::evaluations"],
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
    /// the segment depths a chained step ends at, in order
    chain: Vec<usize>,
}

/// Whether the name after `connector` is the end it runs from.
///
/// KerML writes a connector's declaration only in front of a `from`, so
/// `connector eng to tanks.main1;` names no connector: it relates `eng`
/// to `tanks.main1`. The n-ary form writes its ends in parentheses and
/// may be named without one, so the `to` is what tells them apart.
fn names_an_end(node: &SyntaxNode) -> bool {
    let has = |wanted| {
        node.children_with_tokens()
            .filter_map(|it| it.into_token())
            .any(|it| it.kind() == wanted)
    };
    has(SyntaxKind::CONNECTOR_KW) && has(SyntaxKind::TO_KW) && !has(SyntaxKind::FROM_KW)
        || has(SyntaxKind::BINDING_KW)
            && !has(SyntaxKind::BIND_KW)
            && !has(SyntaxKind::OF_KW)
            && node.children().any(|it| it.kind() == SyntaxKind::VALUE)
}

/// Whether the `=` in a statement writes a connector end rather than a
/// value.
fn binds_an_end(node: &SyntaxNode) -> bool {
    node.children_with_tokens()
        .filter_map(|it| it.into_token())
        .any(|it| matches!(it.kind(), SyntaxKind::BINDING_KW | SyntaxKind::BIND_KW))
}

/// The two ends a `binding` binds.
///
/// SysML writes `binding [1] bind [0..*] base.edges = [0..*] be;` and
/// KerML `binding ab of a = b;` or `binding a = b;`, and in every one of
/// them a declaration stands only in front of the keyword that
/// introduces the first end. So without a `bind` or an `of` the
/// reference after `binding` is that end. The `=` takes the other,
/// which the parser keeps inside the value clause where nothing follows
/// it and beside the clause where a multiplicity does.
fn binding_operands(node: &SyntaxNode) -> Vec<SyntaxNode> {
    let is_end = |kind| {
        matches!(
            kind,
            SyntaxKind::NAME_REF | SyntaxKind::PATH_EXPR | SyntaxKind::NAME | SyntaxKind::TYPE_REF
        )
    };
    let mut out = Vec::new();
    let mut taking = names_an_end(node);
    for element in node.children_with_tokens() {
        match element {
            sysml_syntax::SyntaxElement::Token(token) => {
                if matches!(token.kind(), SyntaxKind::BIND_KW | SyntaxKind::OF_KW) {
                    taking = true;
                }
            }
            sysml_syntax::SyntaxElement::Node(child) => match child.kind() {
                // `bind [0..*] base.edges` counts the end before naming
                // it, and the count is not what the keyword introduced
                SyntaxKind::MULTIPLICITY => {}
                SyntaxKind::VALUE => {
                    out.extend(child.children().filter(|it| is_end(it.kind())));
                    taking = true;
                }
                kind if taking && is_end(kind) => {
                    out.push(child);
                    taking = false;
                }
                _ => {}
            },
        }
    }
    out
}

/// Whether a declaration was written as a member of its owner rather
/// than as a feature of it -- `member feature inCart;`, or the cross
/// feature standing between an `end` and the declaration after it.
fn written_as_member(node: &SyntaxNode) -> bool {
    let tokens = || {
        node.children_with_tokens()
            .filter_map(|it| it.into_token())
            .map(|it| it.kind())
    };
    tokens().any(|kind| kind == SyntaxKind::MEMBER_KW)
        || (tokens().any(|kind| kind == SyntaxKind::END_KW)
            && node
                .children()
                .any(|it| matches!(it.kind(), SyntaxKind::DEFINITION | SyntaxKind::USAGE)))
}

/// The segment depths at which a chained step ends.
///
/// `cart::product_account.inCart` names two features and not three:
/// `::` qualifies one name, and `.` steps from one feature to the next.
/// A name with no dot in it is one step, which is no chain at all --
/// the standard gives a feature either no chaining features or more
/// than one.
fn chain_steps(qname: &SyntaxNode) -> Vec<usize> {
    let mut steps = Vec::new();
    let mut at = 0;
    for token in qname.children_with_tokens().filter_map(|e| e.into_token()) {
        match token.kind() {
            SyntaxKind::IDENT
            | SyntaxKind::UNRESTRICTED_NAME
            | SyntaxKind::DOLLAR
            | SyntaxKind::STAR
            | SyntaxKind::STAR_STAR => at += 1,
            SyntaxKind::DOT => steps.push(at),
            _ => {}
        }
    }
    steps.push(at);
    steps
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
    Some(Target {
        segments: name_segments(&qname),
        range: qname.text_range(),
        name_range: last_name_range(&qname),
        at: segment_ranges(&qname),
        chain: chain_steps(&qname),
    })
}

fn relationship_parts(node: &SyntaxNode) -> Vec<(SyntaxKind, Vec<Target>)> {
    // `connector a ::> a.x to b;` writes no `from`, so `a ::> a.x` is
    // the end it runs from -- `ConnectorEnd : Feature = ...
    // ( declaredName = NAME REFERENCES )? OwnedReferenceSubsetting` --
    // and what the end refers to is not something the connector itself
    // refers to.
    let end_refers = names_an_end(node);
    node.children()
        .filter_map(|part| match part.kind() {
            SyntaxKind::REFERENCES if end_refers => None,
            SyntaxKind::TYPING
            | SyntaxKind::SUBSETTING
            | SyntaxKind::REDEFINITION
            | SyntaxKind::REFERENCES => {
                // `end cart : ShoppingCart crosses selectedProduct.inCart`
                // is written in the same shape as `subsets`, and the
                // standard makes a relationship of its own of it: a
                // cross subsetting says which feature of the other end
                // this one is reached across.
                let crosses = part
                    .children_with_tokens()
                    .filter_map(|it| it.into_token())
                    .map(|it| it.kind())
                    .find(|it| !it.is_trivia())
                    == Some(SyntaxKind::CROSSES_KW);
                match crosses {
                    true => Some((SyntaxKind::CROSSES_KW, part)),
                    false => Some((part.kind(), part)),
                }
            }
            // KerML writes `unions T`, `chains a.b`, `disjoint from T`
            // and their kin as the one shape, told apart by the keyword
            // leading it. Five of them relate a type or a feature to
            // another; the rest say something else and are read, where
            // they are read at all, elsewhere.
            SyntaxKind::RELATION => {
                let lead = part
                    .children_with_tokens()
                    .filter_map(|it| it.into_token())
                    .map(|it| it.kind())
                    .find(|it| !it.is_trivia())?;
                matches!(
                    lead,
                    SyntaxKind::UNIONS_KW
                        | SyntaxKind::INTERSECTS_KW
                        | SyntaxKind::DIFFERENCES_KW
                        | SyntaxKind::CHAINS_KW
                        | SyntaxKind::CONJUGATES_KW
                )
                .then_some((lead, part))
            }
            _ => None,
        })
        .map(|(kind, part)| {
            let targets = part
                .children()
                .filter(|c| c.kind() == SyntaxKind::TYPE_REF)
                .filter_map(|type_ref| {
                    let qname = type_ref
                        .children()
                        .find(|c| c.kind() == SyntaxKind::QUALIFIED_NAME)?;
                    let mut segments = name_segments(&qname);
                    // `port p : ~P` types the port by the conjugate of
                    // `P`, which the port definition owns under that
                    // name. Naming it is a step further down the same
                    // path, so the walk that finds `P` finds it.
                    let conjugated = type_ref
                        .children_with_tokens()
                        .filter_map(|it| it.into_token())
                        .any(|it| it.kind() == SyntaxKind::TILDE);
                    if conjugated {
                        let last = segments.last()?.clone();
                        segments.push(format!("~{last}"));
                    }
                    Some(Target {
                        chain: chain_steps(&qname),
                        segments,
                        range: match conjugated {
                            true => type_ref.text_range(),
                            false => qname.text_range(),
                        },
                        name_range: last_name_range(&qname),
                        at: segment_ranges(&qname),
                    })
                })
                .collect();
            (kind, targets)
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
        .filter(|t| {
            matches!(
                t.kind(),
                SyntaxKind::IDENT | SyntaxKind::UNRESTRICTED_NAME | SyntaxKind::DOLLAR
            )
        })
        // a name written from the root means the same thing in an
        // expression as anywhere else, and the root is spelled as the
        // segment a declared name cannot be
        .map(|t| match t.kind() {
            SyntaxKind::DOLLAR => String::new(),
            _ => sysml_syntax::unquote(t.text()),
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

/// Whether a declaration writing its own name may be answered with
/// itself.
///
/// `part p4 :> p4;` says the feature is the one its type already
/// declares, and the language reads it that way even where the type
/// declares no such thing. Nothing else can mean that: `part v : v;`
/// would make a feature its own type and `part def C :> C;` a
/// definition its own supertype -- loops that say nothing, and that
/// every reader of the model would have to know to stop at.
fn may_name_itself(part: SyntaxKind, is_definition: bool) -> bool {
    match part {
        // `end cart : ShoppingCart crosses cart::product_account.inCart`
        // -- what an end crosses to is reached through the ends of the
        // association, this one included, so the path may start with
        // the very name being declared
        SyntaxKind::SUBSETTING | SyntaxKind::CROSSES_KW => !is_definition,
        SyntaxKind::REDEFINITION | SyntaxKind::REFERENCES => true,
        _ => false,
    }
}

/// What a relationship written as a statement of its own says, for the
/// kinds that write both ends as plain names: the keyword the first of
/// them follows, and the properties the standard keeps the two on.
///
/// `disjoining d disjoint A from B;`, `conjugation c conjugate A ~ B;`
/// and their kin write their ends in shapes of their own and are not
/// read here.
fn relation_ends(kind: ElementKind) -> Option<(SyntaxKind, &'static str, &'static str)> {
    let ends = match kind {
        ElementKind::Specialization => (SyntaxKind::SUBTYPE_KW, "specific", "general"),
        ElementKind::Subclassification => (
            SyntaxKind::SUBCLASSIFIER_KW,
            "subclassifier",
            "superclassifier",
        ),
        ElementKind::Subsetting => (
            SyntaxKind::SUBSET_KW,
            "subsettingFeature",
            "subsettedFeature",
        ),
        ElementKind::Redefinition => (
            SyntaxKind::REDEFINITION_KW,
            "redefiningFeature",
            "redefinedFeature",
        ),
        ElementKind::FeatureTyping => (SyntaxKind::TYPING_KW, "typedFeature", "type"),
        _ => return None,
    };
    Some(ends)
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

/// Whether a statement writes a keyword of its own, rather than one
/// nested in something it declares.
fn has_leading(node: &SyntaxNode, keyword: SyntaxKind) -> bool {
    node.children_with_tokens()
        .filter_map(|part| part.into_token())
        .any(|token| token.kind() == keyword)
}

/// The operands naming a connector's or transition's ends.
///
/// A connector relates every reference it holds. A transition writes an
/// optional name of its own first (`transition off_to_on first off then
/// on`), so only the references introduced by `first`/`then` are ends.
///
/// One statement can be two elements, and then the answer depends on
/// which of them is asking -- so it is asked of the element's metaclass
/// rather than of the syntax alone.
fn end_operands(node: &SyntaxNode, of: ElementKind) -> Vec<SyntaxNode> {
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
        // `then message m of T from a to b;` is the flow and the
        // succession into it, and each has ends of its own: the flow
        // runs from `a` to `b`, and the step runs into the flow from
        // whatever was written above it. Among the elements a control
        // statement builds, a flow is the only connector that is not
        // itself a succession.
        SyntaxKind::CONTROL_STMT
            if of.is_a(ElementKind::ConnectorAsUsage)
                && !of.is_a(ElementKind::SuccessionAsUsage) =>
        {
            &[SyntaxKind::FROM_KW, SyntaxKind::TO_KW][..]
        }
        // `transition t first a ... then b` writes a name of its own first
        SyntaxKind::CONTROL_STMT => &[SyntaxKind::FIRST_KW, SyntaxKind::THEN_KW][..],
        // a binding writes its two ends around an `=` rather than after
        // a keyword each
        _ if node
            .children_with_tokens()
            .filter_map(|it| it.into_token())
            .any(|it| it.kind() == SyntaxKind::BINDING_KW) =>
        {
            return binding_operands(node)
        }
        // `connection c : L connect a to b;` and `flow f of T from a to b;`
        // declare a name and a type before the ends arrive, and
        // `succession a then b;` writes its ends around the keyword.
        // `allocation a : L allocate x to y;` introduces its first end
        // the same way `connect` does.
        _ => &[
            SyntaxKind::CONNECT_KW,
            SyntaxKind::ALLOCATE_KW,
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
    // `connector eng to tanks.main1;` and `connector a ::> a.x to b;`
    // write no `from`, and `BinaryConnectorDeclaration : Connector = (
    // FeatureDeclaration? 'from' | isSufficient ?= 'all' 'from'? )?
    // ConnectorEndMember 'to' ConnectorEndMember` allows a declaration
    // only in front of one. So what stands between the keyword and the
    // `to` is the end the connector runs from, named or not, and the
    // connector has no name of its own. What the end refers to wins
    // over the name it was given, which is why the last one before the
    // `to` is the answer.
    let front_of_a_connector = names_an_end(node)
        .then(|| {
            let mut found = None;
            for element in node.children_with_tokens() {
                if element
                    .as_token()
                    .is_some_and(|token| token.kind() == SyntaxKind::TO_KW)
                {
                    break;
                }
                if let Some(child) = element.into_node() {
                    if is_reference(child.kind())
                        || matches!(child.kind(), SyntaxKind::NAME | SyntaxKind::REFERENCES)
                    {
                        found = Some(child);
                    }
                }
            }
            found
        })
        .flatten();
    let mut front = front_of_a_connector.or_else(|| {
        node.children_with_tokens()
            .take_while(|element| {
                element.as_token().is_none_or(|token| {
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
            .find(|child| is_reference(child.kind()))
    });
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
    // `transition first a accept s do action D then b;` -- `do` takes
    // the rest of the statement with it, so the target parses inside the
    // action the effect declares. It is the transition's target either
    // way, and read only from the statement's own children the
    // transition relates one thing.
    if out.len() < 2 && of.is_a(ElementKind::TransitionUsage) {
        let carried = node
            .children()
            .filter(|child| child.kind() == SyntaxKind::USAGE)
            .find_map(|child| operand_after(&child, SyntaxKind::THEN_KW));
        out.extend(carried);
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
        // one range per segment, the root marker included: what each
        // step of the name landed on is paired off against these
        .filter(|t| {
            matches!(
                t.kind(),
                SyntaxKind::IDENT | SyntaxKind::UNRESTRICTED_NAME | SyntaxKind::DOLLAR
            )
        })
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
            sysml_syntax::unquote(t.text())
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
        // Only a directory that is one is entered: a link to a directory
        // is where a walk goes round in circles, loading every file once
        // per level the kernel allows, or never coming back at all.
        let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
        if is_dir {
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

    /// `subset g subsets f;` relates the same two features as `feature
    /// g :> f;`, with the relationship written as the statement instead
    /// of reified under a declaration. Read only where a declaration
    /// carries it, the statement form reached the model relating nothing
    /// to nothing.
    ///
    /// `disjoining d disjoint A from B;` writes its two types in a shape
    /// of its own and is passed over here; the last two statements miss
    /// on either side, which leaves that end unsaid rather than guessed.
    #[test]
    fn a_relationship_written_as_a_statement_says_what_it_relates() {
        let (ws, stats) = resolved_workspace(&[(
            "r.kerml",
            "package K {\n\
             \tclassifier A;\n\
             \tclassifier B;\n\
             \tfeature f : A;\n\
             \tfeature g : A;\n\
             \tspecialization s subtype A :> B;\n\
             \tsubclassifier B :> A;\n\
             \tsubset g subsets f;\n\
             \tredefinition g redefines f;\n\
             \ttyping g : A;\n\
             \tdisjoining d disjoint A from B;\n\
             \tsubset g subsets nowhere;\n\
             \tsubset nowhere subsets f;\n\
             }\n",
        )]);
        assert_eq!(stats.unresolved, 2, "unresolved: {:?}", ws.unresolved());
        let model = ws.model();
        let related: Vec<(ElementKind, Option<&str>, Option<&str>)> = model
            .owned(ws.file_roots(0)[0])
            .iter()
            .filter_map(|&id| {
                let (_, source, target) = relation_ends(model.kind(id))?;
                let end = |prop| {
                    model
                        .get(id, prop)
                        .and_then(Value::as_id)
                        .and_then(|at| model.name(at))
                };
                Some((model.kind(id), end(source), end(target)))
            })
            .collect();
        assert_eq!(
            related,
            [
                (ElementKind::Specialization, Some("A"), Some("B")),
                (ElementKind::Subclassification, Some("B"), Some("A")),
                (ElementKind::Subsetting, Some("g"), Some("f")),
                (ElementKind::Redefinition, Some("g"), Some("f")),
                (ElementKind::FeatureTyping, Some("g"), Some("A")),
                (ElementKind::Subsetting, Some("g"), None),
                (ElementKind::Subsetting, None, Some("f")),
            ]
        );
    }

    /// `class B conjugates A;` writes the conjugation as a clause, and
    /// the type that is conjugated owns it. `conjugation c conjugate B
    /// conjugates A;` writes the same relationship as a statement, which
    /// the namespace owns -- and by the standard's own account that
    /// leaves B unconjugated, since a conjugated type is one with a
    /// conjugator of its own.
    #[test]
    fn a_conjugates_clause_belongs_to_the_type_it_conjugates() {
        let (ws, stats) = resolved_workspace(&[(
            "c.kerml",
            "package K {\n\
             \tclass A;\n\
             \tclass B conjugates A;\n\
             \tconjugation c conjugate A conjugates B;\n\
             }\n",
        )]);
        assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
        let model = ws.model();
        let conjugations: Vec<(Option<&str>, Option<&str>, Option<&str>)> = model
            .ids()
            .filter(|&id| model.kind(id) == ElementKind::Conjugation)
            .map(|id| {
                let end = |prop| {
                    model
                        .get(id, prop)
                        .and_then(Value::as_id)
                        .and_then(|at| model.name(at))
                };
                let owner = model.owner(id).and_then(|up| model.name(up));
                (owner, end("conjugatedType"), end("originalType"))
            })
            .collect();
        assert_eq!(
            conjugations,
            [
                // written as a statement: the namespace's own, and the
                // shape it writes its two types in is not read here
                (Some("K"), None, None),
                // written as a clause: B's own, relating B to A
                (Some("B"), Some("B"), Some("A")),
            ]
        );
    }

    #[test]
    fn resolves_aliases_and_qualified_paths() {
        let (ws, stats) = resolved_workspace(&[(
            "m.sysml",
            "package P {\n  part def Engine;\n  alias Motor for Engine;\n}\npackage Q {\n  part e : P::Motor;\n  part f : $::P::Engine;\n}",
        )]);
        assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
        // two typings, and the alias's own `for Engine`
        assert_eq!(stats.resolved, 3);
        let alias = ws
            .model()
            .ids()
            .find(|&id| ws.is_alias(id))
            .expect("the alias is an element of the model");
        let engine = ws
            .alias_target(alias)
            .expect("the alias says what it names");
        assert_eq!(ws.qualified_name_of(engine), "P::Engine");
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

    /// A keyword whose base type names something that is not there
    /// leaves the elements it marks without one, and says so once --
    /// the answer is only as good as the workspace it was worked out
    /// in, so it must not outlive the file that was missing.
    #[test]
    fn a_base_type_that_names_nothing_is_reported_once() {
        let (ws, _) = resolved_workspace(&[(
            "m.sysml",
            "package Lib {\n  metadata def SemanticMetadata { attribute baseType; }\n  metadata def cause :> SemanticMetadata {\n    :>> baseType = nowhere;\n  }\n}\npackage M {\n  import Lib::*;\n  #cause 'battery old';\n}",
        )]);
        let names: Vec<&str> = ws.unresolved().iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, ["nowhere"]);
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

    /// The root namespace holds both halves and is on neither side.
    #[test]
    fn the_root_namespace_is_in_no_library() {
        let ws = Workspace::new();
        assert!(!ws.in_library(ws.root()));
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

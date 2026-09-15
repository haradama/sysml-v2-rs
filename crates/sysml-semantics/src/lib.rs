//! Name resolution and relationship reification.
//!
//! A [`Workspace`] holds any number of parsed files built into one shared
//! [`Model`], owned by a synthetic root namespace (the KerML global
//! namespace, addressable as `$`). [`Workspace::resolve_all`] resolves
//! every explicit typing (`: T`), specialization (`:>`), redefinition
//! (`:>>`) and reference subsetting (`::>`) target, and reifies the
//! relationship elements for them into the model with resolved references
//! as properties.
//!
//! It resolves the names written inside expressions too -- a `require
//! constraint { ... }` body, the result of a `calc`, the value after `=`.
//! The model keeps an expression as the text its author wrote rather than
//! as a tree, so those names are read off the syntax and looked up from
//! the element the expression belongs to, which is the scope the language
//! gives them.
//!
//! Lookup handles member and short names, ownership-scope walking,
//! visibility (members default public, imports default private; only
//! `public import` re-exports; `import all` overrides), imports (`A::B`,
//! `A::*`, `A::**`, re-exports through chains), aliases, inherited members
//! through resolved specializations and typings (which is what makes
//! `engine.mass` work), implicit semantic-library specializations (`part
//! def` to `Parts::Part`), user-defined keywords via SemanticMetadata,
//! connector-end scoping, implicit `result` parameters, effective names of
//! unnamed redefining features, and `$`-rooted qualified names.
//!
//! With the official standard library loaded, every reference in the
//! library and in all official example models resolves.
//!
//! # Where things are
//!
//! This file holds the [`Workspace`] itself -- what it is made of, what it
//! caches, and the questions an editor asks it. The work of answering a
//! written name is next door:
//!
//! | Module | What it does |
//! | --- | --- |
//! | `resolve` | What gets resolved, in what order, and what is cleared first |
//! | `lookup` | Working a written name out to the element it names |
//! | `inherit` | What a type reaches through what it specializes |
//! | `imports` | What an `import` brings in, and what an `alias` stands for |
//! | `reify` | Turning a resolved name into the relationship the model keeps |
//! | `implied` | The relationships the notation leaves to be inferred |
//! | `expressions` | The names written inside an expression |
//! | `syntax` | What the notation writes, read off the syntax tree |
//! | `rules` | The constraints the specification states, evaluated |
//! | `ocl` | Reading those constraints, which the metamodel states in OCL |
//!
//! Every one is `impl Workspace` over the fields declared here, so the
//! split is for a reader and costs nothing at run time.

mod expressions;
mod implied;
mod imports;
mod inherit;
mod lookup;
mod ocl;
mod reify;
mod resolve;
/// The well-formedness constraints the specification states in OCL,
/// read from the metamodel and evaluated over a model.
pub mod rules;
mod syntax;

use syntax::*;

use std::collections::{HashMap, HashSet};

use sysml_model::{build_into, ElementId, ElementKind, Model, Value, Vis};
use sysml_syntax::{parse_dialect, Dialect, Parse, SyntaxKind, SyntaxNode, TextRange};

/// How many namespaces deep a lookup will walk through inherited
/// members before it gives up. The corpus, standard library included,
/// never goes past a few dozen; a model that goes thousands deep would
/// take the stack down with it, so it is told its name resolves to
/// nothing instead. The parser bounds its own nesting the same way.
pub(crate) const MAX_INHERITANCE: usize = 512;

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
    /// Where the name was written.
    pub range: TextRange,
    /// The name, as it was written.
    pub name: String,
}

/// A successfully resolved reference (for go-to-definition etc.).
#[derive(Clone, Copy, Debug)]
pub struct Reference {
    /// Index of the file it is in.
    pub file: usize,
    /// whole qualified-name range
    pub range: TextRange,
    /// final segment only (what a rename replaces)
    pub name_range: TextRange,
    /// The element the name resolved to.
    pub target: ElementId,
    /// The element whose text names it. What a model reaches is followed
    /// from here: the references of what has been resolved, not every
    /// reference the workspace has ever recorded.
    pub from: ElementId,
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

/// Everything wrong with a model, in the order it is worth saying.
///
/// [`Workspace::diagnose`] builds this; the field it adds over
/// [`Findings`] is the one no front end can be trusted to decide for
/// itself, which is whether the constraints were worth asking at all.
pub struct Diagnosis {
    /// What the parser and the resolver found.
    pub found: Findings,
    /// What the specification's own constraints found -- empty, and not
    /// merely holding, where the model was in no state to be asked.
    pub rules: rules::Checked,
    /// Whether they were asked at all.
    ///
    /// `rules` is empty both when every constraint held and when none
    /// was put, and those are not the same answer: a reader told "0
    /// violations" about a model nothing was asked of has been told
    /// nothing. Whoever reports this says which of the two it was.
    pub asked: bool,
}

/// One thing wrong, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// Index of the file (in insertion order) it is in.
    pub file: usize,
    /// The range the finding is about.
    pub range: TextRange,
    /// The parser's complaint, or the name that resolved to nothing.
    pub what: String,
}

/// What a name that resolved to nothing might have meant.
///
/// See [`Workspace::suggestions`], which is where the two halves are
/// told apart.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Suggestion {
    /// Elements that answer to the name somewhere in the workspace,
    /// spelled from the root. The name is not wrong; nothing brought it
    /// into scope where it was written, and an import would.
    pub elsewhere: Vec<String>,
    /// Declared names near enough to be worth offering, shortest first.
    /// What to look at when the name is wrong rather than unimported.
    pub near: Vec<String>,
}

/// How a connector end reached what it relates.
enum Reached {
    /// the name the statement wrote
    Written(Vec<String>, Vec<usize>),
    /// the neighbour standing in for an end the statement left unwritten
    Beside(ElementId),
}

/// What a pass of resolution came to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResolveStats {
    /// References that found what they name.
    pub resolved: usize,
    /// References that found nothing. Each is in [`Workspace::unresolved`]
    /// with the place it was written.
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
    /// The conditions an imported member has to satisfy -- the ones in
    /// the import's own brackets and the ones the importing namespace
    /// states beside it -- each with the element its names resolve from.
    filters: Vec<(SyntaxNode, ElementId)>,
}

/// Which memberships an import brings into the namespace that writes it.
///
/// `A::B::**` and `A::B::*::**` are not the same import. The first is a
/// membership import made recursive, and the specification says its
/// `importedMemberships` "returns at least the importedMembership" --
/// `B` itself -- and then, `B` being a namespace, everything below it.
/// The second is a namespace import, which brings in what is inside `B`
/// and not `B`. Reading both as the second left `import P::C::**; x : C;`
/// with nothing named `C`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportScope {
    /// `import A::B;` — one member
    Member,
    /// `import A::*;`
    Members,
    /// `import A::*::**;` — everything under A, but not A
    Recursive,
    /// `import A::B::**;` — B, and everything under it
    RecursiveMember,
}

impl ImportScope {
    /// Is the named element itself imported?
    fn brings_the_member(self) -> bool {
        matches!(self, ImportScope::Member | ImportScope::RecursiveMember)
    }

    /// Are the named element's own members imported?
    fn brings_its_members(self) -> bool {
        !matches!(self, ImportScope::Member)
    }

    /// And what is nested below those?
    fn reaches_below(self) -> bool {
        matches!(self, ImportScope::Recursive | ImportScope::RecursiveMember)
    }
}

/// Any number of parsed files, built into one model and resolved against
/// each other.
///
/// Everything is owned by a synthetic root namespace -- the KerML global
/// namespace, which a model addresses as `$`. Files are added with
/// [`Workspace::add_file`] or [`Workspace::load_dir`] and resolved with
/// [`Workspace::resolve_all`]; what is wrong with the result is
/// [`Workspace::diagnose`].
///
/// A workspace caches a great deal, and [`Clone`] is how a copy is taken
/// to ask questions of -- the language server clones the resolved standard
/// library rather than reading it again for every project.
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
    /// Elements by the whole name they answer to from the root. A
    /// constraint that names a library element asks for the same
    /// handful over and over -- once per element it is checked of --
    /// and working one out is a scan of every name in the workspace.
    pub(crate) globals: HashMap<String, Option<ElementId>>,
    /// What the expression builder stood up, in the order it made them.
    /// A `RequirementUsage` is a kind of `BooleanExpression` and a
    /// subject is a kind of parameter, so what an expression is cannot
    /// be told from the metaclass once the model is built.
    expressions: Vec<ElementId>,
    /// While reading what a redefinition written on an end names.
    ///
    /// `assoc HappensWhile specializes HappensDuring { end feature
    /// thisOccurrence redefines shorterOccurrence ... }` redefines the
    /// end its supertype declares, so the association is searched with
    /// what it inherits before anything its ends reach. What an end
    /// *refers to* is a different question -- `end feature
    /// transferSource references source` names the source of the
    /// enclosing transfer and not the one the connector's own type
    /// inherits -- which is why this is only set for a redefinition.
    redefining: bool,
    /// Relationships by the element they name at the end their
    /// association owns -- the way `Feature::typing` is read -- with the
    /// size of the model they were indexed from. Read through
    /// [`Workspace::reverse_ends`], which is what keeps the two halves
    /// of that pair agreeing.
    reverse: (usize, HashMap<(&'static str, ElementId), Vec<ElementId>>),
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
    /// Files every element of which has been resolved. A reach never
    /// re-enters one: there is nothing left in it to find, and resolving
    /// an element twice records its references twice.
    settled: HashSet<usize>,
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
            // a speculative walk may write elements, and a name that
            // resolved to nothing before one is not settled
            globals: HashMap::new(),
            expressions: Vec::new(),
            redefining: false,
            reverse: (0, HashMap::new()),
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
            settled: self.settled.clone(),
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
    /// An empty workspace, holding nothing but its root namespace.
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
            globals: HashMap::new(),
            expressions: Vec::new(),
            redefining: false,
            reverse: (0, HashMap::new()),
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
            settled: HashSet::new(),
            unresolved: Vec::new(),
            references: Vec::new(),
        }
    }

    /// Parse `text` (dialect chosen from the file name's extension) and add it
    /// to the workspace. Returns the file index.
    ///
    /// The same file read twice is one file: a caller reaching one file two
    /// ways would otherwise declare everything in it twice, and the second
    /// copy would lose every lookup to the first, silently, in a model that
    /// still resolves.
    ///
    /// A name added again over *different* text still adds a second file
    /// rather than replacing the first: a workspace hands element ids to its
    /// callers, and taking a file back would leave every id from it pointing
    /// at nothing. A front end that reopens a file builds the workspace again.
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
        self.expressions.extend(built.expressions);
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

    /// The model every file was built into.
    pub fn model(&self) -> &Model {
        &self.model
    }

    /// The root namespace everything is owned by, which a model
    /// addresses as `$`.
    pub fn root(&self) -> ElementId {
        self.root
    }

    /// What a file was added under. For a file read from disk this is
    /// its path, lossily; for a buffer it is whatever the editor calls
    /// it.
    pub fn file_name(&self, file: usize) -> &str {
        &self.files[file].name
    }

    /// The outermost elements a file declares.
    pub fn file_roots(&self, file: usize) -> &[ElementId] {
        &self.files[file].roots
    }

    /// Every reference that found nothing, with where it was written.
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

    /// [`Workspace::findings`], and the specification's own constraints after
    /// them.
    ///
    /// The order is the point, and so is the stop. A constraint asked of a
    /// model with a dangling reference answers about the hole: one undeclared
    /// type in a five-line file drew four complaints, none of them a second
    /// thing to fix. The constraints are written against the standard library
    /// too, so a workspace loaded without it is not asked them either.
    ///
    /// That rule was written out three times -- once in the command line, once
    /// in the language server, and nowhere at all in the MCP server, which
    /// asked them of anything. The decision is made here and the front ends
    /// say what came of it.
    ///
    /// The constraints are put to `files` as named. Every front end names
    /// the files it was asked about -- the command line what it was
    /// handed, the servers the documents open -- and none names the
    /// library under them, which satisfies the constraints and is not
    /// what anyone asked about.
    pub fn diagnose(&mut self, files: &[usize]) -> Diagnosis {
        let found = self.findings(files);
        let settled =
            found.syntax.is_empty() && found.names.is_empty() && self.has_standard_library();
        let rules = match settled {
            true => self.check_rules(files),
            false => rules::Checked::default(),
        };
        Diagnosis {
            found,
            rules,
            asked: settled,
        }
    }

    /// Root packages declared under a name the standard library has taken.
    ///
    /// Every file's outermost packages are members of one shared root, so a
    /// `package Requirements` of one's own and the library's are two members
    /// under one name. Resolution keeps both halves working by reading each
    /// name on the side of the library boundary it was written on -- but the
    /// name then means one thing in the model and another in the library, and
    /// nothing in the file says so. Two packages of one's own sharing a name
    /// are not reported: the official examples do it deliberately.
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

    /// Everything that answers to the same name as `elem` because it
    /// redefines (or references) it without declaring a name of its own:
    /// `part l : Logical { part :>> component; }` gives `component` a
    /// second home, and `l.component` names that one. A rename that
    /// stops at the declaration leaves those mentions behind.
    pub fn named_after(&self, elem: ElementId) -> Vec<ElementId> {
        // Who borrows a name from whom, over the whole model, once.
        //
        // This used to be asked one step at a time, and each step was a
        // scan of every element there is: with the standard library
        // loaded that is a millisecond and a quarter apiece, so a
        // feature sixty redefinitions deep cost seventy-five
        // milliseconds -- in the rename an editor is waiting on, and
        // growing with the model rather than with the answer.
        let mut borrowers: HashMap<ElementId, Vec<ElementId>> = HashMap::new();
        for heir in self.model.ids() {
            // one that named itself is its own name from here on
            if self.model.get(heir, "declaredName").is_some() {
                continue;
            }
            // Asked of the redefining side, which owns the relationship:
            // that way there is no side of it to be missing.
            for &rel in self.model.owned(heir) {
                let to = match self.model.kind(rel) {
                    ElementKind::Redefinition => "redefinedFeature",
                    ElementKind::ReferenceSubsetting => "referencedFeature",
                    _ => continue,
                };
                if let Some(&Value::Ref(at)) = self.model.maybe(rel, to) {
                    borrowers.entry(at).or_default().push(heir);
                }
            }
        }

        // where the name these all answer to is declared, which is what
        // tells a redefinition that borrows it from one that redefines
        // this feature and is called something else
        let declaration = self.model.naming_element(elem);
        let mut found = Vec::new();
        let mut queue = vec![elem];
        let mut seen: HashSet<ElementId> = std::iter::once(elem).collect();
        while let Some(at) = queue.pop() {
            for &heir in borrowers.get(&at).map_or(&[][..], Vec::as_slice) {
                // A feature that redefines more than one thing answers to
                // the first of them alone: `part :>> driver :>> driver_b`
                // is a `driver`, so `driver_b` is not the name it goes by
                // and renaming `driver_b` leaves every mention of it
                // standing.
                if self.model.naming_element(heir) != declaration {
                    continue;
                }
                if seen.insert(heir) {
                    found.push(heir);
                    queue.push(heir);
                }
            }
        }
        found
    }

    /// Every reference that resolved to `target`, for find-references
    /// and for a rename that has to reach all of them.
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

    /// The innermost model element whose syntax covers `offset` in
    /// `file` -- the workspace root when none does.
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
    /// exactly, then what starts with it, then what merely contains it, and
    /// within each the shorter name first. Searching `Natural` and being
    /// handed two SI units before `ScalarValues::Natural` is the difference
    /// between a useful answer and one to be read through.
    ///
    /// An empty query finds everything, which is what a symbol picker opens
    /// with. Names that tie keep the order the model holds them in.
    ///
    /// The language server's symbol search and the MCP server's library search
    /// are both this; they used to sort differently, and only one put an exact
    /// match first.
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

    /// The named elements whose *documentation* `query` finds, best first: the
    /// whole of it as a phrase, then its words found apart, and within each
    /// the shorter documentation first -- a paragraph largely about what was
    /// asked for before one that mentions it in passing.
    ///
    /// This is what a search over names cannot answer. Somebody transcribing a
    /// specification has the words the specification used: a sentence about
    /// "the resistance a fluid offers to flow" is asking for
    /// `ISQ::DynamicViscosityValue`, which shares not one word with it -- but
    /// the library says "viscosity" and "fluid" in its documentation.
    ///
    /// An empty query finds nothing rather than everything: every documented
    /// element in the library is not an answer to a question nobody asked.
    pub fn search_documentation(&self, query: &str, limit: usize) -> Vec<ElementId> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let words: Vec<&str> = needle.split_whitespace().collect();
        let mut found: Vec<(u8, usize, ElementId)> = self
            .named_elements()
            .filter_map(|(id, _)| {
                let doc = self.documented(id)?;
                let lowered = doc.to_lowercase();
                let rank = if lowered.contains(&needle) {
                    0
                } else if words.len() > 1 && words.iter().all(|word| lowered.contains(word)) {
                    1
                } else {
                    return None;
                };
                Some((rank, doc.len(), id))
            })
            .collect();
        found.sort_by_key(|&(rank, length, _)| (rank, length));
        found.truncate(limit);
        found.into_iter().map(|(_, _, id)| id).collect()
    }

    /// What a name that resolved to nothing might have meant.
    ///
    /// Two answers, and telling them apart is the whole point. A name the
    /// workspace declares somewhere is a right one that nothing brought into
    /// scope, and what it wants is an import. A name nothing declares is a
    /// wrong one, and what it wants is the nearest thing declared.
    ///
    /// `attribute capacity : VolumeValue;` and `attribute temp : Temperature;`
    /// are reported the same way by resolution and are not the same mistake:
    /// the first is `ISQ::VolumeValue` un-imported, the second
    /// `TemperatureValue` misremembered.
    ///
    /// Asked for several names at once because the walk over every declared
    /// name is what costs: sixty thousand with the library loaded, once rather
    /// than once per name.
    pub fn suggestions(&self, wanted: &[String]) -> Vec<Suggestion> {
        // the last segment is what a name answers to; `ISQ::Volume`
        // missed because of `Volume`, not because of `ISQ`
        let asked: Vec<&str> = wanted
            .iter()
            .map(|it| it.rsplit("::").next().unwrap_or(it))
            .collect();
        let lowered: Vec<String> = asked.iter().map(|it| it.to_lowercase()).collect();

        let mut elsewhere: Vec<Vec<ElementId>> = vec![Vec::new(); wanted.len()];
        let mut near: Vec<Vec<(u8, usize, usize, ElementId)>> = vec![Vec::new(); wanted.len()];
        // How many letters wrong is still the same name. A typo in a
        // short name is most of it, so the allowance grows with the
        // name and never falls to nothing: `Mas` for `Mass` counts,
        // `Mas` for `Materials` does not.
        let slack: Vec<usize> = lowered
            .iter()
            .map(|it| it.chars().count().max(3) / 3)
            .collect();
        for (id, name) in self.named_elements() {
            let spelled = name.to_lowercase();
            for (at, needle) in lowered.iter().enumerate() {
                if &spelled == needle {
                    elsewhere[at].push(id);
                    continue;
                }
                // A declared name that holds what was asked for -- `TemperatureValue` for
                // `Temperature`. The other way about is worth offering too, but only
                // where what is declared is a real word and most of what was asked:
                // otherwise every one-letter parameter in the library matches every query
                // containing its letter, and the answer to `VolumeValue` is `L`, `M`,
                // `o`, `u`, `v`.
                let holds = spelled.contains(needle);
                let held = spelled.len() >= 4
                    && needle.contains(&spelled)
                    && spelled.len() * 2 >= needle.len();
                // Ranked the way `search_names` ranks, and for the same reason: a name
                // that *begins* with what was asked is the one probably meant.
                // And a name that is neither, because the mistake was a letter rather
                // than a word: `Wheeel` holds no declared name and is held by none, and
                // is the commonest kind of wrong name there is.
                let (rank, off) = match (holds, held) {
                    (true, _) if spelled.starts_with(needle) => (0, 0),
                    (true, _) => (1, 0),
                    (_, true) => (2, 0),
                    _ => match within(&spelled, needle, slack[at]) {
                        Some(apart) => (3, apart),
                        None => continue,
                    },
                };
                // then how far off it was, and then the shorter name,
                // which is what puts `MassValue` above `SpecificMassValue`
                near[at].push((rank, off, name.len(), id));
            }
        }

        /// How many of each a reader can act on before the list is
        /// worse than the question.
        const MOST: usize = 5;

        (0..wanted.len())
            .map(|at| {
                let mut ranked = std::mem::take(&mut near[at]);
                ranked.sort_by_key(|&(kind, off, len, id)| (kind, off, len, id));
                Suggestion {
                    elsewhere: elsewhere[at]
                        .iter()
                        .take(MOST)
                        .map(|&id| self.qualified_name_of(id))
                        .collect(),
                    near: ranked
                        .iter()
                        .take(MOST)
                        .map(|&(_, _, _, id)| self.qualified_name_of(id))
                        .collect(),
                }
            })
            .collect()
    }

    /// The names that may legally be written at a point in a file: the
    /// members of every enclosing scope, what those inherit, and what
    /// they import, filtered by what is visible from there.
    ///
    /// This is what completion offers, so it answers the same question
    /// resolution asks and answers it the same way.
    pub fn visible_names(
        &mut self,
        file: usize,
        offset: sysml_syntax::TextSize,
    ) -> Vec<(String, ElementKind)> {
        let mut scope = self.innermost_element(file, offset);

        let mut out = Vec::new();
        let mut seen = HashSet::new();
        loop {
            self.collect_visible(scope, Access::Internal, &mut out, &mut seen, &[]);
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
        // The filters a name has to get past to have arrived here:
        // empty for the namespace being typed in, and the import's own
        // once the walk has followed one. What is offered as you type
        // has to be what a lookup would find.
        filters: &[(SyntaxNode, ElementId)],
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
            if !self.admits(child, filters) {
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
            self.collect_visible(sup, sub_access, out, seen, filters);
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
                if imp.scope.brings_the_member() && self.admits(imp.target, &imp.filters) {
                    if let Some(name) = imp
                        .leaf
                        .clone()
                        .or_else(|| self.model.name(imp.target).map(String::from))
                    {
                        out.push((name, self.model.kind(imp.target)));
                    }
                }
                if imp.scope.brings_its_members() {
                    let target_access = if imp.all {
                        Access::Internal
                    } else {
                        Access::External
                    };
                    self.collect_visible(imp.target, target_access, out, seen, &imp.filters);
                    // `import Q::**` reaches what is nested in Q as
                    // well, which is how `class Z :> F;` finds
                    // `Q::Q2::F`. Offering only Q's own members left
                    // the modeller typing blind a name the model
                    // resolves -- lookup has always followed it there.
                    if imp.scope.reaches_below() {
                        for desc in self.nested_visible(imp.target, target_access) {
                            if !self.admits(desc, &imp.filters) {
                                continue;
                            }
                            if let Some(name) = self.model.name(desc) {
                                out.push((name.to_string(), self.model.kind(desc)));
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

    /// How many files have been added. A file is named by its index
    /// into that order.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Every element built from one file, in the order the builder made
    /// them.
    pub fn file_elements(&self, file: usize) -> &[ElementId] {
        &self.files[file].elements
    }

    /// The syntax tree a file was parsed into, with whatever the parser
    /// could not read beside it.
    pub fn file_parse(&self, file: usize) -> &Parse {
        &self.files[file].parse
    }

    /// Whether the standard library is part of this workspace.
    ///
    /// The specification's constraints are written against it: they name
    /// `Base::Anything`, `Performances::Performance`, `ScalarValues::Boolean`.
    /// Asked of a model loaded without the library they answer about what is
    /// missing rather than about the model -- `case def Trip { objective
    /// placed; }` draws four complaints on its own and none with the library
    /// beside it. `Base::Anything` is there exactly when the library is.
    pub fn has_standard_library(&mut self) -> bool {
        self.named_globally("Base::Anything").is_some()
    }

    /// The elements that name `of` at the association end `end`.
    ///
    /// `Feature::typing` is "the FeatureTypings for which a certain Feature is
    /// the typedFeature": a property no metaclass declares, because the
    /// association that has it owns the end. The model keeps the relationship,
    /// so the answer is found the other way about -- and read one at a time
    /// each is a scan of every element, walked over every type a feature
    /// reaches, which is the difference between a check taking seconds and a
    /// quarter of a minute.
    ///
    /// So it is indexed once, for every such end at a time. `fill` builds that
    /// index; which ends there are is the metamodel's business, stated where
    /// the constraints are read.
    ///
    /// The model grows while the constraints are checked -- a derivation may
    /// reify what it reads -- so the index carries the size it was built for
    /// and is built again when that has moved. Stamping it here rather than at
    /// the call site is what stops the two disagreeing.
    pub(crate) fn reverse_ends(
        &mut self,
        fill: impl FnOnce(&Model) -> HashMap<(&'static str, ElementId), Vec<ElementId>>,
        end: &'static str,
        of: ElementId,
    ) -> &[ElementId] {
        if self.reverse.0 != self.model.len() {
            self.reverse = (self.model.len(), fill(&self.model));
        }
        self.reverse.1.get(&(end, of)).map_or(&[], Vec::as_slice)
    }

    /// Where an element is written, for a report that points at it.
    ///
    /// A constraint names the element it does not hold of, and not every
    /// element of a model was written down: an implied specialization, a
    /// reified connector end, a membership put back together from the
    /// containment it stands for. None of those has syntax of its own,
    /// and what a reader goes and looks at instead is the nearest thing
    /// that does -- whatever owns it.
    pub fn element_place(&self, elem: ElementId) -> Option<(usize, TextRange)> {
        let mut at = Some(elem);
        while let Some(id) = at {
            if let (Some(file), Some((_, name))) = (self.element_file(id), self.element_ranges(id))
            {
                return Some((file, name));
            }
            at = self.model.owner(id);
        }
        None
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

    /// The `doc` body attached to an element, if any, as prose.
    ///
    /// What the parser keeps is the inside of the comment, margin and all: a
    /// `doc /* ... */` over several lines arrives as `"* Two states, and the
    /// LED follows\n         * which one is current"`. The `*` down the left
    /// is decoration and so is the indentation that carried it, and every
    /// reader wanting the sentence had to know that -- the diagrams stripped
    /// it, hover and the MCP server did not.
    pub fn documentation_of(&self, elem: ElementId) -> Option<String> {
        self.documented(elem).map(prose)
    }

    /// The same, without the copy -- which is what a search over every
    /// documented element in the library wants.
    fn documented(&self, elem: ElementId) -> Option<&str> {
        self.model
            .owned(elem)
            .iter()
            .find(|c| self.model.kind(**c) == ElementKind::Documentation)
            .and_then(|d| self.model.maybe(*d, "body"))
            .and_then(Value::as_str)
    }
}

/// A block comment's body as the sentences in it.
///
/// The margin comes off each line and the blank lines that a `/* ... */`
/// begins and ends with come off the whole, so what is left is what was
/// written. A single-line doc passes through untouched, which is most of
/// them.
fn prose(body: &str) -> String {
    let lines: Vec<&str> = body
        .lines()
        .map(|line| line.trim().trim_start_matches('*').trim())
        .collect();
    lines.join("\n").trim().to_string()
}

/// How many letters apart two names are, given up on once they are more
/// than `most` apart.
///
/// A name that resolved to nothing is usually one that was nearly written:
/// a letter doubled, one missed, two the other way round. Substring
/// matching finds none of those -- `Wheeel` neither holds `Wheel` nor is
/// held by it -- and this does, the way rustc's own suggestions do.
///
/// A row is given up as soon as every way through it costs more than
/// `most`, which is what makes a walk over sixty thousand names worth
/// doing: almost all are abandoned on their first letter.
fn within(name: &str, wanted: &str, most: usize) -> Option<usize> {
    let name: Vec<char> = name.chars().collect();
    let wanted: Vec<char> = wanted.chars().collect();
    if name.len().abs_diff(wanted.len()) > most {
        return None;
    }
    // the row two above, which is where a pair the other way round is
    // reached from
    let mut earlier: Vec<usize> = Vec::new();
    let mut before: Vec<usize> = (0..=wanted.len()).collect();
    for (down, &here) in name.iter().enumerate() {
        let mut row = Vec::with_capacity(wanted.len() + 1);
        row.push(down + 1);
        let mut best = down + 1;
        for (across, &there) in wanted.iter().enumerate() {
            let mut next = (before[across + 1] + 1)
                .min(row[across] + 1)
                .min(before[across] + usize::from(here != there));
            // Two letters the other way round is one mistake and not
            // two, which is the difference between offering `Wheel` for
            // `Whele` and offering nothing: a swap is what a pair of
            // fingers does, and it is as common as a letter missed.
            if down > 0 && across > 0 && here == wanted[across - 1] && name[down - 1] == there {
                next = next.min(earlier[across - 1] + 1);
            }
            best = best.min(next);
            row.push(next);
        }
        if best > most {
            return None;
        }
        earlier = std::mem::replace(&mut before, row);
    }
    let apart = before[wanted.len()];
    (apart <= most).then_some(apart)
}

/// Does `elem` already specialize `base`, walking the reified
/// specialization relationships the model holds -- the implied ones a
/// materialization pass has written included?
pub(crate) fn reaches(model: &Model, elem: ElementId, base: ElementId) -> bool {
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
            if let Some(Value::Ref(target)) = model.maybe(child, target) {
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
pub(crate) fn implied_bases(kind: ElementKind) -> Vec<&'static str> {
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
/// indirectly, the Association BinaryLink" -- and the same of an
/// association. Each is listed in front of what it narrows, so dropping it
/// leaves the one an n-ary relationship reaches.
pub(crate) const BINARY: [&str; 5] = [
    "Links::BinaryLink",
    "Objects::BinaryLinkObject",
    "Connections::BinaryConnection",
    "Interfaces::BinaryInterface",
    "Flows::Message",
];

/// What is implied only of something that owns ends of its own, however
/// many. `checkFlowUsageFlowSpecialization` asks for `notEmpty`, where
/// the binary rules above ask for exactly two.
pub(crate) const OWNING_ENDS: [&str; 1] = ["Flows::flows"];

/// The library types every definition/usage of a given metaclass
/// implicitly specializes (KerML §7 / SysML §9 semantic library
/// mappings, abridged: only what inherited-member lookup needs).
/// Targets that are not loaded in the workspace are silently skipped.
pub(crate) fn implicit_supertype(kind: ElementKind) -> &'static [&'static str] {
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
        // The library says which is which in as many words: "MetadataItem is the
        // base type of all MetadataDefinitions", and "metadataItems is the base
        // feature of all MetadataUsages". Given the type, a usage was typed by a
        // second metaclass beside the one it names -- a `MetadataDefinition` is a
        // `Metaclass` -- and `validateMetadataFeatureMetaclass`, which asks for
        // exactly one, reported seventy-two sound models.
        MetadataDefinition => &["Metadata::MetadataItem"],
        MetadataUsage => &["Metadata::metadataItems"],
        OccurrenceDefinition | OccurrenceUsage | EventOccurrenceUsage => {
            &["Occurrences::Occurrence"]
        }
        // `Flow` is a *sub*class of `Message`, and there is no
        // `Flows::MessageFlow` at all: a flow was inheriting the ends of
        // the very type that specializes it.
        FlowDefinition => &["Flows::Message", "Flows::MessageAction"],
        FlowUsage => &["Flows::flows", "Flows::messages"],
        SuccessionFlowUsage => &["Flows::successionFlows"],
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
        // The library names no evaluation after an infinite literal,
        // but the metamodel says which one it is all the same:
        // `checkLiteralInfinitySpecialization` is
        // `specializesFromLibrary('Performances::literalIntegerEvaluations')`,
        // the same one an integer literal specializes. Its result is
        // then an integer, which is what
        // `validateMultiplicityRangeBoundResultTypes` asks of the `*`
        // in `[0..*]`.
        LiteralInfinity => &["Performances::literalIntegerEvaluations"],
        FeatureReferenceExpression => &["Performances::evaluations"],
        Connector => &["Links::links"],
        BindingConnector => &["Links::selfLinks"],
        _ => &[],
    }
}

/// Record `target` as a supertype of `elem`, ignoring a self-reference and
/// a target another clause already contributed.
pub(crate) fn push_supertype(supers: &mut Vec<ElementId>, elem: ElementId, target: ElementId) {
    if target != elem && !supers.contains(&target) {
        supers.push(target);
    }
}

/// Whether a path names a model file: `.sysml` or `.kerml`.
pub fn is_model_file(path: &std::path::Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("sysml" | "kerml")
    )
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
        } else if is_model_file(&path) {
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
        // four relationship targets and the path `import Base::*` writes
        assert_eq!((stats.resolved, stats.unresolved), (5, 0), "{report}");
        // relationships were reified
        let model = ws.model();
        let count = |k: ElementKind| model.ids().filter(|id| model.kind(*id) == k).count();
        assert_eq!(count(ElementKind::Subclassification), 1);
        assert_eq!(count(ElementKind::FeatureTyping), 2);
        assert_eq!(count(ElementKind::Redefinition), 1);
    }

    /// A report points at what a reader can go and look at.
    ///
    /// Most of a model was never written down: of the official corpus's 109616
    /// elements only 30235 have syntax of their own, and the other 79380 -- an
    /// implied specialization, a reified connector end, a multiplicity -- are
    /// shown where whatever owns them was written. The one element under no
    /// file is the root, and nothing a constraint is asked of is that.
    #[test]
    fn what_was_never_written_is_shown_where_its_owner_was() {
        let (ws, _) = resolved_workspace(&[(
            "m.sysml",
            "package P {\n\tpart def Wheel;\n\tpart w : Wheel[2];\n}\n",
        )]);
        let named = |ws: &Workspace, want: &str| {
            ws.model()
                .ids()
                .find(|&id| ws.model().name(id) == Some(want))
                .expect("declared")
        };
        let wheel = named(&ws, "w");
        let (file, at) = ws.element_place(wheel).expect("`w` is written");
        assert_eq!(
            ws.file_parse(file).syntax().text().to_string()[at].to_string(),
            "w"
        );

        // the typing of `w` is a relationship the notation implies and
        // never writes, so it is shown at the `w` that carries it
        let typing = ws
            .model()
            .owned(wheel)
            .iter()
            .copied()
            .find(|&it| ws.model().kind(it) == ElementKind::FeatureTyping)
            .expect("the typing is reified");
        assert_eq!(ws.element_ranges(typing), None, "it was never written");
        assert_eq!(ws.element_place(typing), Some((file, at)));

        // and the root of the workspace is under no file
        let root = ws
            .model()
            .ids()
            .find(|&id| ws.model().owner(id).is_none())
            .expect("everything is read from a root");
        assert_eq!(ws.element_place(root), None);
    }

    /// `subset g subsets f;` relates the same two features as `feature g :>
    /// f;`, with the relationship written as the statement instead of reified
    /// under a declaration. Read only where a declaration carries it, the
    /// statement form reached the model relating nothing to nothing.
    ///
    /// `disjoining d disjoint A from B;` writes its two types in a shape of
    /// its own -- the name first, which leaves `disjoint A` a part and `B` the
    /// bare operand after `from` -- and is read by position either way. The
    /// last two statements miss on either side, which leaves that end unsaid
    /// rather than guessed.
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
                (ElementKind::Disjoining, Some("A"), Some("B")),
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
        assert_eq!(stats.resolved, 2); // `import A::*` and `inside : Widget`
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

    /// The specification's own tables, in full: every operator symbol
    /// the notation writes maps to a function in the Kernel Function
    /// Library, and every trigger keyword to one in the Kernel Semantic
    /// Library. An invocation that finds none of them says what it
    /// invokes nowhere.
    #[test]
    fn every_symbol_the_specification_tabulates_names_its_function() {
        let mapped: Vec<(&str, &str)> = [
            "all", "istype", "hastype", "@", "@@", "as", "meta", "==", "!=", "===", "!==", "[",
            "#", ",", ".", "if", "??", "and", "or", "implies", "collect", "select", "xor", "not",
            "~", "|", "&", "<", ">", "<=", ">=", "+", "-", "*", "/", "%", "^", "**", "..",
        ]
        .into_iter()
        .map(|operator| {
            (
                operator,
                invoked_function(operator)
                    .unwrap_or_else(|| panic!("the table names a function for `{operator}`")),
            )
        })
        .collect();
        // each is read from one of the three packages the table names
        assert!(mapped.iter().all(|(_, named)| {
            named.starts_with("BaseFunctions::")
                || named.starts_with("ControlFunctions::")
                || named.starts_with("DataFunctions::")
        }));
        // `^` and `**` are the one function, written two ways
        assert_eq!(invoked_function("^"), invoked_function("**"));
        // and a symbol the table does not name invokes nothing
        assert_eq!(invoked_function("<=>"), None);

        // the second column: only a type extent and the two the library
        // leaves undefined cannot be evaluated at model level
        let cannot: Vec<&str> = mapped
            .iter()
            .filter(|(_, named)| evaluable_at_model_level(named) == Some(false))
            .map(|(operator, _)| *operator)
            .collect();
        assert_eq!(cannot, vec!["all", "[", "~"]);
        assert_eq!(
            evaluable_at_model_level("DataFunctions::+"),
            Some(true),
            "addition is evaluated at model level"
        );
        assert_eq!(evaluable_at_model_level("Nowhere::atAll"), None);

        // the three trigger kinds name the three trigger functions,
        // and nothing else names one
        assert_eq!(triggered_function("when"), Some("Triggers::TriggerWhen"));
        assert_eq!(triggered_function("at"), Some("Triggers::TriggerAt"));
        assert_eq!(triggered_function("after"), Some("Triggers::TriggerAfter"));
        assert_eq!(triggered_function("whenever"), None);
    }

    #[test]
    fn kerml_dialect_and_short_names() {
        let (ws, stats) = resolved_workspace(&[(
            "k.kerml",
            "package K {\n  classifier <B> Base;\n  classifier Derived :> B;\n  feature f : Derived;\n}",
        )]);
        assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
    }

    /// A whole name is answered however many elements share its last segment.
    ///
    /// The library types the standard implies are found by their whole name,
    /// and the last segment of one is often what a model calls its own
    /// features: `x` names two hundred and seventy elements of the published
    /// corpus by itself. Taking only the first few hundred candidates left the
    /// answer to how many other things happened to be called the same.
    #[test]
    fn a_name_shared_by_a_crowd_still_answers() {
        let crowd = (0..600)
            .map(|at| format!("  package P{at} {{ classifier x; }}\n"))
            .collect::<String>();
        let model = format!("package Crowd {{\n{crowd}}}\npackage Deep {{\n  classifier x;\n}}\n");
        let (mut ws, _) = resolved_workspace(&[("m.kerml", &model)]);
        let found = ws.named_globally("Deep::x").expect("`Deep::x` is declared");
        assert_eq!(ws.qualified_name_of(found), "Deep::x");
        assert!(ws.named_globally("Crowd::P599::x").is_some());
        assert!(ws.named_globally("Deep::nothing").is_none());
    }
}

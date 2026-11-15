//! A model read from interchange JSON, written back out as the textual
//! notation.
//!
//! A document of the standard's JSON is an array of objects with UUIDs for
//! names, and what a reader wants to know of one is what a person would
//! have written: `part def Truck :> Vehicle { part :>> wheels[6]; }`. This
//! writes what a model holds the way the notation says it -- packages,
//! imports (filtered ones too), aliases, comments, definitions and usages
//! with their typings, specializations, subsettings, redefinitions,
//! multiplicities and values; connections, bindings, flows, messages,
//! successions, allocations and interfaces by the ends they connect, and
//! `then b;` where a succession follows the step before it; transitions
//! with their triggers, guards and effects; the action statements
//! (`accept`, `send`, `assign`, `if`, `while`, `loop`, `for`, the control
//! nodes, `terminate`); `perform`, `exhibit`, `include`, `satisfy`,
//! `assert`, events and the roles that refer to what plays them;
//! metadata, dependencies, filters and exposures; KerML's connectors and
//! its unions, intersections, differences, disjoinings, featurings and
//! inversions; and expressions -- and says of anything else that it was
//! not written, rather than guessing at it, since a wrong guess reads as a
//! model that says something it does not.
//!
//! An expression is written as the text it was written as where the
//! document keeps that (a `TextualRepresentation` in `sysml`, which is what
//! `sysml export` attaches to every one), and otherwise from what it is:
//! the operator and its arguments, what it invokes, what it refers to.
//! The text is preferred because it is exact: `v.m` and the feature `m`
//! it comes to are not the same thing, and a model that has resolved the
//! one may hold only the other.
//!
//! Every name it refers to something by is the name that would find it
//! from where it is written: the name alone where an import brings it in,
//! the path down from the nearest namespace around it that holds it --
//! which is how a private member is reached at all -- and the whole path,
//! from `$::` where something nearer answers to its first name, where
//! neither will do.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use sysml_model::{ElementId, ElementKind as K, Model, Role, Value, Vis};

/// What came of writing a model out.
pub struct Written {
    /// The notation.
    pub sysml: String,
    /// Whether that is KerML rather than SysML: a model of classes and
    /// features is written in the dialect those keywords are in.
    pub kerml: bool,
    /// Each element that was left out, as its metaclass and the name of
    /// the element it is in.
    pub unwritten: Vec<(String, String)>,
}

/// Write `roots` and everything under them.
///
/// `outside` holds the elements that stand for something the document
/// only refers to, each with the name it is to be written as: they are
/// named where they are used and declared nowhere.
pub fn write(model: &Model, roots: &[ElementId], outside: &HashMap<ElementId, String>) -> Written {
    write_with(model, roots, outside, true)
}

/// [`write`], with `kept_text` saying whether an expression is written as
/// the text the document kept of it, where it kept one. Without, every
/// expression is written from what it is, as one from a document that
/// keeps no text is: an expression's text and its structure can be told
/// apart only by asking for each.
pub fn write_with(
    model: &Model,
    roots: &[ElementId],
    outside: &HashMap<ElementId, String>,
    kept_text: bool,
) -> Written {
    let mut writer = Writer {
        model,
        outside,
        out: String::new(),
        unwritten: Vec::new(),
        kerml: false,
        structural: !kept_text,
        scope: None,
        done: HashSet::new(),
        unnamed: HashSet::new(),
        importing: false,
    };
    for &root in roots {
        if outside.contains_key(&root) {
            continue;
        }
        // the namespace a workspace hangs its files off has no name and is
        // no declaration: what it holds is the top level of the text
        if model.kind(root) == K::Namespace && model.name(root).is_none() {
            writer.members(root, 0, &[]);
        } else {
            writer.member(root, 0);
        }
    }
    Written {
        sysml: writer.out,
        kerml: writer.kerml,
        unwritten: writer.unwritten,
    }
}

struct Writer<'m> {
    model: &'m Model,
    outside: &'m HashMap<ElementId, String>,
    out: String,
    unwritten: Vec<(String, String)>,
    kerml: bool,
    /// Write every expression from what it is, never from the text it
    /// kept: what a document from another tool gets.
    structural: bool,
    /// The namespace the name being written is written in, which is what
    /// it is looked up from: the owner of the declaration being written.
    scope: Option<ElementId>,
    /// What has been written already, as part of something before it:
    /// `then action b;` declares `b` and the succession to it at once.
    done: HashSet<ElementId>,
    /// Elements whose name the model gave them rather than the text: the
    /// `effect` of a transition and the `trigger` it accepts, which a
    /// model is built with and a declaration of that name is not.
    unnamed: HashSet<ElementId>,
    /// Whether what is being written is an import, whose name is looked up
    /// without the imports beside it: `import ScalarValues::Real;` is not
    /// `import Real;` for being in a package that imports it.
    importing: bool,
}

/// How a connector is written: the keyword its declaration takes, and the
/// clause that names its ends, with what stands between two of them.
struct Connects {
    /// `connection`, `binding`, ... -- written only where the connector
    /// has a declaration of its own: a name, a type, a multiplicity.
    keyword: &'static str,
    /// `connect`, `bind`, `first`, `allocate`, `from` ...
    lead: &'static str,
    /// `to`, `=`, `then`
    between: &'static str,
    /// Whether the clause may stand without the keyword in front of it:
    /// `connect a to b;` but not `from a to b;`.
    alone: bool,
}

impl Writer<'_> {
    /// Every member of `owner` that is a declaration of its own, one after
    /// another, but those in `skip`, which whatever owns them has said
    /// already. Relationships are said by the declaration they belong to.
    fn members(&mut self, owner: ElementId, depth: usize, skip: &[ElementId]) {
        for child in self.body_of(owner, skip) {
            self.member(child, depth);
        }
    }

    /// The members [`members`](Self::members) would write.
    fn body_of(&self, owner: ElementId, skip: &[ElementId]) -> Vec<ElementId> {
        self.model
            .owned(owner)
            .iter()
            .copied()
            .filter(|&child| {
                !self.outside.contains_key(&child)
                    && !self.said_in_the_header(child)
                    && !skip.contains(&child)
                    && !self.done.contains(&child)
            })
            .collect()
    }

    /// Whether `child` is said by the declaration that owns it, or is
    /// nothing the text says at all.
    fn said_in_the_header(&self, child: ElementId) -> bool {
        let model = self.model;
        let kind = model.kind(child);
        // what the toolchain implied, the reader of the text implies again
        if model.flag(child, "isImplied") {
            return true;
        }
        // `:> B` is said by the type it is about; a relationship declared
        // in a namespace, `specialization subtype A specializes B;`, is a
        // member of its own
        if let Some((source, _, _, _)) = relates(kind) {
            return match model.maybe(child, source) {
                Some(Value::Ref(source)) => Some(*source) == model.owner(child),
                _ => true,
            };
        }
        // the chain of features a name with dots in it stands for
        if kind == K::Feature && model.name(child).is_none() && !self.chain(child).is_empty() {
            return true;
        }
        matches!(
            kind,
            K::FeatureTyping
                | K::ConjugatedPortTyping
                | K::Subclassification
                | K::Subsetting
                | K::Redefinition
                | K::ReferenceSubsetting
                | K::Specialization
                | K::Conjugation
                | K::CrossSubsetting
                | K::Unioning
                | K::Intersecting
                | K::Differencing
                | K::FeatureInverting
                | K::TypeFeaturing
                | K::FeatureValue
                | K::MultiplicityRange
                | K::FeatureChaining
                // what a comment or metadata is about is said by it
                | K::Annotation
                // `~P` is declared by `port def P` without a word
                | K::ConjugatedPortDefinition
                | K::PortConjugation
        )
    }

    fn member(&mut self, elem: ElementId, depth: usize) {
        if !self.done.insert(elem) {
            return;
        }
        let saved = std::mem::replace(&mut self.scope, self.model.owner(elem));
        self.member_in_scope(elem, depth);
        self.scope = saved;
    }

    fn member_in_scope(&mut self, elem: ElementId, depth: usize) {
        let model = self.model;
        let kind = model.kind(elem);
        let indent = "    ".repeat(depth);
        match kind {
            K::Documentation | K::Comment => {
                let body = model
                    .maybe(elem, "body")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    // a body cannot hold the end of the comment it is in
                    .replace("*/", "* /");
                let mut line = format!("{indent}{}", self.prefix(elem));
                line.push_str(match kind {
                    K::Documentation => "doc",
                    _ => "comment",
                });
                if let Some(short) = model
                    .maybe(elem, "declaredShortName")
                    .and_then(Value::as_str)
                {
                    let _ = write!(line, " <{}>", sysml_syntax::quote(short));
                }
                if let Some(name) = model.name(elem) {
                    let _ = write!(line, " {}", sysml_syntax::quote(name));
                }
                let locale = model.maybe(elem, "locale").and_then(Value::as_str);
                // `comment about A` -- what it annotates, where that is not
                // what it is in
                let about = self.about(elem);
                if kind == K::Comment && !about.is_empty() {
                    match about
                        .iter()
                        .map(|&it| self.name_of(it))
                        .collect::<Option<Vec<_>>>()
                    {
                        Some(named) => {
                            let _ = write!(line, " about {}", named.join(", "));
                        }
                        None => {
                            self.leave_out(elem, &indent);
                            return;
                        }
                    }
                }
                if let Some(locale) = locale {
                    let _ = write!(line, " locale {}", string(locale));
                }
                let _ = writeln!(self.out, "{line} {}", comment(&body, &indent));
                return;
            }
            K::TextualRepresentation => {
                let language = model
                    .maybe(elem, "language")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let body = model
                    .maybe(elem, "body")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .replace("*/", "* /");
                let mut line = format!("{indent}{}rep", self.prefix(elem));
                if let Some(short) = model
                    .maybe(elem, "declaredShortName")
                    .and_then(Value::as_str)
                {
                    let _ = write!(line, " <{}>", sysml_syntax::quote(short));
                }
                if let Some(name) = model.name(elem) {
                    let _ = write!(line, " {}", sysml_syntax::quote(name));
                }
                let _ = writeln!(
                    self.out,
                    "{line} language {} {}",
                    string(language),
                    comment(&body, &indent)
                );
                return;
            }
            K::NamespaceImport | K::MembershipImport => {
                self.import(elem, depth, "import");
                return;
            }
            K::NamespaceExpose | K::MembershipExpose => {
                self.import(elem, depth, "expose");
                return;
            }
            K::ElementFilterMembership => {
                let condition = match model.maybe(elem, "condition") {
                    Some(Value::Ref(condition)) => Some(*condition),
                    _ => model.owned(elem).first().copied(),
                };
                match condition.and_then(|it| self.expr(it)) {
                    Some(said) => {
                        let _ = writeln!(self.out, "{indent}{}filter {said};", self.prefix(elem));
                    }
                    None => self.leave_out(elem, &indent),
                }
                return;
            }
            K::Dependency => {
                self.dependency(elem, &indent);
                return;
            }
            // `specialization Gen subtype A specializes B;`, `disjoint A
            // from B;` -- a relationship between two things neither of
            // which is where it is written
            _ if relates(kind).is_some() => {
                let Some((source, target, declared, words)) = relates(kind) else {
                    return;
                };
                let named = |prop: &str| match model.maybe(elem, prop) {
                    Some(Value::Ref(it)) => self.feature_ref(*it),
                    _ => None,
                };
                match (named(source), named(target)) {
                    (Some(one), Some(other)) => {
                        let mut line = format!("{indent}{}", self.prefix(elem));
                        let (before, between) = words;
                        match (model.name(elem), kind) {
                            // `featuring F of y by C;` names it in the
                            // middle of the one word
                            (Some(name), K::TypeFeaturing) => {
                                let _ = write!(
                                    line,
                                    "featuring {} of {one} by {other}",
                                    sysml_syntax::quote(name)
                                );
                            }
                            (Some(name), _) => {
                                let _ = write!(
                                    line,
                                    "{declared} {} {before} {one} {between} {other}",
                                    sysml_syntax::quote(name)
                                );
                            }
                            (None, _) => {
                                let _ = write!(line, "{before} {one} {between} {other}");
                            }
                        }
                        self.finish(line, elem, depth, &[]);
                    }
                    _ => self.leave_out(elem, &indent),
                }
                return;
            }
            // `first start;` -- the step an action begins with, named
            K::Membership if model.maybe(elem, "memberName").is_none() => {
                let first = match model.maybe(elem, "memberElement") {
                    Some(Value::Ref(target)) => self.feature_ref(*target),
                    _ => None,
                };
                match first {
                    Some(first) => {
                        let _ = writeln!(self.out, "{indent}first {first};");
                    }
                    None => self.leave_out(elem, &indent),
                }
                return;
            }
            // an alias is a membership that carries a name of its own
            K::Membership => {
                let target = match model.maybe(elem, "memberElement") {
                    Some(Value::Ref(target)) => self.name_of(*target),
                    _ => None,
                };
                let name = model.maybe(elem, "memberName").and_then(Value::as_str);
                if let (Some(target), Some(name)) = (target, name) {
                    let line = format!(
                        "{indent}{}alias {} for {target}",
                        self.prefix(elem),
                        sysml_syntax::quote(name)
                    );
                    self.finish(line, elem, depth, &[]);
                } else {
                    self.leave_out(elem, &indent);
                }
                return;
            }
            _ => {}
        }
        // the trailing expression of a body is written as itself, last
        if model.member_role(elem) == Some(Role::Result) && kind.is_a(K::Expression) {
            match self.expr(elem) {
                Some(said) => {
                    let _ = writeln!(self.out, "{indent}{said}");
                }
                None => self.leave_out(elem, &indent),
            }
            return;
        }
        if kind == K::SuccessionAsUsage && self.then(elem, depth) {
            return;
        }
        if kind == K::TransitionUsage && self.transition_after(elem, depth) {
            return;
        }
        if let Some(connects) = connects(kind) {
            if self.connector(elem, depth, &connects) {
                return;
            }
        }
        let written = match kind {
            K::TransitionUsage => self.transition(elem, depth),
            K::AcceptActionUsage => self.accept(elem, depth),
            K::SendActionUsage => self.send(elem, depth),
            K::AssignmentActionUsage => self.assign(elem, depth),
            K::IfActionUsage => self.if_action(elem, depth, false),
            K::WhileLoopActionUsage => self.while_loop(elem, depth),
            K::ForLoopActionUsage => self.for_loop(elem, depth),
            K::TerminateActionUsage => self.terminate(elem, depth),
            K::SatisfyRequirementUsage => self.satisfy(elem, depth),
            K::MetadataUsage | K::MetadataFeature => self.metadata(elem, depth),
            _ => match keyword(kind) {
                Some(keyword) => {
                    self.declaration(elem, depth, keyword);
                    true
                }
                None => false,
            },
        };
        if !written {
            self.leave_out(elem, &indent);
        }
    }

    /// `keyword name : T :> S [m] = v { ... }`, with what goes in front of
    /// it -- visibility, role, direction and the flags.
    fn declaration(&mut self, elem: ElementId, depth: usize, keyword: &str) {
        let model = self.model;
        let kind = model.kind(elem);
        let indent = "    ".repeat(depth);
        if kerml_only(kind) {
            self.kerml = true;
        }
        let mut words: Vec<String> = Vec::new();
        let prefix = self.prefix(elem);
        if !prefix.is_empty() {
            words.push(prefix.trim_end().to_string());
        }
        // `member feature f;` -- a feature a type owns without its being a
        // feature of it
        if model.owning_member(elem)
            && kind.is_a(K::Feature)
            && model
                .owner(elem)
                .is_some_and(|owner| model.kind(owner).is_a(K::Type))
            && !model.flag(elem, "isEnd")
        {
            words.push("member".to_string());
        }
        let role = model.member_role(elem);
        let said = match role {
            Some(Role::Subject) => "subject",
            Some(Role::Actor) => "actor",
            Some(Role::Stakeholder) => "stakeholder",
            Some(Role::Objective) => "objective",
            Some(Role::Return) => "return",
            // the literals of an enumeration are its variants, and are
            // written as nothing more than literals
            Some(Role::Variant) if self.in_enumeration(elem) => "",
            Some(Role::Variant) => "variant",
            Some(Role::Entry) => "entry",
            Some(Role::Do) => "do",
            Some(Role::Exit) => "exit",
            Some(Role::Assume) => "assume",
            Some(Role::Require) => "require",
            Some(Role::Frame) => "frame",
            Some(Role::Verify) => "verify",
            Some(Role::Render) => "render",
            Some(Role::Result) | None => "",
        };
        if !said.is_empty() {
            words.push(said.to_string());
        }
        // a parameter's role says its direction, and the notation will not
        // have it said twice
        let parameter = matches!(
            role,
            Some(Role::Subject | Role::Actor | Role::Stakeholder | Role::Return | Role::Objective)
        );
        let direction = model.direction(elem).filter(|_| !parameter);
        if let Some(direction) = direction {
            words.push(direction.to_string());
        }
        // an enumeration is a variation of its literals, and says so by
        // being one: `enum def` writes neither word
        // a message is an abstract flow, and says so by its keyword
        let message = kind == K::FlowUsage && model.flag(elem, "isAbstract");
        let keyword = if message { "message" } else { keyword };
        // the words in front of the keyword, in the order each notation
        // takes them: KerML's `const end feature b;` and `derived var
        // feature v;`, SysML's `derived constant ref attribute y;` and `end
        // [1] item a;`
        let flag = |name: &str| model.flag(elem, name);
        let end = flag("isEnd");
        let kerml = !kind.is_a(K::Usage) && !kind.is_a(K::Definition);
        let mut modifiers: Vec<&str> = Vec::new();
        let mut prefixed = Vec::new();
        let mut cross = None;
        if end {
            cross = self.cross_multiplicity(elem);
        }
        if kerml {
            if end {
                if flag("isConstant") {
                    modifiers.push("const");
                }
                modifiers.push("end");
            } else {
                for (name, word) in [("isDerived", "derived"), ("isAbstract", "abstract")] {
                    if flag(name) {
                        modifiers.push(word);
                    }
                }
                if flag("isComposite") && !kind.is_a(K::Connector) {
                    modifiers.push("composite");
                } else if flag("isPortion") {
                    modifiers.push("portion");
                }
                if flag("isConstant") {
                    modifiers.push("const");
                } else if flag("isVariable") {
                    modifiers.push("var");
                }
            }
        } else if !message && kind != K::EnumerationDefinition {
            if end {
                modifiers.push("end");
            }
            // `abstract` goes without saying of a variation
            for (name, word) in [
                ("isDerived", "derived"),
                ("isVariation", "variation"),
                ("isConstant", "constant"),
            ] {
                if flag(name) {
                    modifiers.push(word);
                }
            }
            if flag("isAbstract") && !flag("isVariation") {
                modifiers.insert(
                    modifiers
                        .iter()
                        .position(|w| *w == "variation" || *w == "constant")
                        .unwrap_or(modifiers.len()),
                    "abstract",
                );
            }
        } else if end {
            modifiers.push("end");
        }
        words.extend(modifiers.iter().map(|it| it.to_string()));
        // `end [1] port p;` -- the cross feature of an end, after `end`
        if let Some((feature, multiplicity)) = cross {
            words.push(multiplicity);
            prefixed.push(feature);
        }
        // a usage of a kind that composes, held by reference, says so
        if self.referential(elem) && !end {
            words.push("ref".to_string());
        }
        if flag("isIndividual") {
            words.push("individual".to_string());
        }
        // `render asTreeDiagram;`, `verify r;`, `require c;` -- a role
        // that refers to what plays it, rather than declaring it
        let refers = matches!(
            role,
            Some(Role::Render | Role::Verify | Role::Frame | Role::Require | Role::Assume)
        ) && self.declared(elem).is_none()
            && model.maybe(elem, "declaredShortName").is_none();
        if let Some(target) = refers.then(|| self.only_referenced(elem)).flatten() {
            words.push(target);
            let mut line = words.join(" ");
            self.header(elem, &mut line, true);
            let line = format!("{indent}{}", line.trim_start());
            self.finish(line, elem, depth, &[]);
            return;
        }
        // `#service def X;` -- a definition or usage of no kind of its own
        // is written with the metadata that says what it is in front
        if matches!(kind, K::Definition | K::Usage) {
            for &child in model.owned(elem) {
                if model.kind(child) != K::MetadataUsage
                    || model.name(child).is_some()
                    || !self.body_of(child, &[]).is_empty()
                    || !self.about(child).is_empty()
                {
                    continue;
                }
                let typed = model.owned(child).iter().find_map(|&rel| {
                    match (model.kind(rel), model.maybe(rel, "type")) {
                        (K::FeatureTyping, Some(Value::Ref(ty))) => Some(*ty),
                        _ => None,
                    }
                });
                if let Some(named) = typed.and_then(|ty| self.name_of(ty)) {
                    words.push(format!("#{named}"));
                    prefixed.push(child);
                }
            }
        }
        // `perform x;` -- a performance, exhibition, inclusion or event of
        // no name of its own is written as the one thing it refers to
        let shorthand = performs(kind).and_then(|word| {
            let anonymous =
                self.declared(elem).is_none() && model.maybe(elem, "declaredShortName").is_none();
            anonymous
                .then(|| self.only_referenced(elem))
                .flatten()
                .map(|target| (word, target))
        });
        if let Some((word, target)) = shorthand {
            let in_role = matches!(role, Some(Role::Entry | Role::Do | Role::Exit));
            match kind == K::AssertConstraintUsage && model.flag(elem, "isNegated") {
                true => words.push(format!("{word} not {target}")),
                // `do senseTemperature;` performs what it names
                false if in_role && kind == K::PerformActionUsage => words.push(target),
                false => words.push(format!("{word} {target}")),
            }
            let mut line = format!("{indent}{}", words.join(" "));
            self.header(elem, &mut line, true);
            self.finish(line, elem, depth, &[]);
            return;
        }
        if kind == K::AssertConstraintUsage && model.flag(elem, "isNegated") {
            words.push("assert not constraint".to_string());
        } else if let Some(portion) = model
            .maybe(elem, "portionKind")
            .and_then(Value::as_str)
            .filter(|_| model.flag(elem, "isPortion"))
        {
            // `timeslice t;` is an occurrence of no other word, and
            // `snapshot part p;` a part
            words.push(portion.to_string());
            if kind != K::OccurrenceUsage {
                words.push(keyword.to_string());
            }
        } else if kind == K::ReferenceUsage && (direction.is_some() || parameter) {
            // `in x : T;` -- a parameter is a reference, and the word
            // that would say so is not written
        } else if kind == K::ReferenceUsage && self.declared(elem).is_none() && self.redefines(elem)
        {
            // `:>> m = 2;` is a reference of no keyword
        } else if (!parameter || kind.is_a(K::Definition) || kind != K::ReferenceUsage)
            && !keyword.is_empty()
        {
            // a role written alone stands for a reference: `subject v :
            // V;`, but `return attribute r : R;` says what it is
            words.push(keyword.to_string());
        }
        if let Some(short) = model
            .maybe(elem, "declaredShortName")
            .and_then(Value::as_str)
        {
            words.push(format!("<{}>", sysml_syntax::quote(short)));
        }
        // `class all C;` -- a type whose every instance is one; SysML writes
        // no such word, and a definition it implies one of says nothing
        if model.flag(elem, "isSufficient") && kerml {
            words.push("all".to_string());
        }
        if let Some(name) = self.declared(elem) {
            words.push(sysml_syntax::quote(name));
        }
        // `entry;` -- an entry action that is nothing more
        if matches!(role, Some(Role::Entry | Role::Do | Role::Exit))
            && kind == K::ActionUsage
            && self.declared(elem).is_none()
            && self.body_of(elem, &[]).is_empty()
        {
            let mut header = String::new();
            self.header(elem, &mut header, false);
            if header.is_empty() {
                let _ = writeln!(self.out, "{indent}{prefix}{said};");
                return;
            }
        }
        let mut line = words.join(" ");
        self.header(elem, &mut line, false);
        // `message m of T;` -- a flow with no ends to name still says what
        // it carries
        if let Some(&carried) = model
            .owned(elem)
            .iter()
            .find(|&&it| model.kind(it) == K::PayloadFeature)
        {
            if let Some(payload) = self.payload(carried) {
                let _ = write!(line, " of {payload}");
                prefixed.push(carried);
            }
        }
        let line = format!("{indent}{}", line.trim_start());
        // `state s parallel { ... }` -- the word stands before the body,
        // which it then has to have
        if model.flag(elem, "isParallel") {
            let _ = writeln!(self.out, "{line} parallel {{");
            self.members(elem, depth + 1, &prefixed);
            let _ = writeln!(self.out, "{indent}}}");
            return;
        }
        self.finish(line, elem, depth, &prefixed);
    }

    /// End a declaration: with `;` where nothing more is said of it, and
    /// otherwise with its members in braces.
    fn finish(&mut self, line: String, elem: ElementId, depth: usize, skip: &[ElementId]) {
        let indent = "    ".repeat(depth);
        if self.body_of(elem, skip).is_empty() {
            let _ = writeln!(self.out, "{};", line.trim_end());
            return;
        }
        let _ = writeln!(self.out, "{} {{", line.trim_end());
        self.members(elem, depth + 1, skip);
        let _ = writeln!(self.out, "{indent}}}");
    }

    /// The name an element declares, as the text would write it.
    fn declared(&self, elem: ElementId) -> Option<&str> {
        match self.unnamed.contains(&elem) {
            true => None,
            false => self.model.name(elem),
        }
    }

    /// Visibility, as the words in front of a member.
    fn prefix(&self, elem: ElementId) -> String {
        match self.visibility(elem) {
            "" => String::new(),
            said => format!("{said} "),
        }
    }

    /// Whether a usage of a kind that is composite unless it says otherwise
    /// is held by reference: `ref part driver : Person;`. The defaults are
    /// the ones a model is built with: a port is a reference but within a
    /// port, an attribute and an event always are, and nothing within a
    /// value is part of it.
    fn referential(&self, elem: ElementId) -> bool {
        let model = self.model;
        let kind = model.kind(elem);
        if !kind.is_a(K::Usage)
            || model.flag(elem, "isComposite")
            || model.direction(elem).is_some()
            || model.member_role(elem).is_some()
            || kind.is_a(K::ConnectorAsUsage)
        {
            return false;
        }
        let Some(owner) = model.owner(elem) else {
            return false;
        };
        let owning = model.kind(owner);
        // a usage at the top of a package has nothing to be part of
        if !owning.is_a(K::Type) {
            return false;
        }
        composite_by_default(kind, owning)
    }

    /// Whether `elem` is a literal of an enumeration.
    fn in_enumeration(&self, elem: ElementId) -> bool {
        self.model.kind(elem) == K::EnumerationUsage
            && self
                .model
                .owner(elem)
                .is_some_and(|owner| self.model.kind(owner) == K::EnumerationDefinition)
    }

    /// Whether a declaration redefines something it says.
    fn redefines(&self, elem: ElementId) -> bool {
        self.model.owned(elem).iter().any(|&rel| {
            self.model.kind(rel) == K::Redefinition && !self.model.flag(rel, "isImplied")
        })
    }

    /// What a declaration that says nothing but one reference refers to:
    /// `perform takePicture;` and the like.
    fn only_referenced(&self, elem: ElementId) -> Option<String> {
        let model = self.model;
        let mut found = None;
        for &rel in model.owned(elem) {
            if model.flag(rel, "isImplied") || model.kind(rel) != K::ReferenceSubsetting {
                continue;
            }
            if found.is_some() {
                return None;
            }
            found = match model.maybe(rel, "referencedFeature") {
                Some(Value::Ref(target)) => Some(self.feature_ref(*target)?),
                _ => None,
            };
        }
        found
    }

    /// What a declaration says between its name and its body: `: T`,
    /// `:> S`, `:>> R`, `::> F`, `[m]` and `= v`, in the order the
    /// notation puts them. `referenced_said` leaves out the `::>` a
    /// shorthand has said already.
    fn header(&mut self, elem: ElementId, line: &mut String, referenced_said: bool) {
        let model = self.model;
        let mut typed = Vec::new();
        let mut specialized = Vec::new();
        let mut redefined = Vec::new();
        let mut referenced = Vec::new();
        let mut conjugates = None;
        let mut crossed = Vec::new();
        // KerML's type and feature relationships, each written after the
        // specializations with the words it is written with
        let mut related: Vec<(&str, String)> = Vec::new();
        let mut multiplicity = None;
        let mut value = None;
        for &rel in model.owned(elem) {
            if model.flag(rel, "isImplied") {
                continue;
            }
            let target = |prop: &str| match model.maybe(rel, prop) {
                Some(Value::Ref(target)) => Some(*target),
                _ => None,
            };
            match model.kind(rel) {
                K::FeatureTyping | K::ConjugatedPortTyping => {
                    if let Some(name) = target("type").and_then(|t| self.type_name(t)) {
                        typed.push(name);
                    }
                }
                K::Subclassification => {
                    if let Some(name) = target("superclassifier").and_then(|t| self.name_of(t)) {
                        specialized.push(name);
                    }
                }
                K::Specialization => {
                    if let Some(name) = target("general").and_then(|t| self.name_of(t)) {
                        specialized.push(name);
                    }
                }
                K::Conjugation => {
                    conjugates = target("originalType").and_then(|t| self.name_of(t));
                }
                K::CrossSubsetting => {
                    if let Some(name) = target("crossedFeature").and_then(|t| self.feature_ref(t)) {
                        crossed.push(name);
                    }
                }
                K::Unioning
                | K::Intersecting
                | K::Differencing
                | K::FeatureInverting
                | K::TypeFeaturing
                | K::Disjoining => {
                    let (word, prop) = match model.kind(rel) {
                        K::Unioning => ("unions", "unioningType"),
                        K::Intersecting => ("intersects", "intersectingType"),
                        K::Differencing => ("differences", "differencingType"),
                        K::FeatureInverting => ("inverse of", "invertingFeature"),
                        K::TypeFeaturing => ("featured by", "featuringType"),
                        _ => ("disjoint from", "disjoiningType"),
                    };
                    // a disjoining in a namespace is a statement of its own
                    if model.kind(rel) == K::Disjoining && target("typeDisjoined") != Some(elem) {
                        continue;
                    }
                    if let Some(name) = target(prop).and_then(|t| self.feature_ref(t)) {
                        related.push((word, name));
                    }
                }
                K::Subsetting => {
                    if let Some(name) = target("subsettedFeature").and_then(|t| self.feature_ref(t))
                    {
                        specialized.push(name);
                    }
                }
                K::Redefinition => {
                    if let Some(name) = target("redefinedFeature").and_then(|t| self.feature_ref(t))
                    {
                        redefined.push(name);
                    }
                }
                K::ReferenceSubsetting if !referenced_said => {
                    if let Some(name) =
                        target("referencedFeature").and_then(|t| self.feature_ref(t))
                    {
                        referenced.push(name);
                    }
                }
                K::MultiplicityRange => multiplicity = self.multiplicity(rel),
                K::FeatureValue => {
                    let said = target("value").and_then(|v| self.expr(v));
                    match said {
                        Some(said) => {
                            let how = match (
                                model.flag(rel, "isDefault"),
                                model.flag(rel, "isInitial"),
                            ) {
                                (true, true) => "default :=",
                                (true, false) => "default =",
                                (false, true) => ":=",
                                (false, false) => "=",
                            };
                            value = Some(format!("{how} {said}"));
                        }
                        None => self
                            .unwritten
                            .push(("expression".to_string(), self.where_is(elem))),
                    }
                }
                _ => {}
            }
        }
        if let Some(original) = conjugates {
            let _ = write!(line, " ~ {original}");
        }
        // a declaration of no name begins with what it specializes rather
        // than its type: `:>> num : Real`, since `: Real` begins nothing
        let typing = match typed.is_empty() {
            true => String::new(),
            false => format!(" : {}", typed.join(", ")),
        };
        let unnamed = line.trim().is_empty()
            || !(redefined.is_empty() && specialized.is_empty() && referenced.is_empty())
                && self.declared(elem).is_none()
                && model.maybe(elem, "declaredShortName").is_none();
        if !unnamed {
            line.push_str(&typing);
        }
        if !specialized.is_empty() {
            let _ = write!(line, " :> {}", specialized.join(", "));
        }
        if !redefined.is_empty() {
            let _ = write!(line, " :>> {}", redefined.join(", "));
        }
        if !referenced.is_empty() {
            let _ = write!(line, " ::> {}", referenced.join(", "));
        }
        if unnamed {
            line.push_str(&typing);
        }
        if let Some(multiplicity) = multiplicity {
            let _ = write!(line, " {multiplicity}");
        }
        if model.flag(elem, "isOrdered") {
            line.push_str(" ordered");
        }
        if model.maybe(elem, "isUnique") == Some(&Value::Bool(false)) {
            line.push_str(" nonunique");
        }
        if !crossed.is_empty() {
            let _ = write!(line, " crosses {}", crossed.join(", "));
        }
        for word in [
            "unions",
            "intersects",
            "differences",
            "disjoint from",
            "featured by",
            "inverse of",
        ] {
            let named: Vec<&str> = related
                .iter()
                .filter(|(said, _)| *said == word)
                .map(|(_, name)| name.as_str())
                .collect();
            if !named.is_empty() {
                let _ = write!(line, " {word} {}", named.join(", "));
            }
        }
        if let Some(value) = value {
            let _ = write!(line, " {value}");
        }
    }

    /// The declaration of a connector by the ends it connects, or `false`
    /// where it has no ends to say -- a connection definition's usage
    /// written as a plain declaration.
    fn connector(&mut self, elem: ElementId, depth: usize, connects: &Connects) -> bool {
        let model = self.model;
        let kind = model.kind(elem);
        let ends: Vec<ElementId> = model
            .owned(elem)
            .iter()
            .copied()
            .filter(|&it| model.flag(it, "isEnd") && !model.flag(it, "isImplied"))
            .collect();
        if ends.is_empty() || model.kind(elem).is_a(K::Definition) {
            return false;
        }
        // an end that says more than what it reaches -- `end :>> end1 ::>
        // d1;` -- is declared in the body, and so is every end of one
        // with more than two that names any
        let named = ends.iter().any(|&end| model.name(end).is_some());
        if (named && ends.len() > 2) || ends.iter().any(|&end| self.declares_more(end)) {
            return false;
        }
        let Some(said) = ends
            .iter()
            .map(|&end| self.end(end))
            .collect::<Option<Vec<String>>>()
        else {
            return false;
        };
        let indent = "    ".repeat(depth);
        let mut skip = ends.clone();
        // the payload of a flow is said as `of T`
        let mut payload = None;
        if let Some(&carried) = model
            .owned(elem)
            .iter()
            .find(|&&it| model.kind(it) == K::PayloadFeature)
        {
            skip.push(carried);
            payload = self.payload(carried);
        }
        let message = kind == K::FlowUsage && model.flag(elem, "isAbstract");
        let mut line = format!("{indent}{}", self.prefix(elem));
        if model.flag(elem, "isAbstract") && !message {
            line.push_str("abstract ");
        }
        let mut head = String::new();
        if let Some(short) = model
            .maybe(elem, "declaredShortName")
            .and_then(Value::as_str)
        {
            let _ = write!(head, " <{}>", sysml_syntax::quote(short));
        }
        if let Some(name) = model.name(elem) {
            let _ = write!(head, " {}", sysml_syntax::quote(name));
        }
        self.header(elem, &mut head, false);
        if kerml_only(kind) {
            self.kerml = true;
        }
        let clause = match said.as_slice() {
            [one, other] => format!("{} {one} {} {other}", connects.lead, connects.between),
            many => format!("{} ({})", connects.lead, many.join(", ")),
        };
        let clause = clause.trim_start().to_string();
        let clause = match &payload {
            Some(payload) => format!("of {payload} {clause}"),
            None => clause,
        };
        let keyword = match message {
            true => "message",
            false => connects.keyword,
        };
        // `connect a to b;` says all there is of a connector of no name,
        // type or payload; anything more is said in its declaration
        if kerml_only(kind) {
            // KerML writes the keyword always, and the word before the
            // ends only after a declaration: `connector a to b;` but
            // `connector c : T from a to b;`, `binding x of a = b;`
            let ends = match said.as_slice() {
                [one, other] => format!("{one} {} {other}", connects.between),
                many => format!("({})", many.join(", ")),
            };
            let declared = !head.trim().is_empty() || payload.is_some();
            let lead = match (kind, declared, said.len()) {
                (K::Succession, _, 2) => "first ",
                (K::Flow | K::SuccessionFlow, _, 2) => "from ",
                (K::Connector, true, 2) => "from ",
                (K::BindingConnector, true, _) => "of ",
                _ => "",
            };
            let carried = payload
                .as_ref()
                .map(|it| format!(" of {it}"))
                .unwrap_or_default();
            let _ = write!(line, "{keyword}{head}{carried} {lead}{ends}");
        } else if head.trim().is_empty() && payload.is_none() && connects.alone && !message {
            line.push_str(&clause);
        } else {
            let _ = write!(line, "{keyword}{head} {clause}");
        }
        self.finish(line, elem, depth, &skip);
        true
    }

    /// The cross feature an end owns, and how it is written in front of
    /// it: `[1]`, or `inCart [0..1]`.
    fn cross_multiplicity(&mut self, end: ElementId) -> Option<(ElementId, String)> {
        let model = self.model;
        let cross = sysml_model::owned_cross_feature(model, end)
            .filter(|&it| !model.flag(it, "isImplied") && self.chain(it).is_empty())?;
        let range = model
            .owned(cross)
            .iter()
            .copied()
            .find(|&it| model.kind(it) == K::MultiplicityRange)?;
        let multiplicity = self.multiplicity(range)?;
        Some(match model.name(cross) {
            Some(name) => (
                cross,
                format!("{} {multiplicity}", sysml_syntax::quote(name)),
            ),
            None => (cross, multiplicity),
        })
    }

    /// Whether a connector end says more than the clause can: what it
    /// redefines, subsets or is typed by, or members of its own.
    fn declares_more(&self, end: ElementId) -> bool {
        let model = self.model;
        model.owned(end).iter().any(|&child| {
            !model.flag(child, "isImplied")
                && (matches!(
                    model.kind(child),
                    K::Redefinition | K::Subsetting | K::FeatureTyping | K::FeatureValue
                ) || (!self.said_in_the_header(child)
                    && !matches!(model.kind(child), K::Feature | K::ReferenceUsage)))
        })
    }

    /// One end of a connector, as the clause names it: `a`, `a.b.c`,
    /// `[0..*] g ::> p2`.
    fn end(&mut self, end: ElementId) -> Option<String> {
        let model = self.model;
        let mut said = String::new();
        let mut target = None;
        let mut nested = None;
        // the cross multiplicity written in front of an end
        let cross = self.cross_multiplicity(end);
        if let Some((_, multiplicity)) = &cross {
            let _ = write!(said, "{multiplicity} ");
        }
        for &child in model.owned(end) {
            if model.flag(child, "isImplied") || cross.as_ref().is_some_and(|(it, _)| *it == child)
            {
                continue;
            }
            match model.kind(child) {
                K::ReferenceSubsetting => {
                    target = match model.maybe(child, "referencedFeature") {
                        Some(Value::Ref(target)) => Some(*target),
                        _ => None,
                    };
                }
                // what a flow end reaches in what it names: `from a.x`
                K::ReferenceUsage => {
                    nested = model.owned(child).iter().find_map(|&rel| {
                        match (model.kind(rel), model.maybe(rel, "redefinedFeature")) {
                            (K::Redefinition, Some(Value::Ref(feature))) => Some(*feature),
                            _ => None,
                        }
                    });
                    nested?;
                }
                K::Feature
                | K::MultiplicityRange
                | K::FeatureChaining
                | K::Redefinition
                | K::Subsetting
                | K::FeatureTyping => {}
                _ => return None,
            }
        }
        if let Some(name) = model.name(end) {
            let _ = write!(said, "{} ::> ", sysml_syntax::quote(name));
        }
        let named = match (target, nested) {
            (Some(target), Some(nested)) => format!(
                "{}.{}",
                self.feature_ref(target)?,
                sysml_syntax::quote(&self.simple_name(nested)?)
            ),
            (Some(target), None) => self.feature_ref(target)?,
            // `from a.b.x` -- the end is the chain to what holds `x`
            (None, Some(nested)) if !self.chain(end).is_empty() => format!(
                "{}.{}",
                self.chained(end)?,
                sysml_syntax::quote(&self.simple_name(nested)?)
            ),
            (None, Some(nested)) => self.feature_ref(nested)?,
            (None, None) => self.chained(end)?,
        };
        said.push_str(&named);
        Some(said)
    }

    /// The payload a flow carries: `T`, `p : T`, or what its value is.
    fn payload(&mut self, carried: ElementId) -> Option<String> {
        let model = self.model;
        let mut line = String::new();
        if let Some(name) = model.name(carried) {
            line.push_str(&sysml_syntax::quote(name));
        }
        self.header(carried, &mut line, false);
        let line = line.trim();
        // `of T` and `of p : T`, but `: T` alone is no payload
        let line = line.strip_prefix(": ").unwrap_or(line);
        (!line.is_empty()).then(|| line.to_string())
    }

    /// What a comment or metadata says it is about, where that is not what
    /// it is in: the `Annotation`s it owns, or the elements it names.
    fn about(&self, elem: ElementId) -> Vec<ElementId> {
        let model = self.model;
        let mut about: Vec<ElementId> = model
            .owned(elem)
            .iter()
            .filter(|&&it| model.kind(it) == K::Annotation && !model.flag(it, "isImplied"))
            .filter_map(|&it| match model.maybe(it, "annotatedElement") {
                Some(Value::Ref(target)) => Some(*target),
                _ => None,
            })
            .collect();
        if about.is_empty() {
            about = match model.maybe(elem, "annotatedElement") {
                Some(Value::RefList(all)) => all.clone(),
                Some(Value::Ref(one)) => vec![*one],
                _ => Vec::new(),
            };
        }
        let owner = model.owner(elem);
        match about.as_slice() {
            [one] if Some(*one) == owner => Vec::new(),
            _ => about,
        }
    }

    /// What a succession written `then b;` runs from: the nearest member of
    /// the same body before it that one can follow -- a step, an
    /// occurrence, what a `first` names, or where the connector before it
    /// went -- passing over the successions and transitions between, as
    /// the notation reads it.
    fn step_before(&self, succession: ElementId) -> Option<ElementId> {
        let model = self.model;
        let owner = model.owner(succession)?;
        let members = model.owned(owner);
        let at = members.iter().position(|&it| it == succession)?;
        for &member in members[..at].iter().rev() {
            let kind = model.kind(member);
            if model.flag(member, "isImplied")
                || kind.is_a(K::SuccessionAsUsage)
                || kind.is_a(K::TransitionUsage)
            {
                continue;
            }
            if kind.is_a(K::ConnectorAsUsage) {
                if let Some(Value::RefList(related)) = model.maybe(member, "relatedFeature") {
                    return related.last().copied();
                }
                continue;
            }
            if kind.is_a(K::Membership) {
                if let Some(Value::Ref(named)) = model.maybe(member, "memberElement") {
                    return Some(*named);
                }
                continue;
            }
            if kind.is_a(K::Step) || kind.is_a(K::OccurrenceUsage) {
                return Some(member);
            }
        }
        None
    }

    /// The two things a succession relates, by the ends it owns, where
    /// each end is just the feature it reaches.
    fn ends_of(&self, succession: ElementId) -> Option<(ElementId, ElementId)> {
        let model = self.model;
        let ends: Vec<ElementId> = model
            .owned(succession)
            .iter()
            .copied()
            .filter(|&it| model.flag(it, "isEnd") && !model.flag(it, "isImplied"))
            .collect();
        let reached = |end: ElementId| -> Option<ElementId> {
            if model.name(end).is_some() {
                return None;
            }
            let mut found = None;
            for &child in model.owned(end) {
                if model.flag(child, "isImplied") {
                    continue;
                }
                match (model.kind(child), model.maybe(child, "referencedFeature")) {
                    (K::ReferenceSubsetting, Some(Value::Ref(target))) => found = Some(*target),
                    (K::MultiplicityRange, _) => {}
                    _ => return None,
                }
            }
            found
        };
        match ends.as_slice() {
            [source, target] => Some((reached(*source)?, reached(*target)?)),
            _ => None,
        }
    }

    /// Whether a declaration says nothing of its own: no name, no type, no
    /// multiplicity, nothing in its body but `skip`.
    fn says_nothing(&mut self, elem: ElementId, skip: &[ElementId]) -> bool {
        let model = self.model;
        if model.name(elem).is_some()
            || model.maybe(elem, "declaredShortName").is_some()
            || !self.body_of(elem, skip).is_empty()
        {
            return false;
        }
        let mut header = String::new();
        let before = self.unwritten.len();
        self.header(elem, &mut header, false);
        self.unwritten.truncate(before);
        header.is_empty()
    }

    /// `then b;` for a succession from the step before it, and `then
    /// action b;` where what it runs to is declared right after it.
    fn then(&mut self, elem: ElementId, depth: usize) -> bool {
        let model = self.model;
        let Some((source, target)) = self.ends_of(elem) else {
            return false;
        };
        let ends: Vec<ElementId> = model
            .owned(elem)
            .iter()
            .copied()
            .filter(|&it| model.flag(it, "isEnd"))
            .collect();
        if self.step_before(elem) != Some(source) || !self.says_nothing(elem, &ends) {
            return false;
        }
        let indent = "    ".repeat(depth);
        let prefix = self.prefix(elem);
        // what it runs to, declared next, is declared in it
        let next = model.owner(elem).and_then(|owner| {
            let members = model.owned(owner);
            let at = members.iter().position(|&it| it == elem)?;
            members[at + 1..]
                .iter()
                .copied()
                .find(|&it| !self.outside.contains_key(&it) && !self.said_in_the_header(it))
        });
        // an action declared in a `then` makes a succession that is
        // composite, as one that names what it runs to does, and anything
        // else declared in one a succession that is not: the model says
        // which was written
        let declares = next == Some(target)
            && model.kind(target).is_a(K::OccurrenceUsage)
            && !model.kind(target).is_a(K::ConnectorAsUsage)
            && model.member_role(target).is_none()
            && ((model.kind(target) == K::ActionUsage) == model.flag(elem, "isComposite")
                // one with no name of its own cannot be named after it
                || model.name(target).is_none());
        if declares {
            let saved = std::mem::take(&mut self.out);
            self.member(target, depth);
            let written = std::mem::replace(&mut self.out, saved);
            let _ = write!(self.out, "{indent}{prefix}then {}", written.trim_start());
            return true;
        }
        let Some(named) = self.feature_ref(target) else {
            return false;
        };
        let _ = writeln!(self.out, "{indent}{prefix}then {named};");
        true
    }

    /// A transition from the step before it, written as the notation
    /// writes one there: `if g then b;` and `else c;` after a decision,
    /// `accept s then b;` after a state.
    fn transition_after(&mut self, elem: ElementId, depth: usize) -> bool {
        let model = self.model;
        if model.name(elem).is_some() || model.maybe(elem, "declaredShortName").is_some() {
            return false;
        }
        let owned = model.owned(elem).to_vec();
        let Some(&succession) = owned
            .iter()
            .find(|&&it| model.kind(it) == K::SuccessionAsUsage)
        else {
            return false;
        };
        let Some((source, target)) = self.ends_of(succession) else {
            return false;
        };
        if self.step_before(elem) != Some(source) {
            return false;
        }
        let refs = |prop: &str| -> Vec<ElementId> {
            match model.maybe(elem, prop) {
                Some(Value::RefList(all)) => all.clone(),
                Some(Value::Ref(one)) => vec![*one],
                _ => Vec::new(),
            }
        };
        let (triggers, guards, effects) = (
            refs("triggerAction"),
            refs("guardExpression"),
            refs("effectAction"),
        );
        let skip: Vec<ElementId> = owned
            .iter()
            .copied()
            .filter(|&it| {
                it == succession
                    || triggers.contains(&it)
                    || guards.contains(&it)
                    || effects.contains(&it)
                    || model.direction(it).is_some()
            })
            .collect();
        if !self.says_nothing(elem, &skip) {
            return false;
        }
        let in_state = model.owner(elem).is_some_and(|owner| {
            matches!(
                model.kind(owner),
                K::StateDefinition | K::StateUsage | K::ExhibitStateUsage
            )
        });
        let Some(target) = self.feature_ref(target) else {
            return false;
        };
        let mut said = Vec::new();
        if let Some(&trigger) = triggers.first() {
            match self.trigger(trigger).filter(|_| in_state) {
                Some(trigger) => said.push(format!("accept {trigger}")),
                None => return false,
            }
        }
        if let Some(&guard) = guards.first() {
            match self.expr(guard) {
                Some(guard) => said.push(format!("if {guard}")),
                None => return false,
            }
        }
        if let Some(&effect) = effects.first() {
            match self.inline(effect).filter(|_| in_state) {
                Some(effect) => said.push(format!("do {effect}")),
                None => return false,
            }
        }
        let indent = "    ".repeat(depth);
        let prefix = self.prefix(elem);
        match (said.is_empty(), in_state) {
            // in an action a transition with no guard is the `else` of a
            // decision; in a state, one with nothing to say is a succession
            (true, false) => {
                let _ = writeln!(self.out, "{indent}{prefix}else {target};");
            }
            (true, true) => return false,
            (false, _) => {
                let _ = writeln!(
                    self.out,
                    "{indent}{prefix}{} then {target};",
                    said.join(" ")
                );
            }
        }
        true
    }

    /// `transition t first a accept s : Sig if g do action e then b;`, or
    /// in an action, `succession first a if g then b;`.
    fn transition(&mut self, elem: ElementId, depth: usize) -> bool {
        let model = self.model;
        let indent = "    ".repeat(depth);
        let owned = model.owned(elem).to_vec();
        let Some(&succession) = owned
            .iter()
            .find(|&&it| model.kind(it) == K::SuccessionAsUsage)
        else {
            return false;
        };
        let ends: Vec<ElementId> = model
            .owned(succession)
            .iter()
            .copied()
            .filter(|&it| model.flag(it, "isEnd"))
            .collect();
        let [source, target] = ends.as_slice() else {
            return false;
        };
        let (Some(source), Some(target)) = (self.end(*source), self.end(*target)) else {
            return false;
        };
        let refs = |prop: &str| -> Vec<ElementId> {
            match model.maybe(elem, prop) {
                Some(Value::RefList(all)) => all.clone(),
                Some(Value::Ref(one)) => vec![*one],
                _ => Vec::new(),
            }
        };
        let triggers = refs("triggerAction");
        let guards = refs("guardExpression");
        let effects = refs("effectAction");
        let in_state = model.owner(elem).is_some_and(|owner| {
            matches!(
                model.kind(owner),
                K::StateDefinition | K::StateUsage | K::ExhibitStateUsage
            )
        });
        let mut line = format!("{indent}{}", self.prefix(elem));
        line.push_str(if in_state { "transition" } else { "succession" });
        if let Some(name) = model.name(elem) {
            let _ = write!(line, " {}", sysml_syntax::quote(name));
        }
        let _ = write!(line, " first {source}");
        if let Some(&trigger) = triggers.first() {
            let Some(said) = self.trigger(trigger).filter(|_| in_state) else {
                return false;
            };
            let _ = write!(line, " accept {said}");
        }
        if let Some(&guard) = guards.first() {
            let Some(said) = self.expr(guard) else {
                return false;
            };
            let _ = write!(line, " if {said}");
        }
        if let Some(&effect) = effects.first() {
            let Some(said) = self.inline(effect).filter(|_| in_state) else {
                return false;
            };
            let _ = write!(line, " do {said}");
        }
        let _ = write!(line, " then {target}");
        // the parameters a transition is given are no member it declares
        let skip: Vec<ElementId> = owned
            .iter()
            .copied()
            .filter(|&it| {
                it == succession
                    || triggers.contains(&it)
                    || guards.contains(&it)
                    || effects.contains(&it)
                    || model.direction(it).is_some()
            })
            .collect();
        self.finish(line, elem, depth, &skip);
        true
    }

    /// What a transition or an accept action waits for: `Sig`, `s : Sig`,
    /// `at t`, `after 5 [s]`, `when x > 1`, with `via p` where it names
    /// what it receives through.
    fn trigger(&mut self, accept: ElementId) -> Option<String> {
        let model = self.model;
        let parameters = self.parameters(accept);
        let payload = *parameters.first()?;
        let mut said = String::new();
        // `accept at t` -- the payload's value is the change or time it
        // waits for
        match self.value_of(payload) {
            Some(value) if model.kind(value) == K::TriggerInvocationExpression => {
                let how = model
                    .maybe(value, "kind")
                    .and_then(Value::as_str)
                    .unwrap_or("when");
                let argument = self.arguments(value).into_iter().next()?;
                let _ = write!(said, "{how} {}", self.expr(argument)?);
            }
            Some(_) => return None,
            None => {
                let mut line = String::new();
                if let Some(name) = model.name(payload) {
                    line.push_str(&sysml_syntax::quote(name));
                }
                self.header(payload, &mut line, false);
                let line = line.trim();
                // `accept Sig` types an unnamed payload
                said.push_str(line.strip_prefix(": ").unwrap_or(line));
                if said.is_empty() {
                    return None;
                }
            }
        }
        if let Some(&receiver) = parameters.get(1) {
            if let Some(value) = self.value_of(receiver) {
                let _ = write!(said, " via {}", self.expr(value)?);
            }
        }
        Some(said)
    }

    /// An action written in line, as the effect of a transition is: the
    /// declaration without its `;`.
    fn inline(&mut self, elem: ElementId) -> Option<String> {
        if self.model.name(elem) == Some("effect") {
            self.unnamed.insert(elem);
        }
        let saved = std::mem::take(&mut self.out);
        let before = self.unwritten.len();
        self.member(elem, 0);
        let written = std::mem::replace(&mut self.out, saved);
        if self.unwritten.len() > before || written.contains("// not written") {
            self.unwritten.truncate(before);
            return None;
        }
        let written = written.trim_end();
        Some(written.strip_suffix(';').unwrap_or(written).to_string())
    }

    /// The words an action statement starts with: `action a ` where it is
    /// named, nothing where it is not.
    fn action_head(&self, elem: ElementId) -> String {
        let model = self.model;
        let mut head = self.prefix(elem);
        // `entry assign x := 1;` -- the role a state's action plays
        match model.member_role(elem) {
            Some(Role::Entry) => head.push_str("entry "),
            Some(Role::Do) => head.push_str("do "),
            Some(Role::Exit) => head.push_str("exit "),
            _ => {}
        }
        if let Some(name) = self.declared(elem) {
            let _ = write!(head, "action {} ", sysml_syntax::quote(name));
        }
        head
    }

    /// The parameters an action statement is handed, in order.
    fn parameters(&self, elem: ElementId) -> Vec<ElementId> {
        let model = self.model;
        model
            .owned(elem)
            .iter()
            .copied()
            .filter(|&it| model.direction(it).is_some() && !model.flag(it, "isImplied"))
            .collect()
    }

    /// `accept s : Sig via p;`
    fn accept(&mut self, elem: ElementId, depth: usize) -> bool {
        let indent = "    ".repeat(depth);
        let Some(said) = self.trigger(elem) else {
            return false;
        };
        let line = format!("{indent}{}accept {said}", self.action_head(elem));
        // the payload and what it is received through; any other
        // parameter is declared in its body
        let skip: Vec<ElementId> = self.parameters(elem).into_iter().take(2).collect();
        self.finish(line, elem, depth, &skip);
        true
    }

    /// `send x via p to q;`
    fn send(&mut self, elem: ElementId, depth: usize) -> bool {
        let indent = "    ".repeat(depth);
        // a parameter that redefines one is declared in the body: `send {
        // in :>> payload = s; }`
        let parameters: Vec<ElementId> = self
            .parameters(elem)
            .into_iter()
            .take_while(|&it| !self.redefines(it) && self.model.name(it).is_none())
            .collect();
        let mut said = Vec::new();
        for (at, &parameter) in parameters.iter().enumerate() {
            let Some(value) = self.value_of(parameter) else {
                continue;
            };
            let Some(written) = self.expr(value) else {
                return false;
            };
            said.push(match at {
                0 => written,
                1 => format!("via {written}"),
                _ => format!("to {written}"),
            });
        }
        if said.is_empty() && self.body_of(elem, &parameters).is_empty() {
            return false;
        }
        let line = format!("{indent}{}send {}", self.action_head(elem), said.join(" "));
        let skip: Vec<ElementId> = parameters.into_iter().take(3).collect();
        self.finish(line, elem, depth, &skip);
        true
    }

    /// `assign x := v;`
    fn assign(&mut self, elem: ElementId, depth: usize) -> bool {
        let model = self.model;
        let indent = "    ".repeat(depth);
        let parameters = self.parameters(elem);
        let assigned = model
            .owned(elem)
            .iter()
            .copied()
            .find(|&it| model.kind(it) == K::Membership && !model.flag(it, "isImplied"));
        let Some(assigned) = assigned else {
            return false;
        };
        let Some(Value::Ref(feature)) = model.maybe(assigned, "memberElement") else {
            return false;
        };
        let feature = *feature;
        // `assign a.b := v` reaches `b` through what the first parameter
        // comes to
        let target = match parameters.first().and_then(|&it| self.value_of(it)) {
            Some(through) => match (self.expr(through), self.simple_name(feature)) {
                (Some(through), Some(name)) => format!("{through}.{}", sysml_syntax::quote(&name)),
                _ => return false,
            },
            None => match self.feature_ref(feature) {
                Some(named) => named,
                None => return false,
            },
        };
        let Some(value) = parameters
            .get(1)
            .and_then(|&it| self.value_of(it))
            .and_then(|it| self.expr(it))
        else {
            return false;
        };
        let line = format!(
            "{indent}{}assign {target} := {value}",
            self.action_head(elem)
        );
        let mut skip: Vec<ElementId> = parameters.into_iter().take(2).collect();
        skip.push(assigned);
        self.finish(line, elem, depth, &skip);
        true
    }

    /// What a clause of a statement says before its braces: `action
    /// charging ` where the action it is has a name or a type of its own.
    fn clause_head(&mut self, clause: ElementId) -> String {
        let model = self.model;
        let mut head = String::new();
        if let Some(name) = model.name(clause) {
            head.push_str(&sysml_syntax::quote(name));
        }
        self.header(clause, &mut head, false);
        match head.trim() {
            "" => String::new(),
            said => format!("action {said} "),
        }
    }

    /// `if c { ... } else { ... }`, and `else if` for an if in an else.
    fn if_action(&mut self, elem: ElementId, depth: usize, chained: bool) -> bool {
        let model = self.model;
        let indent = "    ".repeat(depth);
        let parameters = self.parameters(elem);
        let [condition, then, rest @ ..] = parameters.as_slice() else {
            return false;
        };
        let Some(condition) = self.expr(*condition) else {
            return false;
        };
        let head = match chained {
            true => String::new(),
            false => format!("{indent}{}", self.action_head(elem)),
        };
        let named = self.clause_head(*then);
        let _ = writeln!(self.out, "{head}if {condition} {named}{{");
        self.members(*then, depth + 1, &[]);
        match rest.first() {
            Some(&otherwise) if model.kind(otherwise) == K::IfActionUsage => {
                let _ = write!(self.out, "{indent}}} else ");
                return self.if_action(otherwise, depth, true);
            }
            Some(&otherwise) => {
                let named = self.clause_head(otherwise);
                let _ = writeln!(self.out, "{indent}}} else {named}{{");
                self.members(otherwise, depth + 1, &[]);
            }
            None => {}
        }
        let _ = writeln!(self.out, "{indent}}}");
        true
    }

    /// `while c { ... } until d;`, and `loop { ... }` where there is no
    /// condition to hold to.
    fn while_loop(&mut self, elem: ElementId, depth: usize) -> bool {
        let model = self.model;
        let indent = "    ".repeat(depth);
        let parameters = self.parameters(elem);
        let mut condition = None;
        let mut body = None;
        let mut until = None;
        for &parameter in &parameters {
            let kind = model.kind(parameter);
            match (kind.is_a(K::Expression), kind.is_a(K::ActionUsage), body) {
                (true, _, None) => condition = Some(parameter),
                // `loop` holds to nothing: the condition it is handed is
                // a parameter with no value
                (false, false, None) => {
                    if let Some(value) = self.value_of(parameter) {
                        condition = Some(value);
                    }
                }
                (false, true, None) => body = Some(parameter),
                (true, _, Some(_)) => until = Some(parameter),
                (false, _, Some(_)) => return false,
            }
        }
        let Some(body) = body else {
            return false;
        };
        let head = format!("{indent}{}", self.action_head(elem));
        let named = self.clause_head(body);
        let opening = match condition.map(|it| self.expr(it)) {
            Some(Some(said)) => format!("{head}while {said} {named}{{"),
            Some(None) => return false,
            None => format!("{head}loop {named}{{"),
        };
        let until = match until.map(|it| self.expr(it)) {
            Some(None) => return false,
            Some(Some(said)) => Some(said),
            None => None,
        };
        let _ = writeln!(self.out, "{opening}");
        self.members(body, depth + 1, &[]);
        match until {
            Some(until) => {
                let _ = writeln!(self.out, "{indent}}} until {until};");
            }
            None => {
                let _ = writeln!(self.out, "{indent}}}");
            }
        }
        true
    }

    /// `for i : T in xs { ... }`
    fn for_loop(&mut self, elem: ElementId, depth: usize) -> bool {
        let model = self.model;
        let indent = "    ".repeat(depth);
        let variable = model.owned(elem).iter().copied().find(|&it| {
            model.kind(it) == K::ReferenceUsage
                && model.direction(it).is_none()
                && !model.flag(it, "isImplied")
        });
        let parameters = self.parameters(elem);
        let (Some(variable), [sequence, body]) = (variable, parameters.as_slice()) else {
            return false;
        };
        let Some(sequence) = self.expr(*sequence) else {
            return false;
        };
        let mut declared = model
            .name(variable)
            .map(sysml_syntax::quote)
            .unwrap_or_default();
        self.header(variable, &mut declared, false);
        let named = self.clause_head(*body);
        let _ = writeln!(
            self.out,
            "{indent}{}for {} in {sequence} {named}{{",
            self.action_head(elem),
            declared.trim()
        );
        self.members(*body, depth + 1, &[]);
        let _ = writeln!(self.out, "{indent}}}");
        true
    }

    /// `terminate;` and `terminate x;`
    fn terminate(&mut self, elem: ElementId, depth: usize) -> bool {
        let indent = "    ".repeat(depth);
        let parameters = self.parameters(elem);
        let what = match parameters.first().and_then(|&it| self.value_of(it)) {
            Some(value) => match self.expr(value) {
                Some(said) => format!(" {said}"),
                None => return false,
            },
            None => String::new(),
        };
        let line = format!("{indent}{}terminate{what}", self.action_head(elem));
        let skip: Vec<ElementId> = parameters.into_iter().take(1).collect();
        self.finish(line, elem, depth, &skip);
        true
    }

    /// `satisfy r by x;`, and `not satisfy` for one that is negated.
    fn satisfy(&mut self, elem: ElementId, depth: usize) -> bool {
        let model = self.model;
        let indent = "    ".repeat(depth);
        let target = |prop: &str| match model.maybe(elem, prop) {
            Some(Value::Ref(target)) => Some(*target),
            _ => None,
        };
        let mut line = format!("{indent}{}", self.prefix(elem));
        if model.flag(elem, "isNegated") {
            line.push_str("not ");
        }
        // the requirement satisfied is the one it refers to, or one it
        // declares: `satisfy requirement r : R by x;`
        match self.declared(elem) {
            Some(name) => {
                let _ = write!(line, "satisfy requirement {}", sysml_syntax::quote(name));
                self.header(elem, &mut line, false);
            }
            None => {
                let Some(requirement) =
                    target("satisfiedRequirement").and_then(|it| self.feature_ref(it))
                else {
                    return false;
                };
                let _ = write!(line, "satisfy {requirement}");
                let mut rest = String::new();
                self.header(elem, &mut rest, true);
                line.push_str(&rest);
            }
        }
        // the satisfying feature is the subject's value, written after
        // `by`; the subject itself is said by that
        let skip: Vec<ElementId> = model
            .owned(elem)
            .iter()
            .copied()
            .filter(|&child| model.member_role(child) == Some(Role::Subject))
            .collect();
        if let Some(by) = target("satisfyingFeature").and_then(|it| self.feature_ref(it)) {
            let _ = write!(line, " by {by}");
        }
        self.finish(line, elem, depth, &skip);
        true
    }

    /// `@M;`, `@M { x = 1; }`, `metadata m : M about a;`.
    fn metadata(&mut self, elem: ElementId, depth: usize) -> bool {
        let model = self.model;
        let indent = "    ".repeat(depth);
        let typed: Vec<String> = model
            .owned(elem)
            .iter()
            .filter(|&&rel| model.kind(rel) == K::FeatureTyping && !model.flag(rel, "isImplied"))
            .filter_map(|&rel| match model.maybe(rel, "type") {
                Some(Value::Ref(ty)) => self.name_of(*ty),
                _ => None,
            })
            .collect();
        let [ty] = typed.as_slice() else {
            return false;
        };
        let about: Vec<String> = match self
            .about(elem)
            .iter()
            .map(|&it| self.name_of(it))
            .collect()
        {
            Some(named) => named,
            None => return false,
        };
        let mut line = format!("{indent}{}", self.prefix(elem));
        match model.name(elem) {
            Some(name) => {
                let _ = write!(line, "metadata {} : {ty}", sysml_syntax::quote(name));
            }
            None if about.is_empty() => {
                let _ = write!(line, "@{ty}");
            }
            None => {
                let _ = write!(line, "metadata {ty}");
            }
        }
        if !about.is_empty() {
            let _ = write!(line, " about {}", about.join(", "));
        }
        self.finish(line, elem, depth, &[]);
        true
    }

    /// `dependency d from a to b, c;`
    fn dependency(&mut self, elem: ElementId, indent: &str) {
        let model = self.model;
        let names = |prop: &str| -> Option<Vec<String>> {
            match model.maybe(elem, prop) {
                Some(Value::RefList(all)) => all.iter().map(|&it| self.name_of(it)).collect(),
                Some(Value::Ref(one)) => Some(vec![self.name_of(*one)?]),
                _ => None,
            }
        };
        let (Some(clients), Some(suppliers)) = (names("client"), names("supplier")) else {
            self.leave_out(elem, indent);
            return;
        };
        let mut line = format!("{indent}{}dependency", self.prefix(elem));
        if let Some(name) = model.name(elem) {
            let _ = write!(line, " {} from", sysml_syntax::quote(name));
        }
        let line = format!("{line} {} to {}", clients.join(", "), suppliers.join(", "));
        self.finish(line, elem, indent.len() / 4, &[]);
    }

    /// `import P::*;`, `import P::x::**;`, `import P::**[@Safety];`, and
    /// the same of `expose`.
    fn import(&mut self, elem: ElementId, depth: usize, word: &str) {
        let model = self.model;
        let indent = &"    ".repeat(depth);
        // an exposure keeps the text it names what it exposes by, which is
        // exact where the model is not: `v::**` and `v::*::**` are one
        // import in it
        let kept = match word {
            "expose" => self.kept_text(elem),
            _ => None,
        };
        let filtered = self.filtered(elem);
        let target = match kept {
            Some(kept) => self.imported_as(elem, Some(kept)),
            None => self.imported_as(elem, None),
        };
        let Some(target) = target else {
            self.leave_out(elem, indent);
            return;
        };
        // an exposure is always private, and says nothing of it
        let visibility = match (word, model.member_visibility(elem)) {
            ("expose", _) => "",
            (_, Some(Vis::Public)) => "public ",
            (_, Some(Vis::Protected)) => "protected ",
            (_, Some(Vis::Private)) => "private ",
            (_, None) => "",
        };
        let every = match model.flag(elem, "isImportAll") && word == "import" {
            true => "all ",
            false => "",
        };
        let line = format!("{indent}{visibility}{word} {every}{target}");
        // what the import holds besides -- a `doc` -- is said in its body;
        // the text kept of it, and the package a filter makes, are not
        let skip: Vec<ElementId> = model
            .owned(elem)
            .iter()
            .copied()
            .filter(|&it| model.kind(it) == K::TextualRepresentation || Some(it) == filtered)
            .collect();
        self.finish(line, elem, depth, &skip);
    }

    /// The package of no name a filtered import imports from, where it is
    /// one: it holds the import written and the conditions in brackets.
    fn filtered(&self, import: ElementId) -> Option<ElementId> {
        let model = self.model;
        let target = member_target(model, import)?;
        (model.kind(target) == K::Package
            && model.name(target).is_none()
            && model.owner(target) == Some(import))
        .then_some(target)
    }

    /// What an import names, with the `::*`, `::**` and the filters in
    /// brackets it is written with.
    fn imported(&mut self, import: ElementId) -> Option<String> {
        self.imported_as(import, None)
    }

    /// [`imported`](Self::imported), with what is imported from written
    /// as `kept` where the document kept the text of it.
    fn imported_as(&mut self, import: ElementId, kept: Option<String>) -> Option<String> {
        let model = self.model;
        if let Some(package) = self.filtered(import) {
            let inner =
                model.owned(package).iter().copied().find(|&it| {
                    matches!(model.kind(it), K::NamespaceImport | K::MembershipImport)
                })?;
            let mut said = match kept {
                Some(kept) => kept,
                None => self.imported(inner)?,
            };
            for &member in model.owned(package) {
                if model.kind(member) != K::ElementFilterMembership {
                    continue;
                }
                let condition = match model.maybe(member, "condition") {
                    Some(Value::Ref(condition)) => Some(*condition),
                    _ => model.owned(member).first().copied(),
                }?;
                let _ = write!(said, "[{}]", self.expr(condition)?);
            }
            return Some(said);
        }
        if let Some(kept) = kept {
            return Some(kept);
        }
        self.importing = true;
        let target = member_target(model, import).and_then(|it| self.name_of(it));
        self.importing = false;
        let all = matches!(model.kind(import), K::NamespaceImport | K::NamespaceExpose);
        let tail = match (all, model.flag(import, "isRecursive")) {
            (true, true) => "::*::**",
            (true, false) => "::*",
            (false, true) => "::**",
            (false, false) => "",
        };
        Some(format!("{}{tail}", target?))
    }

    /// `[n]`, `[l..u]` or `[*]` from the bounds a multiplicity holds.
    fn multiplicity(&mut self, range: ElementId) -> Option<String> {
        let model = self.model;
        let bound = |prop: &str| match model.maybe(range, prop) {
            Some(Value::Ref(bound)) => Some(*bound),
            _ => None,
        };
        let mut bounds: Vec<ElementId> = match (bound("lowerBound"), bound("upperBound")) {
            (Some(lower), Some(upper)) if lower != upper => vec![lower, upper],
            (_, Some(upper)) => vec![upper],
            _ => Vec::new(),
        };
        if bounds.is_empty() {
            bounds = match model.maybe(range, "bound") {
                Some(Value::RefList(bounds)) if !bounds.is_empty() => bounds.clone(),
                Some(Value::Ref(bound)) => vec![*bound],
                _ => model
                    .owned(range)
                    .iter()
                    .copied()
                    .filter(|&b| model.kind(b).is_a(K::Expression))
                    .collect(),
            };
        }
        let written: Option<Vec<String>> = bounds.iter().map(|&b| self.expr(b)).collect();
        match written.as_deref() {
            Some([one]) => Some(format!("[{one}]")),
            Some([lower, upper]) => Some(format!("[{lower}..{upper}]")),
            _ => {
                self.unwritten
                    .push(("multiplicity".to_string(), self.where_is(range)));
                None
            }
        }
    }

    /// The value a feature is given, where it is given one.
    fn value_of(&self, feature: ElementId) -> Option<ElementId> {
        let model = self.model;
        model.owned(feature).iter().find_map(|&rel| {
            match (model.kind(rel), model.maybe(rel, "value")) {
                (K::FeatureValue, Some(Value::Ref(value))) => Some(*value),
                _ => None,
            }
        })
    }

    /// The text an expression was written as, where the document kept it.
    fn kept_text(&self, expr: ElementId) -> Option<String> {
        let model = self.model;
        model.owned(expr).iter().find_map(|&child| {
            if model.kind(child) != K::TextualRepresentation {
                return None;
            }
            let language = model.maybe(child, "language").and_then(Value::as_str)?;
            if !language.eq_ignore_ascii_case("sysml") && !language.eq_ignore_ascii_case("kerml") {
                return None;
            }
            let body = model.maybe(child, "body").and_then(Value::as_str)?.trim();
            (!body.is_empty()).then(|| body.to_string())
        })
    }

    /// An expression as the notation writes it.
    fn expr(&self, expr: ElementId) -> Option<String> {
        if !self.structural {
            if let Some(text) = self.kept_text(expr) {
                return Some(text);
            }
        }
        self.structure(expr)
    }

    /// The arguments an invocation is handed: each of its input
    /// parameters' values, in order.
    fn arguments(&self, expr: ElementId) -> Vec<ElementId> {
        let model = self.model;
        model
            .owned(expr)
            .iter()
            .copied()
            .filter(|&it| {
                model.kind(it).is_a(K::Feature)
                    && model.member_role(it) != Some(Role::Return)
                    && model.direction(it) == Some("in")
            })
            .filter_map(|it| self.value_of(it))
            .collect()
    }

    /// The elements an expression names rather than is handed: what it
    /// invokes, the type it classifies by, the feature a chain goes to.
    fn named_in(&self, expr: ElementId) -> Vec<ElementId> {
        let model = self.model;
        model
            .owned(expr)
            .iter()
            .filter(|&&it| model.kind(it) == K::Membership && !model.flag(it, "isImplied"))
            .filter_map(|&it| match model.maybe(it, "memberElement") {
                Some(Value::Ref(named)) => Some(*named),
                _ => None,
            })
            .collect()
    }

    /// An expression written from what it is, for a document that does
    /// not keep its text. Every operation is parenthesised: the notation's
    /// precedence is not something to get wrong, and parentheses add
    /// nothing to the model.
    fn structure(&self, expr: ElementId) -> Option<String> {
        let model = self.model;
        if let Some(literal) = self.literal(expr) {
            return Some(literal);
        }
        let arguments = || -> Option<Vec<String>> {
            self.arguments(expr)
                .into_iter()
                .map(|it| self.expr(it))
                .collect()
        };
        match model.kind(expr) {
            K::NullExpression => Some("null".to_string()),
            K::FeatureReferenceExpression => {
                let referent = match model.maybe(expr, "referent") {
                    Some(Value::Ref(referent)) => Some(*referent),
                    _ => self.named_in(expr).first().copied(),
                };
                // `accept when x > 1` refers to the expression it holds
                let Some(referent) = referent else {
                    let held = model.owned(expr).iter().copied().find(|&it| {
                        model.kind(it).is_a(K::Expression)
                            && model.member_role(it) != Some(Role::Return)
                    })?;
                    return self.expr(held);
                };
                // a parameter of the body an expression is in is named as
                // it is declared, and has no qualified name to be named by
                let local = model.owner(referent).is_some_and(|owner| {
                    model.kind(owner) == K::Expression && model.direction(referent).is_some()
                });
                match local {
                    true => model.name(referent).map(sysml_syntax::quote),
                    false => self.feature_ref(referent),
                }
            }
            K::InvocationExpression => {
                let invoked = self.named_in(expr).first().copied()?;
                let named = self.name_of(invoked)?;
                Some(format!("{named}({})", arguments()?.join(", ")))
            }
            K::ConstructorExpression => {
                let invoked = self.named_in(expr).first().copied()?;
                let named = self.name_of(invoked)?;
                let result = model
                    .owned(expr)
                    .iter()
                    .copied()
                    .find(|&it| model.member_role(it) == Some(Role::Return))?;
                let handed: Option<Vec<String>> = self
                    .arguments(result)
                    .into_iter()
                    .map(|it| self.expr(it))
                    .collect();
                Some(format!("new {named}({})", handed?.join(", ")))
            }
            K::MetadataAccessExpression => {
                let of = self.named_in(expr).first().copied()?;
                Some(format!("{}.metadata", self.name_of(of)?))
            }
            K::FeatureChainExpression => {
                let [from]: [String; 1] = arguments()?.try_into().ok()?;
                let to = self.named_in(expr).last().copied()?;
                Some(format!(
                    "({from}).{}",
                    sysml_syntax::quote(&self.simple_name(to)?)
                ))
            }
            K::SelectExpression | K::CollectExpression => {
                let [from, body]: [String; 2] = arguments()?.try_into().ok()?;
                let word = match model.kind(expr) {
                    K::SelectExpression => "select",
                    _ => "collect",
                };
                Some(format!("({from})->{word} {body}"))
            }
            K::IndexExpression => {
                let [from, index]: [String; 2] = arguments()?.try_into().ok()?;
                Some(format!("({from})#({index})"))
            }
            K::OperatorExpression => {
                let operator = model.maybe(expr, "operator").and_then(Value::as_str)?;
                let handed = arguments()?;
                let classifies = matches!(
                    operator,
                    "as" | "meta" | "istype" | "hastype" | "@" | "@@" | "all"
                );
                if classifies {
                    let ty = self.name_of(*self.named_in(expr).first()?)?;
                    return match handed.as_slice() {
                        [] => Some(format!("{operator}{}{ty}", spacer(operator))),
                        [of] => Some(format!("({of} {operator} {ty})")),
                        _ => None,
                    };
                }
                match (operator, handed.as_slice()) {
                    ("if", [condition, then, otherwise]) => {
                        Some(format!("(if {condition} ? {then} else {otherwise})"))
                    }
                    // `(a, b, c)` nests to the right, and is written flat
                    (",", [first, rest]) => {
                        let rest = rest
                            .strip_prefix('(')
                            .and_then(|it| it.strip_suffix(')'))
                            .filter(|it| it.contains(','))
                            .unwrap_or(rest);
                        Some(format!("({first}, {rest})"))
                    }
                    ("[", [value, unit]) => Some(format!("({value} [{unit}])")),
                    ("#", [from, index]) => Some(format!("({from})#({index})")),
                    (_, [one]) => Some(format!("({operator}{}{one})", spacer(operator))),
                    (_, [left, right]) => Some(format!("({left} {operator} {right})")),
                    _ => None,
                }
            }
            K::TriggerInvocationExpression => {
                let how = model.maybe(expr, "kind").and_then(Value::as_str)?;
                let [what]: [String; 1] = arguments()?.try_into().ok()?;
                Some(format!("{how} {what}"))
            }
            // a body: the parameters it declares, and the expression it
            // comes to
            K::Expression => {
                let result = model.owned(expr).iter().copied().find(|&it| {
                    model.member_role(it) == Some(Role::Result)
                        && model.kind(it).is_a(K::Expression)
                })?;
                let declared: Vec<ElementId> = model
                    .owned(expr)
                    .iter()
                    .copied()
                    .filter(|&it| {
                        model.kind(it) == K::Feature
                            && model.direction(it) == Some("in")
                            && model.name(it).is_some()
                    })
                    .collect();
                let came_to = self.expr(result)?;
                if declared.is_empty() {
                    return Some(came_to);
                }
                let mut said = String::from("{ ");
                for parameter in declared {
                    let _ = write!(said, "in {}; ", sysml_syntax::quote(model.name(parameter)?));
                }
                let _ = write!(said, "{came_to} }}");
                Some(said)
            }
            _ => None,
        }
    }

    /// A literal as the notation writes it, or nothing for an expression
    /// that is not one.
    fn literal(&self, expr: ElementId) -> Option<String> {
        let model = self.model;
        // asked only of a literal that has a value: `*` has none, and
        // asking it for one is asking for a property it does not have
        let value = model.maybe(expr, "value");
        match (model.kind(expr), value) {
            (K::LiteralInfinity, _) => Some("*".to_string()),
            (K::LiteralInteger, Some(Value::Int(n))) => Some(n.to_string()),
            (K::LiteralRational, Some(Value::Real(r))) => Some(format!("{r:?}")),
            (K::LiteralRational, Some(Value::Int(n))) => Some(format!("{n}.0")),
            (K::LiteralBoolean, Some(Value::Bool(b))) => Some(b.to_string()),
            (K::LiteralString, Some(Value::String(s))) => Some(string(s)),
            _ => None,
        }
    }

    /// The name a type is written by where something is typed by it: a
    /// conjugated port definition is its port definition, conjugated.
    fn type_name(&self, ty: ElementId) -> Option<String> {
        if self.model.kind(ty) == K::ConjugatedPortDefinition {
            let original = self.model.owner(ty)?;
            return Some(format!("~{}", self.name_of(original)?));
        }
        self.name_of(ty)
    }

    /// The features a feature is the chain of, where it is one.
    fn chain(&self, feature: ElementId) -> Vec<ElementId> {
        let model = self.model;
        match model.maybe(feature, "chainingFeature") {
            Some(Value::RefList(all)) if !all.is_empty() => all.clone(),
            _ => model
                .owned(feature)
                .iter()
                .filter(|&&it| model.kind(it) == K::FeatureChaining)
                .filter_map(|&it| match model.maybe(it, "chainingFeature") {
                    Some(Value::Ref(link)) => Some(*link),
                    _ => None,
                })
                .collect(),
        }
    }

    /// `a.b.c` for a feature that is the chain of `a`, `b` and `c`.
    fn chained(&self, feature: ElementId) -> Option<String> {
        let chain = self.chain(feature);
        let (first, rest) = chain.split_first()?;
        let mut said = self.name_of(*first)?;
        for &link in rest {
            said.push('.');
            said.push_str(&sysml_syntax::quote(&self.simple_name(link)?));
        }
        Some(said)
    }

    /// The name a feature is referred to by: its qualified name, or the
    /// chain of names it is.
    fn feature_ref(&self, feature: ElementId) -> Option<String> {
        if !self.outside.contains_key(&feature)
            && self.model.name(feature).is_none()
            && !self.chain(feature).is_empty()
        {
            return self.chained(feature);
        }
        self.name_of(feature)
    }

    /// The name `elem` is referred to by from where it is being written:
    /// the name `outside` gives it, or its path down from the nearest
    /// namespace around the scope that holds it -- `Automobile` within
    /// the package that declares it, where `'Package Example'::Automobile`
    /// would not reach a private one -- taken from further out wherever a
    /// nearer declaration of the same name would be found first. Nothing
    /// where a step on the way down has no name.
    fn name_of(&self, elem: ElementId) -> Option<String> {
        let model = self.model;
        let mut around: Vec<ElementId> = Vec::new();
        let mut at = self.scope;
        while let Some(e) = at {
            around.push(e);
            at = model.owner(e);
        }
        // the name alone, where an import around the scope brings it in
        // and nothing nearer answers to it: what `private import
        // ScalarValues::*;` is written for, and the only way to a private
        // member a view exposes
        if let Some(name) = self.simple_name(elem).filter(|_| !self.importing) {
            for (depth, &nearer) in around.iter().enumerate() {
                if self.declares(nearer, &name, elem) {
                    break;
                }
                if self.imports(nearer, elem) {
                    let shadowed = around[..depth]
                        .iter()
                        .any(|&closer| self.declares(closer, &name, elem));
                    if !shadowed {
                        return Some(sysml_syntax::quote(&name));
                    }
                    break;
                }
            }
        }
        if let Some(named) = self.outside.get(&elem) {
            // the library's own name, unless something nearer answers to
            // its first segment: `$::Views::asTreeDiagram` past a package
            // of the model's own called `Views`
            let first = sysml_syntax::split_qualified(named).into_iter().next()?;
            let shadowed = around
                .iter()
                .any(|&nearer| self.declares(nearer, &first, elem));
            return Some(match shadowed {
                true => format!("$::{named}"),
                false => named.clone(),
            });
        }
        let path = self.path(elem);
        // the nearest namespace around the scope that holds `elem`, and
        // the names down from it
        for (depth, &outer) in around.iter().enumerate() {
            let Some(from) = path.iter().position(|(e, _)| *e == outer) else {
                continue;
            };
            let below = &path[from + 1..];
            // the scope is `elem` itself or inside it: it is named from
            // further out
            let Some((first_at, Some(first))) = below.first() else {
                continue;
            };
            let Some(names) = below
                .iter()
                .map(|(_, name)| name.as_deref())
                .collect::<Option<Vec<&str>>>()
            else {
                continue;
            };
            // a nearer declaration of the first name would be found first
            let shadowed = around[..depth]
                .iter()
                .any(|&nearer| self.declares(nearer, first, *first_at));
            if shadowed {
                continue;
            }
            return Some(sysml_syntax::quote_qualified(names));
        }
        // from the top, where every step is named
        let names: Vec<&str> = path
            .iter()
            .skip_while(|(e, name)| name.is_none() && model.owner(*e).is_none())
            .map(|(_, name)| name.as_deref())
            .collect::<Option<Vec<&str>>>()?;
        let first = names.first()?;
        let shadowed = around.iter().any(|&nearer| {
            path.iter()
                .find(|(_, name)| name.as_deref() == Some(*first))
                .is_some_and(|&(top, _)| self.declares(nearer, first, top))
        });
        let said = sysml_syntax::quote_qualified(names);
        Some(match shadowed {
            true => format!("$::{said}"),
            false => said,
        })
    }

    /// Whether `namespace` imports `elem` by its name: an import of the
    /// namespace it is in, or of it.
    fn imports(&self, namespace: ElementId, elem: ElementId) -> bool {
        let model = self.model;
        let owner_name = |of: ElementId| -> Option<String> {
            // a stand-in for the library has no owner here, and is in the
            // namespace its name says
            let named = self.outside.get(&of)?;
            let mut segments = sysml_syntax::split_qualified(named);
            segments.pop();
            Some(sysml_syntax::quote_qualified(
                segments.iter().map(String::as_str),
            ))
        };
        model.owned(namespace).iter().any(|&member| {
            if model.flag(member, "isImplied") {
                return false;
            }
            match (model.kind(member), member_target(model, member)) {
                (K::NamespaceImport | K::NamespaceExpose, Some(imported)) => {
                    model.owner(elem) == Some(imported)
                        || (self.outside.contains_key(&elem)
                            && self.outside.get(&imported).is_some()
                            && owner_name(elem).as_ref() == self.outside.get(&imported))
                }
                (K::MembershipImport | K::MembershipExpose, Some(imported)) => imported == elem,
                _ => false,
            }
        })
    }

    /// `elem` and every namespace it is in, from the top down, each with
    /// the name it answers to, where it answers to one.
    fn path(&self, elem: ElementId) -> Vec<(ElementId, Option<String>)> {
        let model = self.model;
        let mut path = Vec::new();
        let mut at = Some(elem);
        while let Some(e) = at {
            path.push((e, self.simple_name(e)));
            at = model.owner(e);
        }
        path.reverse();
        path
    }

    /// The one name an element answers to: its effective name, or, where
    /// that is borrowed from something the document only refers to, the
    /// last segment of the name that stands for it. A stand-in for the
    /// library is named by its `@id`, and a redefinition that borrows its
    /// name was named by that.
    fn simple_name(&self, elem: ElementId) -> Option<String> {
        let model = self.model;
        let naming = model.naming_element(elem).unwrap_or(elem);
        if let Some(named) = self
            .outside
            .get(&naming)
            .or_else(|| self.outside.get(&elem))
        {
            return sysml_syntax::split_qualified(named).pop();
        }
        model.effective_name(elem).map(str::to_string)
    }

    /// Whether `namespace` holds something other than `but` that answers
    /// to `name`: a member of its own, one it inherits from a type it
    /// specializes, or one an import of it brings in.
    fn declares(&self, namespace: ElementId, name: &str, but: ElementId) -> bool {
        let mut seen = HashSet::new();
        self.declares_within(namespace, name, but, &mut seen, 0)
    }

    fn declares_within(
        &self,
        namespace: ElementId,
        name: &str,
        but: ElementId,
        seen: &mut HashSet<ElementId>,
        depth: usize,
    ) -> bool {
        let model = self.model;
        if depth > 8 || !seen.insert(namespace) {
            return false;
        }
        for &member in model.owned(namespace) {
            if member == but {
                continue;
            }
            let answers = model.effective_name(member) == Some(name)
                || model.maybe(member, "memberName").and_then(Value::as_str) == Some(name)
                || model
                    .maybe(member, "declaredShortName")
                    .and_then(Value::as_str)
                    == Some(name);
            if answers && !model.flag(member, "isImplied") {
                return true;
            }
            let general = match model.kind(member) {
                K::FeatureTyping => model.maybe(member, "type"),
                K::Subclassification => model.maybe(member, "superclassifier"),
                K::Subsetting | K::ReferenceSubsetting => model.maybe(member, "subsettedFeature"),
                K::Redefinition => model.maybe(member, "redefinedFeature"),
                K::NamespaceImport => model.maybe(member, "importedNamespace"),
                K::MembershipImport => {
                    if let Some(Value::Ref(imported)) = model.maybe(member, "importedMembership") {
                        if *imported != but && model.effective_name(*imported) == Some(name) {
                            return true;
                        }
                    }
                    None
                }
                _ => None,
            };
            if let Some(Value::Ref(general)) = general {
                if !self.outside.contains_key(general)
                    && self.declares_within(*general, name, but, seen, depth + 1)
                {
                    return true;
                }
            }
        }
        false
    }

    fn visibility(&self, elem: ElementId) -> &'static str {
        match self.model.member_visibility(elem) {
            Some(Vis::Private) => "private",
            Some(Vis::Protected) => "protected",
            _ => "",
        }
    }

    /// An element this does not write: said in the text where it would
    /// have been, so a reader sees the gap, and listed.
    fn leave_out(&mut self, elem: ElementId, indent: &str) {
        let kind = self.model.kind(elem).name();
        let _ = writeln!(self.out, "{indent}// not written: {kind}");
        self.unwritten.push((kind.to_string(), self.where_is(elem)));
    }

    /// The element something left out was in, by name.
    fn where_is(&self, elem: ElementId) -> String {
        let mut at = Some(elem);
        while let Some(e) = at {
            if let Some(name) = self.name_of(e) {
                return name;
            }
            at = self.model.owner(e);
        }
        "the top level".to_string()
    }
}

/// What a relationship of this metaclass relates, as the properties that
/// name its two sides, and how a declaration of one is written: the word
/// that names it, and the words before the one side and between the two.
#[allow(clippy::type_complexity)]
fn relates(
    kind: K,
) -> Option<(
    &'static str,
    &'static str,
    &'static str,
    (&'static str, &'static str),
)> {
    Some(match kind {
        K::Subclassification => (
            "subclassifier",
            "superclassifier",
            "specialization",
            ("subclassifier", "specializes"),
        ),
        K::Specialization => (
            "specific",
            "general",
            "specialization",
            ("subtype", "specializes"),
        ),
        K::FeatureTyping => (
            "typedFeature",
            "type",
            "specialization",
            ("typing", "typed by"),
        ),
        K::Subsetting => (
            "subsettingFeature",
            "subsettedFeature",
            "specialization",
            ("subset", "subsets"),
        ),
        K::Redefinition => (
            "redefiningFeature",
            "redefinedFeature",
            "specialization",
            ("redefinition", "redefines"),
        ),
        K::Conjugation => (
            "conjugatedType",
            "originalType",
            "conjugation",
            ("conjugate", "conjugates"),
        ),
        K::Disjoining => (
            "typeDisjoined",
            "disjoiningType",
            "disjoining",
            ("disjoint", "from"),
        ),
        K::FeatureInverting => (
            "featureInverted",
            "invertingFeature",
            "inverting",
            ("inverse", "of"),
        ),
        K::TypeFeaturing => (
            "featureOfType",
            "featuringType",
            "featuring",
            ("featuring", "by"),
        ),
        _ => return None,
    })
}

/// What an import or exposure brings in: the namespace whose members it
/// imports, or the one membership.
fn member_target(model: &Model, import: ElementId) -> Option<ElementId> {
    let prop = match model.kind(import) {
        K::NamespaceImport | K::NamespaceExpose => "importedNamespace",
        _ => "importedMembership",
    };
    match model.maybe(import, prop) {
        Some(Value::Ref(target)) => Some(*target),
        _ => None,
    }
}

/// Whether a usage of `kind` owned by a type of kind `owning` is composite
/// where it does not say: what `sysml_model`'s builder decides of one.
fn composite_by_default(kind: K, owning: K) -> bool {
    if kind == K::ReferenceUsage
        || kind.is_a(K::AttributeUsage)
        || kind.is_a(K::EventOccurrenceUsage)
        || !kind.is_a(K::Usage)
        || owning.is_a(K::DataType)
        || owning.is_a(K::AttributeUsage)
    {
        return false;
    }
    if owning.is_a(K::PortDefinition) || owning.is_a(K::PortUsage) {
        return kind.is_a(K::PortUsage);
    }
    !kind.is_a(K::PortUsage)
}

/// A comment's body between `/*` and `*/`, each line after the first down
/// a margin of `*`: what is read back from that is each line as it is,
/// indentation and all, where a margin of spaces alone would be taken off
/// with the indentation the lines share.
fn comment(body: &str, indent: &str) -> String {
    let lines: Vec<&str> = body.trim().lines().collect();
    match lines.as_slice() {
        [] => "/* */".to_string(),
        [one] => format!("/* {one} */"),
        [first, rest @ ..] => {
            let mut said = format!("/* {first}");
            for line in rest {
                let _ = write!(said, "\n{indent} * {line}");
            }
            let _ = write!(said, "\n{indent} */");
            said
        }
    }
}

/// A string literal: quoted, with what would end it escaped.
fn string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// What goes between a prefix operator and its operand: nothing after a
/// symbol, a space after a word.
fn spacer(operator: &str) -> &'static str {
    match operator.chars().all(|c| c.is_ascii_alphabetic()) {
        true => " ",
        false => "",
    }
}

/// How a connector of this metaclass is written, where it is one.
fn connects(kind: K) -> Option<Connects> {
    let (keyword, lead, between, alone) = match kind {
        K::ConnectionUsage => ("connection", "connect", "to", true),
        K::InterfaceUsage => ("interface", "connect", "to", false),
        K::AllocationUsage => ("allocation", "allocate", "to", true),
        K::BindingConnectorAsUsage => ("binding", "bind", "=", true),
        K::SuccessionAsUsage => ("succession", "first", "then", true),
        K::FlowUsage => ("flow", "from", "to", false),
        K::SuccessionFlowUsage => ("succession flow", "from", "to", false),
        K::Connector => ("connector", "from", "to", false),
        K::BindingConnector => ("binding", "", "=", false),
        K::Succession => ("succession", "first", "then", false),
        K::Flow => ("flow", "from", "to", false),
        K::SuccessionFlow => ("succession flow", "from", "to", false),
        _ => return None,
    };
    Some(Connects {
        keyword,
        lead,
        between,
        alone,
    })
}

/// The word a performance, exhibition, inclusion, event or assertion of
/// something declared elsewhere is written with.
fn performs(kind: K) -> Option<&'static str> {
    Some(match kind {
        K::PerformActionUsage => "perform",
        K::ExhibitStateUsage => "exhibit",
        K::IncludeUseCaseUsage => "include",
        K::EventOccurrenceUsage => "event",
        K::AssertConstraintUsage => "assert",
        _ => return None,
    })
}

/// The keyword a declaration of this metaclass is written with, for the
/// metaclasses written as `keyword name : T :> S [m] { ... }` -- and
/// nothing for the rest, whose notation is a construct of its own.
fn keyword(kind: K) -> Option<&'static str> {
    Some(match kind {
        K::Package => "package",
        K::LibraryPackage => "library package",
        K::Namespace => "namespace",
        K::AttributeDefinition => "attribute def",
        K::AttributeUsage => "attribute",
        K::EnumerationDefinition => "enum def",
        K::EnumerationUsage => "enum",
        K::OccurrenceDefinition => "occurrence def",
        K::OccurrenceUsage => "occurrence",
        K::EventOccurrenceUsage => "event occurrence",
        K::ItemDefinition => "item def",
        K::ItemUsage => "item",
        K::PartDefinition => "part def",
        K::PartUsage => "part",
        K::PortDefinition => "port def",
        K::PortUsage => "port",
        K::ReferenceUsage => "ref",
        K::ConnectionDefinition => "connection def",
        K::ConnectionUsage => "connection",
        K::InterfaceDefinition => "interface def",
        K::InterfaceUsage => "interface",
        K::AllocationDefinition => "allocation def",
        K::AllocationUsage => "allocation",
        K::FlowDefinition => "flow def",
        K::FlowUsage => "flow",
        K::ActionDefinition => "action def",
        K::ActionUsage => "action",
        K::PerformActionUsage => "perform action",
        K::DecisionNode => "decide",
        K::MergeNode => "merge",
        K::ForkNode => "fork",
        K::JoinNode => "join",
        K::StateDefinition => "state def",
        K::StateUsage => "state",
        K::ExhibitStateUsage => "exhibit state",
        K::ConstraintDefinition => "constraint def",
        K::ConstraintUsage => "constraint",
        K::AssertConstraintUsage => "assert constraint",
        K::RequirementDefinition => "requirement def",
        K::RequirementUsage => "requirement",
        K::ConcernDefinition => "concern def",
        K::ConcernUsage => "concern",
        K::CalculationDefinition => "calc def",
        K::CalculationUsage => "calc",
        K::CaseDefinition => "case def",
        K::CaseUsage => "case",
        K::AnalysisCaseDefinition => "analysis def",
        K::AnalysisCaseUsage => "analysis",
        K::VerificationCaseDefinition => "verification def",
        K::VerificationCaseUsage => "verification",
        K::UseCaseDefinition => "use case def",
        K::UseCaseUsage => "use case",
        K::IncludeUseCaseUsage => "include use case",
        K::ViewDefinition => "view def",
        K::ViewUsage => "view",
        K::ViewpointDefinition => "viewpoint def",
        K::ViewpointUsage => "viewpoint",
        K::RenderingDefinition => "rendering def",
        K::RenderingUsage => "rendering",
        K::MetadataDefinition => "metadata def",
        // what a user's keyword declares: `#service def X;`
        K::Definition => "def",
        K::Type => "type",
        K::Classifier => "classifier",
        K::Class => "class",
        K::DataType => "datatype",
        K::Structure => "struct",
        K::Association => "assoc",
        K::AssociationStructure => "assoc struct",
        K::Behavior => "behavior",
        K::Function => "function",
        K::Predicate => "predicate",
        K::Interaction => "interaction",
        K::Metaclass => "metaclass",
        K::Feature => "feature",
        K::Step => "step",
        K::Expression => "expr",
        K::Connector => "connector",
        K::BindingConnector => "binding",
        K::Succession => "succession",
        K::Flow => "flow",
        K::SuccessionFlow => "succession flow",
        K::BooleanExpression => "bool",
        K::Invariant => "inv",
        _ => return None,
    })
}

/// Whether a metaclass is written only in KerML.
fn kerml_only(kind: K) -> bool {
    matches!(
        kind,
        K::Type
            | K::Classifier
            | K::Class
            | K::DataType
            | K::Structure
            | K::Association
            | K::AssociationStructure
            | K::Behavior
            | K::Function
            | K::Predicate
            | K::Interaction
            | K::Metaclass
            | K::Feature
            | K::Step
            | K::Namespace
            | K::Connector
            | K::BindingConnector
            | K::Succession
            | K::Flow
            | K::SuccessionFlow
            | K::BooleanExpression
            | K::Invariant
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text`, resolved without a library, exported and read back the way
    /// `import_json` reads a document, and written -- from the text each
    /// expression kept, or from what it is.
    fn written(text: &str, kept_text: bool) -> Written {
        let mut ws = sysml_semantics::Workspace::new();
        ws.add_file("m.sysml", text);
        ws.resolve_all();
        // what an import imports is the resolver's, and is handed to the
        // export as `sysml export` hands it
        let mut extras = sysml_interchange::Extras::default();
        let ids: Vec<ElementId> = ws.model().ids().collect();
        for id in ids {
            if let Some(target) = ws.imported_alias(id).or_else(|| ws.import_of(id)) {
                extras.import_targets.insert(id, target);
            }
        }
        let document = sysml_interchange::to_json_with(ws.model(), &extras).to_string();
        let (model, roots, _) = sysml_interchange::read_json(document.as_bytes()).unwrap();
        write_with(&model, &roots, &HashMap::new(), kept_text)
    }

    #[test]
    fn an_expression_is_written_from_what_it_is_where_no_text_was_kept() {
        let text = "package P {\n\
                    \tcalc def Sq { in x; x * x }\n\
                    \tattribute a = 1 + 2 * 3;\n\
                    \tattribute b = -a;\n\
                    \tattribute c = if a > 0 ? a else 0;\n\
                    \tattribute d = Sq(a);\n\
                    \tattribute e = (1, 2, 3);\n\
                    \tattribute f = \"say \\\"so\\\"\";\n\
                    \tattribute g = null;\n\
                    }\n";
        let from_structure = written(text, false);
        let sysml = &from_structure.sysml;
        for said in [
            "x * x",
            "attribute a = (1 + (2 * 3));",
            "attribute b = (-a);",
            "attribute c = (if (a > 0) ? a else 0);",
            "attribute d = Sq(a);",
            "attribute e = (1, 2, 3);",
            "attribute f = \"say \\\"so\\\"\";",
            "attribute g = null;",
        ] {
            assert!(sysml.contains(said), "no `{said}` in\n{sysml}");
        }
        assert!(
            from_structure.unwritten.is_empty(),
            "{:?}",
            from_structure.unwritten
        );
        // and the text kept, where it was
        let kept = written(text, true).sysml;
        assert!(kept.contains("attribute a = 1 + 2 * 3;"), "{kept}");
        assert!(
            kept.contains("attribute c = if a > 0 ? a else 0;"),
            "{kept}"
        );
    }

    /// A name is written the way it is found from where it stands: what an
    /// import brings in by itself, a private member from inside the
    /// namespace that holds it, and from `$::` where something nearer
    /// answers to the name it starts with.
    #[test]
    fn a_name_is_written_the_way_it_is_found_from_where_it_stands() {
        let text = "package Lib { part def Wheel; }\n\
                    package P {\n\
                    \tprivate import Lib::*;\n\
                    \tprivate part def Hidden;\n\
                    \tpart w : Wheel;\n\
                    \tpart h : Hidden;\n\
                    \tpart def Q {\n\
                    \t\tpart def Lib;\n\
                    \t\tpart def Wheel;\n\
                    \t\tpart x : $::Lib::Wheel;\n\
                    \t\tpart y : Wheel;\n\
                    \t}\n\
                    }\n";
        let sysml = written(text, true).sysml;
        assert!(sysml.contains("part w : Wheel;"), "{sysml}");
        assert!(sysml.contains("part h : Hidden;"), "{sysml}");
        // past a `Lib` and a `Wheel` of `Q`'s own
        assert!(sysml.contains("part x : $::Lib::Wheel;"), "{sysml}");
        assert!(sysml.contains("part y : Wheel;"), "{sysml}");
        // an import names what it imports in full
        assert!(sysml.contains("private import Lib::*;"), "{sysml}");
    }
}

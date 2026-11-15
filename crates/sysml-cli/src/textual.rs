//! A model read from interchange JSON, written back out as the textual
//! notation.
//!
//! A document of the standard's JSON is an array of objects with UUIDs for
//! names, and what a reader wants to know of one is what a person would
//! have written: `part def Truck :> Vehicle { part :>> wheels[6]; }`. This
//! writes the declarations a model holds -- packages, imports, aliases,
//! definitions and usages, with their typings, specializations,
//! subsettings, redefinitions, multiplicities, literal values and
//! documentation -- and says of everything else that it was not written,
//! rather than guessing at it. Behaviour (successions, transitions,
//! actions with bodies), connector ends and any expression that is not a
//! literal are what it leaves out: each needs the grammar of the construct
//! it is in, and a wrong guess at one reads as a model that says
//! something it does not.
//!
//! Every name it writes for something the model refers to is qualified,
//! so the text resolves without depending on which imports came through.

use std::collections::HashMap;
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
    let mut writer = Writer {
        model,
        outside,
        out: String::new(),
        unwritten: Vec::new(),
        kerml: false,
    };
    for &root in roots {
        if outside.contains_key(&root) {
            continue;
        }
        // the namespace a workspace hangs its files off has no name and is
        // no declaration: what it holds is the top level of the text
        if model.kind(root) == K::Namespace && model.name(root).is_none() {
            writer.members(root, 0);
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
}

impl Writer<'_> {
    /// Every member of `owner` that is a declaration of its own, one after
    /// another. Relationships are said by the declaration they belong to.
    fn members(&mut self, owner: ElementId, depth: usize) {
        for &child in self.model.owned(owner) {
            if self.outside.contains_key(&child) || self.said_in_the_header(child) {
                continue;
            }
            self.member(child, depth);
        }
    }

    /// Whether `child` is said by the declaration that owns it, or is
    /// nothing the text says at all.
    fn said_in_the_header(&self, child: ElementId) -> bool {
        let kind = self.model.kind(child);
        // what the toolchain implied, the reader of the text implies again
        self.model.flag(child, "isImplied")
            || matches!(
                kind,
                K::FeatureTyping
                    | K::ConjugatedPortTyping
                    | K::Subclassification
                    | K::Subsetting
                    | K::Redefinition
                    | K::ReferenceSubsetting
                    | K::Specialization
                    | K::FeatureValue
                    | K::MultiplicityRange
                    // `~P` is declared by `port def P` without a word
                    | K::ConjugatedPortDefinition
                    | K::PortConjugation
            )
    }

    fn member(&mut self, elem: ElementId, depth: usize) {
        let model = self.model;
        let kind = model.kind(elem);
        let indent = "    ".repeat(depth);
        if matches!(kind, K::Documentation | K::Comment) {
            let body = model
                .get(elem, "body")
                .and_then(Value::as_str)
                .unwrap_or_default()
                // a body cannot hold the end of the comment it is in
                .replace("*/", "* /");
            let word = match kind {
                K::Documentation => "doc",
                _ => "comment",
            };
            let _ = writeln!(self.out, "{indent}{word} /* {} */", body.trim());
            return;
        }
        if matches!(kind, K::NamespaceImport | K::MembershipImport) {
            self.import(elem, &indent);
            return;
        }
        // an alias is a membership that carries a name of its own
        if kind == K::Membership {
            let target = match model.get(elem, "memberElement") {
                Some(Value::Ref(target)) => self.name_of(*target),
                _ => None,
            };
            let name = model.get(elem, "memberName").and_then(Value::as_str);
            if let (Some(target), Some(name)) = (target, name) {
                let _ = writeln!(
                    self.out,
                    "{indent}{}alias {} for {target};",
                    match self.visibility(elem) {
                        "" => String::new(),
                        said => format!("{said} "),
                    },
                    sysml_syntax::quote(name)
                );
                return;
            }
        }
        let Some(keyword) = keyword(kind) else {
            self.leave_out(elem, &indent);
            return;
        };
        if kerml_only(kind) {
            self.kerml = true;
        }
        let mut words: Vec<String> = Vec::new();
        let visibility = self.visibility(elem);
        if !visibility.is_empty() {
            words.push(visibility.to_string());
        }
        let role = model.member_role(elem);
        let said = match role {
            Some(Role::Subject) => "subject",
            Some(Role::Actor) => "actor",
            Some(Role::Stakeholder) => "stakeholder",
            Some(Role::Objective) => "objective",
            Some(Role::Return) => "return",
            Some(Role::Variant) => "variant",
            Some(Role::Entry) => "entry",
            Some(Role::Do) => "do",
            Some(Role::Exit) => "exit",
            Some(Role::Assume) => "assume",
            Some(Role::Require) => "require",
            Some(Role::Frame) => "frame",
            Some(Role::Verify) => "verify",
            // the trailing expression of a body, and how a view is
            // rendered, are written as nothing a declaration is
            Some(Role::Result | Role::Render) => {
                self.leave_out(elem, &indent);
                return;
            }
            None => "",
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
        if !parameter {
            if let Some(direction) = model.direction(elem) {
                words.push(direction.to_string());
            }
        }
        for (flag, word) in [
            ("isAbstract", "abstract"),
            ("isVariation", "variation"),
            ("isReadOnly", "readonly"),
            ("isDerived", "derived"),
            ("isConstant", "constant"),
            ("isEnd", "end"),
        ] {
            // asked only of what declares the flag: a package has no
            // `isAbstract`, and a default read off another metaclass is
            // nothing the model said
            if kind.feature(flag).is_some() && model.flag(elem, flag) {
                words.push(word.to_string());
            }
        }
        // a role written alone stands for its usage: `subject v : V;`
        if !parameter || kind.is_a(K::Definition) {
            words.push(keyword.to_string());
        }
        if let Some(short) = model.get(elem, "declaredShortName").and_then(Value::as_str) {
            words.push(format!("<{}>", sysml_syntax::quote(short)));
        }
        if let Some(name) = model.name(elem) {
            words.push(sysml_syntax::quote(name));
        }
        let mut line = format!("{indent}{}", words.join(" "));
        self.header(elem, &mut line);
        let body: Vec<ElementId> = model
            .owned(elem)
            .iter()
            .copied()
            .filter(|&child| !self.outside.contains_key(&child) && !self.said_in_the_header(child))
            .collect();
        if body.is_empty() {
            let _ = writeln!(self.out, "{};", line.trim_end());
            return;
        }
        let _ = writeln!(self.out, "{} {{", line.trim_end());
        self.members(elem, depth + 1);
        let _ = writeln!(self.out, "{indent}}}");
    }

    /// What a declaration says between its name and its body: `: T`,
    /// `:> S`, `:>> R`, `::> F`, `[m]` and `= v`, in the order the
    /// notation puts them.
    fn header(&mut self, elem: ElementId, line: &mut String) {
        let model = self.model;
        let mut typed = Vec::new();
        let mut specialized = Vec::new();
        let mut redefined = Vec::new();
        let mut referenced = Vec::new();
        let mut multiplicity = None;
        let mut value = None;
        for &rel in model.owned(elem) {
            if model.flag(rel, "isImplied") {
                continue;
            }
            let target = |prop: &str| match model.get(rel, prop) {
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
                K::Subsetting => {
                    if let Some(name) = target("subsettedFeature").and_then(|t| self.name_of(t)) {
                        specialized.push(name);
                    }
                }
                K::Redefinition => {
                    if let Some(name) = target("redefinedFeature").and_then(|t| self.name_of(t)) {
                        redefined.push(name);
                    }
                }
                K::ReferenceSubsetting => {
                    if let Some(name) = target("referencedFeature").and_then(|t| self.name_of(t)) {
                        referenced.push(name);
                    }
                }
                K::MultiplicityRange => multiplicity = self.multiplicity(rel),
                K::FeatureValue => {
                    let said = target("value").and_then(|v| self.literal(v));
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
        if !typed.is_empty() {
            let _ = write!(line, " : {}", typed.join(", "));
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
        if let Some(multiplicity) = multiplicity {
            let _ = write!(line, " {multiplicity}");
        }
        if let Some(value) = value {
            let _ = write!(line, " {value}");
        }
    }

    fn import(&mut self, elem: ElementId, indent: &str) {
        let model = self.model;
        let (prop, all) = match model.kind(elem) {
            K::NamespaceImport => ("importedNamespace", true),
            _ => ("importedMembership", false),
        };
        let Some(target) = (match model.get(elem, prop) {
            Some(Value::Ref(target)) => self.name_of(*target),
            _ => None,
        }) else {
            self.leave_out(elem, indent);
            return;
        };
        let visibility = match model.member_visibility(elem) {
            Some(Vis::Public) => "public ",
            Some(Vis::Protected) => "protected ",
            Some(Vis::Private) => "private ",
            None => "",
        };
        let recursive = model.flag(elem, "isRecursive");
        let tail = match (all, recursive) {
            (true, true) => "::*::**",
            (true, false) => "::*",
            (false, true) => "::**",
            (false, false) => "",
        };
        let _ = writeln!(self.out, "{indent}{visibility}import {target}{tail};");
    }

    /// `[n]`, `[l..u]` or `[*]` from the bounds a multiplicity holds, where
    /// each is a literal.
    fn multiplicity(&mut self, range: ElementId) -> Option<String> {
        let model = self.model;
        let bounds: Vec<ElementId> = match model.get(range, "bound") {
            Some(Value::RefList(bounds)) => bounds.clone(),
            Some(Value::Ref(bound)) => vec![*bound],
            _ => model
                .owned(range)
                .iter()
                .copied()
                .filter(|&b| model.kind(b).is_a(K::Expression))
                .collect(),
        };
        let written: Option<Vec<String>> = bounds.iter().map(|&b| self.literal(b)).collect();
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

    /// A literal as the notation writes it, or nothing for an expression
    /// that is not one.
    fn literal(&self, expr: ElementId) -> Option<String> {
        let model = self.model;
        let value = model.get(expr, "value");
        match (model.kind(expr), value) {
            (K::LiteralInfinity, _) => Some("*".to_string()),
            (K::LiteralInteger, Some(Value::Int(n))) => Some(n.to_string()),
            (K::LiteralRational, Some(Value::Real(r))) => Some(format!("{r:?}")),
            (K::LiteralRational, Some(Value::Int(n))) => Some(format!("{n}.0")),
            (K::LiteralBoolean, Some(Value::Bool(b))) => Some(b.to_string()),
            (K::LiteralString, Some(Value::String(s))) => Some(format!(
                "\"{}\"",
                s.replace('\\', "\\\\").replace('"', "\\\"")
            )),
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

    /// The qualified name `elem` is referred to by: the name `outside`
    /// gives it, or its path down from the top of the model by the names
    /// each step answers to. Nothing where a step on the way has none.
    fn name_of(&self, elem: ElementId) -> Option<String> {
        if let Some(named) = self.outside.get(&elem) {
            return Some(named.clone());
        }
        let mut segments = Vec::new();
        let mut at = Some(elem);
        while let Some(e) = at {
            // the namespace the document hangs everything off names
            // nothing of its own
            if self.model.owner(e).is_none() && self.model.name(e).is_none() {
                break;
            }
            segments.push(self.model.effective_name(e)?);
            at = self.model.owner(e);
        }
        segments.reverse();
        (!segments.is_empty()).then(|| sysml_syntax::quote_qualified(segments))
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
        K::ActionDefinition => "action def",
        K::ActionUsage => "action",
        K::StateDefinition => "state def",
        K::StateUsage => "state",
        K::ConstraintDefinition => "constraint def",
        K::ConstraintUsage => "constraint",
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
        K::ViewDefinition => "view def",
        K::ViewUsage => "view",
        K::ViewpointDefinition => "viewpoint def",
        K::ViewpointUsage => "viewpoint",
        K::RenderingDefinition => "rendering def",
        K::RenderingUsage => "rendering",
        K::MetadataDefinition => "metadata def",
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
    )
}

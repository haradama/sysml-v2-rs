//! Generates `crates/sysml-model/src/generated.rs` from the normative
//! machine-readable metamodel published by the OMG with the KerML 1.0 and
//! SysML 2.0 specifications (`vendor/metamodel/KerML.xmi` and
//! `vendor/metamodel/SysML.xmi`).
//!
//! Run with `cargo run -p sysml-model --features codegen --bin
//! sysml-codegen`. The output is committed, so this
//! only needs to run again when the vendored metamodel is updated; a test
//! keeps the committed file in sync with the vendored metamodel.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

#[derive(Debug)]
struct Class {
    is_abstract: bool,
    supers: Vec<String>,
    features: Vec<Feature>,
}

#[derive(Debug)]
struct Feature {
    name: String,
    ty: FeatureTy,
    many: bool,
    derived: bool,
}

#[derive(Debug)]
enum FeatureTy {
    Data(&'static str),
    /// a class or enumeration, resolved by name after parsing
    Named(String),
}

#[derive(Debug)]
struct Enum {
    literals: Vec<String>,
}

/// One well-formedness constraint, as the specification states it.
#[derive(Debug)]
struct Rule {
    name: String,
    /// The metaclass it is about: the class the rule is written inside.
    metaclass: String,
    /// The OCL that has to hold of every instance of that metaclass.
    ocl: String,
    /// What the specification says the rule means, in prose.
    says: String,
}

/// Generate the Rust metamodel source from the KerML and SysML XMI
/// documents (in that order — SysML references KerML elements by URI).
pub fn generate_source(kerml_xmi: &str, sysml_xmi: &str) -> String {
    let mut classes: BTreeMap<String, Class> = BTreeMap::new();
    let mut enums: BTreeMap<String, Enum> = BTreeMap::new();
    // xmi:id -> classifier name, shared across both documents so that
    // `href="...KerML.xmi#<id>"` references resolve
    let mut ids: BTreeMap<String, String> = BTreeMap::new();

    for xml in [kerml_xmi, sysml_xmi] {
        let doc = roxmltree::Document::parse(xml).expect("invalid XMI");
        collect_ids(&doc, &mut ids);
    }
    for xml in [kerml_xmi, sysml_xmi] {
        let doc = roxmltree::Document::parse(xml).expect("invalid XMI");
        collect_classifiers(&doc, &ids, &mut classes, &mut enums);
    }
    let mut rules: Vec<Rule> = Vec::new();
    for xml in [kerml_xmi, sysml_xmi] {
        let doc = roxmltree::Document::parse(xml).expect("invalid XMI");
        collect_rules(&doc, &mut rules);
    }

    // transitive ancestors (excluding self), name-sorted for determinism
    let mut ancestors: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for name in classes.keys() {
        let mut acc = Vec::new();
        collect_ancestors(name, &classes, &mut acc);
        acc.sort();
        acc.dedup();
        ancestors.insert(name.clone(), acc);
    }

    generate(&classes, &enums, &ancestors, &rules)
}

/// The constraints the metamodel states, in the order it states them.
///
/// A rule is written inside the class it is about, so the owning class
/// names the metaclass -- thirteen of them carry no `constrainedElement`
/// of their own, and where the two are both there they agree.
fn collect_rules(doc: &roxmltree::Document, rules: &mut Vec<Rule>) {
    for node in doc.descendants() {
        if !node.has_tag_name("ownedRule") {
            continue;
        }
        // The metamodel writes three kinds of rule under one tag: the
        // `validate*` ones a model has to satisfy, and the `check*` and
        // `derive*` ones saying how a derived property is worked out.
        // Only the first is a constraint, and only one carrying OCL can
        // be evaluated -- so the two are one question.
        let (Some(name), Some(ocl)) = (
            node.attribute("name")
                .filter(|it| it.starts_with("validate")),
            node.children()
                .find(|c| c.has_tag_name("specification"))
                .and_then(|spec| spec.attribute("body")),
        ) else {
            continue;
        };
        let metaclass = node
            .ancestors()
            .find(|up| xmi_type(up) == Some("uml:Class"))
            .and_then(|up| up.attribute("name"))
            .expect("a rule is written inside the class it constrains");
        let says = node
            .children()
            .find(|c| c.has_tag_name("ownedComment"))
            .and_then(|comment| comment.attribute("body"))
            .map(plain)
            .unwrap_or_default();
        rules.push(Rule {
            name: name.to_string(),
            metaclass: metaclass.to_string(),
            ocl: ocl.to_string(),
            says,
        });
    }
}

/// The prose of a comment, with the HTML the metamodel writes it in
/// taken out: what is wanted is a sentence a reader can act on, and
/// `&lt;code&gt;` around a name is not part of it.
fn plain(html: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for ch in html.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    // one space between words, whatever the source wrapped with
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Regenerate `crates/sysml-model/src/generated.rs` from the vendored
/// metamodel (`SYSML_CODEGEN_OUT` overrides the output path). Returns the
/// output path.
pub fn run() -> std::path::PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out_path = match std::env::var("SYSML_CODEGEN_OUT") {
        Ok(path) => std::path::PathBuf::from(path),
        Err(_) => root.join("crates/sysml-model/src/generated.rs"),
    };
    let read = |name: &str| {
        let path = root.join("vendor/metamodel").join(name);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
    };
    let code = generate_source(&read("KerML.xmi"), &read("SysML.xmi"));
    std::fs::write(&out_path, code).expect("cannot write generated.rs");
    out_path
}

const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";
const XMI: &str = "http://www.omg.org/spec/XMI/20161101";
const PRIMITIVE_TYPES_URI: &str = "https://www.omg.org/spec/UML/20161101/PrimitiveTypes.xmi#";

fn xmi_type<'a>(node: &roxmltree::Node<'a, '_>) -> Option<&'a str> {
    node.attribute((XMI, "type"))
        .or_else(|| node.attribute((XSI, "type")))
}

fn collect_ids(doc: &roxmltree::Document, ids: &mut BTreeMap<String, String>) {
    for node in doc.descendants() {
        if matches!(xmi_type(&node), Some("uml:Class") | Some("uml:Enumeration")) {
            let name = node.attribute("name").expect("classifier without name");
            if let Some(id) = node.attribute((XMI, "id")) {
                ids.insert(id.to_string(), name.to_string());
            }
        }
    }
}

fn collect_classifiers(
    doc: &roxmltree::Document,
    ids: &BTreeMap<String, String>,
    classes: &mut BTreeMap<String, Class>,
    enums: &mut BTreeMap<String, Enum>,
) {
    for node in doc.descendants() {
        match xmi_type(&node) {
            Some("uml:Class") => {
                let name = node.attribute("name").expect("class without name");
                let supers = node
                    .children()
                    .filter(|c| c.has_tag_name("generalization"))
                    .map(|g| {
                        let general = g
                            .children()
                            .find(|c| c.has_tag_name("general"))
                            .expect("generalization without general");
                        resolve_ref(&general, ids)
                    })
                    .collect();
                let features = node
                    .children()
                    .filter(|c| c.has_tag_name("ownedAttribute"))
                    .map(|attr| parse_feature(&attr, ids))
                    .collect();
                let previous = classes.insert(
                    name.to_string(),
                    Class {
                        is_abstract: node.attribute("isAbstract") == Some("true"),
                        supers,
                        features,
                    },
                );
                assert!(previous.is_none(), "duplicate metaclass {name}");
            }
            Some("uml:Enumeration") => {
                let name = node.attribute("name").expect("enumeration without name");
                let literals = node
                    .children()
                    .filter(|c| c.has_tag_name("ownedLiteral"))
                    .map(|l| {
                        l.attribute("name")
                            .expect("literal without name")
                            .to_string()
                    })
                    .collect();
                enums.insert(name.to_string(), Enum { literals });
            }
            _ => {}
        }
    }
}

fn parse_feature(attr: &roxmltree::Node, ids: &BTreeMap<String, String>) -> Feature {
    let name = attr.attribute("name").expect("property without name");
    let type_node = attr
        .children()
        .find(|c| c.has_tag_name("type"))
        .unwrap_or_else(|| panic!("property {name} without type"));
    let ty = match type_node.attribute("href") {
        Some(href) if href.starts_with(PRIMITIVE_TYPES_URI) => {
            let primitive = &href[PRIMITIVE_TYPES_URI.len()..];
            FeatureTy::Data(match primitive {
                "Boolean" => "Boolean",
                "Integer" => "Integer",
                "Real" => "Real",
                "String" => "String",
                "UnlimitedNatural" => "UnlimitedNatural",
                other => panic!("unknown primitive type {other}"),
            })
        }
        _ => FeatureTy::Named(resolve_ref(&type_node, ids)),
    };
    // A property holds a collection wherever its upper bound is not one.
    // `MultiplicityRange::bound [1..2]` holds a pair as readily as
    // `[0..*]` holds any number, and reading only `*` as many left that
    // one and `Flow::flowEnd [0..2]` looking like single values.
    let many = attr
        .children()
        .find(|c| c.has_tag_name("upperValue"))
        .and_then(|u| u.attribute("value"))
        .is_some_and(|v| v != "1");
    Feature {
        name: name.to_string(),
        ty,
        many,
        derived: attr.attribute("isDerived") == Some("true"),
    }
}

/// Resolve an element reference: `xmi:idref="id"` or `href="...#id"`.
fn resolve_ref(node: &roxmltree::Node, ids: &BTreeMap<String, String>) -> String {
    let id = node
        .attribute((XMI, "idref"))
        .or_else(|| node.attribute("href").and_then(|h| h.rsplit('#').next()))
        .expect("reference without idref or href");
    ids.get(id)
        .unwrap_or_else(|| panic!("unresolved reference {id}"))
        .clone()
}

fn collect_ancestors(name: &str, classes: &BTreeMap<String, Class>, acc: &mut Vec<String>) {
    let Some(class) = classes.get(name) else {
        panic!("unresolved supertype {name}");
    };
    for sup in &class.supers {
        if !acc.contains(sup) {
            acc.push(sup.clone());
            collect_ancestors(sup, classes, acc);
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn generate(
    classes: &BTreeMap<String, Class>,
    enums: &BTreeMap<String, Enum>,
    ancestors: &BTreeMap<String, Vec<String>>,
    rules: &[Rule],
) -> String {
    let mut o = String::new();
    let w = &mut o;

    writeln!(
        w,
        "//! GENERATED by `cargo run -p sysml-model --features codegen \
--bin sysml-codegen`"
    )
    .unwrap();
    writeln!(
        w,
        "//! from the OMG normative metamodel (vendor/metamodel/KerML.xmi +\n\
         //! SysML.xmi). Do not edit by hand."
    )
    .unwrap();
    writeln!(w, "#![allow(clippy::all)]").unwrap();
    writeln!(w).unwrap();

    // --- ElementKind ---
    writeln!(
        w,
        "/// Every metaclass of the KerML/SysML v2 abstract syntax."
    )
    .unwrap();
    writeln!(
        w,
        "#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]"
    )
    .unwrap();
    writeln!(w, "pub enum ElementKind {{").unwrap();
    for name in classes.keys() {
        writeln!(w, "    {name},").unwrap();
    }
    writeln!(w, "}}").unwrap();
    writeln!(w).unwrap();

    writeln!(w, "pub const ELEMENT_KINDS: &[ElementKind] = &[").unwrap();
    for name in classes.keys() {
        writeln!(w, "    ElementKind::{name},").unwrap();
    }
    writeln!(w, "];").unwrap();
    writeln!(w).unwrap();

    writeln!(w, "impl ElementKind {{").unwrap();

    writeln!(w, "    pub fn name(self) -> &'static str {{").unwrap();
    writeln!(w, "        match self {{").unwrap();
    for name in classes.keys() {
        writeln!(w, "            ElementKind::{name} => \"{name}\",").unwrap();
    }
    writeln!(w, "        }}").unwrap();
    writeln!(w, "    }}").unwrap();
    writeln!(w).unwrap();

    writeln!(
        w,
        "    pub fn from_name(name: &str) -> Option<ElementKind> {{"
    )
    .unwrap();
    writeln!(w, "        Some(match name {{").unwrap();
    for name in classes.keys() {
        writeln!(w, "            \"{name}\" => ElementKind::{name},").unwrap();
    }
    writeln!(w, "            _ => return None,").unwrap();
    writeln!(w, "        }})").unwrap();
    writeln!(w, "    }}").unwrap();
    writeln!(w).unwrap();

    writeln!(w, "    pub fn is_abstract(self) -> bool {{").unwrap();
    let abstracts: Vec<_> = classes
        .iter()
        .filter(|(_, c)| c.is_abstract)
        .map(|(n, _)| n)
        .collect();
    writeln!(w, "        matches!(self,").unwrap();
    for (i, name) in abstracts.iter().enumerate() {
        let sep = if i == 0 { "" } else { "|" };
        writeln!(w, "            {sep} ElementKind::{name}").unwrap();
    }
    writeln!(w, "        )").unwrap();
    writeln!(w, "    }}").unwrap();
    writeln!(w).unwrap();

    writeln!(w, "    /// Direct supertypes in the metamodel.").unwrap();
    writeln!(
        w,
        "    pub fn direct_supertypes(self) -> &'static [ElementKind] {{"
    )
    .unwrap();
    writeln!(w, "        match self {{").unwrap();
    for (name, class) in classes {
        let supers: Vec<String> = class
            .supers
            .iter()
            .map(|s| format!("ElementKind::{s}"))
            .collect();
        writeln!(
            w,
            "            ElementKind::{name} => &[{}],",
            supers.join(", ")
        )
        .unwrap();
    }
    writeln!(w, "        }}").unwrap();
    writeln!(w, "    }}").unwrap();
    writeln!(w).unwrap();

    writeln!(w, "    /// All transitive supertypes (excluding `self`).").unwrap();
    writeln!(w, "    pub fn ancestors(self) -> &'static [ElementKind] {{").unwrap();
    writeln!(w, "        match self {{").unwrap();
    for (name, ancs) in ancestors {
        let list: Vec<String> = ancs.iter().map(|s| format!("ElementKind::{s}")).collect();
        writeln!(
            w,
            "            ElementKind::{name} => &[{}],",
            list.join(", ")
        )
        .unwrap();
    }
    writeln!(w, "        }}").unwrap();
    writeln!(w, "    }}").unwrap();
    writeln!(w).unwrap();

    writeln!(
        w,
        "    /// Is this kind the same as, or a specialization of, `ancestor`?"
    )
    .unwrap();
    writeln!(w, "    pub fn is_a(self, ancestor: ElementKind) -> bool {{").unwrap();
    writeln!(
        w,
        "        self == ancestor || self.ancestors().contains(&ancestor)"
    )
    .unwrap();
    writeln!(w, "    }}").unwrap();
    writeln!(w).unwrap();

    writeln!(
        w,
        "    /// Structural features declared directly on this metaclass."
    )
    .unwrap();
    writeln!(
        w,
        "    pub fn own_features(self) -> &'static [FeatureMeta] {{"
    )
    .unwrap();
    writeln!(w, "        match self {{").unwrap();
    for (name, class) in classes {
        writeln!(w, "            ElementKind::{name} => &[").unwrap();
        for f in &class.features {
            let ty = match &f.ty {
                FeatureTy::Data(d) => format!("FeatureType::Data(PrimitiveType::{d})"),
                FeatureTy::Named(n) if enums.contains_key(n) => {
                    format!("FeatureType::Enumeration(EnumType::{n})")
                }
                FeatureTy::Named(n) => format!("FeatureType::Class(ElementKind::{n})"),
            };
            writeln!(
                w,
                "                FeatureMeta {{ name: \"{}\", ty: {ty}, many: {}, derived: {} }},",
                f.name, f.many, f.derived
            )
            .unwrap();
        }
        writeln!(w, "            ],").unwrap();
    }
    writeln!(w, "        }}").unwrap();
    writeln!(w, "    }}").unwrap();
    writeln!(w).unwrap();

    writeln!(
        w,
        "    /// Look up a feature by name on this metaclass or any ancestor."
    )
    .unwrap();
    writeln!(
        w,
        "    pub fn feature(self, name: &str) -> Option<&'static FeatureMeta> {{"
    )
    .unwrap();
    writeln!(
        w,
        "        if let Some(f) = self.own_features().iter().find(|f| f.name == name) {{"
    )
    .unwrap();
    writeln!(w, "            return Some(f);").unwrap();
    writeln!(w, "        }}").unwrap();
    writeln!(w, "        self.ancestors()").unwrap();
    writeln!(w, "            .iter()").unwrap();
    writeln!(
        w,
        "            .find_map(|a| a.own_features().iter().find(|f| f.name == name))"
    )
    .unwrap();
    writeln!(w, "    }}").unwrap();
    writeln!(w, "}}").unwrap();
    writeln!(w).unwrap();

    // --- feature metadata types ---
    writeln!(w, "/// Metadata for one structural feature of a metaclass.").unwrap();
    writeln!(w, "#[derive(Clone, Copy, Debug, PartialEq, Eq)]").unwrap();
    writeln!(w, "pub struct FeatureMeta {{").unwrap();
    writeln!(w, "    pub name: &'static str,").unwrap();
    writeln!(w, "    pub ty: FeatureType,").unwrap();
    writeln!(w, "    pub many: bool,").unwrap();
    writeln!(w, "    pub derived: bool,").unwrap();
    writeln!(w, "}}").unwrap();
    writeln!(w).unwrap();
    writeln!(w, "#[derive(Clone, Copy, Debug, PartialEq, Eq)]").unwrap();
    writeln!(w, "pub enum FeatureType {{").unwrap();
    writeln!(w, "    Data(PrimitiveType),").unwrap();
    writeln!(w, "    Enumeration(EnumType),").unwrap();
    writeln!(w, "    Class(ElementKind),").unwrap();
    writeln!(w, "}}").unwrap();
    writeln!(w).unwrap();
    writeln!(w, "#[derive(Clone, Copy, Debug, PartialEq, Eq)]").unwrap();
    writeln!(
        w,
        "pub enum PrimitiveType {{ Boolean, Integer, Real, String, UnlimitedNatural }}"
    )
    .unwrap();
    writeln!(w).unwrap();
    writeln!(w, "#[derive(Clone, Copy, Debug, PartialEq, Eq)]").unwrap();
    writeln!(w, "pub enum EnumType {{").unwrap();
    for name in enums.keys() {
        writeln!(w, "    {name},").unwrap();
    }
    writeln!(w, "}}").unwrap();
    writeln!(w).unwrap();

    // --- metamodel enums ---
    for (name, e) in enums {
        writeln!(w, "#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]").unwrap();
        writeln!(w, "pub enum {name} {{").unwrap();
        for lit in &e.literals {
            writeln!(w, "    {},", capitalize(lit)).unwrap();
        }
        writeln!(w, "}}").unwrap();
        writeln!(w).unwrap();
        writeln!(w, "impl {name} {{").unwrap();
        writeln!(w, "    pub fn literal(self) -> &'static str {{").unwrap();
        writeln!(w, "        match self {{").unwrap();
        for lit in &e.literals {
            writeln!(w, "            {name}::{} => \"{lit}\",", capitalize(lit)).unwrap();
        }
        writeln!(w, "        }}").unwrap();
        writeln!(w, "    }}").unwrap();
        writeln!(w, "    pub fn from_literal(s: &str) -> Option<{name}> {{").unwrap();
        writeln!(w, "        Some(match s {{").unwrap();
        for lit in &e.literals {
            writeln!(w, "            \"{lit}\" => {name}::{},", capitalize(lit)).unwrap();
        }
        writeln!(w, "            _ => return None,").unwrap();
        writeln!(w, "        }})").unwrap();
        writeln!(w, "    }}").unwrap();
        writeln!(w, "}}").unwrap();
        writeln!(w).unwrap();
    }

    accessors(classes, enums, w);
    constraints(rules, w);

    o
}

/// The specification's own well-formedness constraints, as data.
///
/// The OCL is carried across verbatim rather than compiled here: what
/// evaluates it is `sysml-semantics`, and a rule whose text this file
/// paraphrased would be a rule the specification did not write.
fn constraints(rules: &[Rule], w: &mut String) {
    writeln!(
        w,
        "\n/// One well-formedness constraint of the abstract syntax, as the\n\
         /// specification states it.\n\
         #[derive(Clone, Copy, Debug, PartialEq, Eq)]\n\
         pub struct Rule {{\n\
         \x20   /// The name the specification gives it, e.g. `validateFeatureChainExpressionOperator`.\n\
         \x20   pub name: &'static str,\n\
         \x20   /// The metaclass every instance of which it holds of.\n\
         \x20   pub metaclass: ElementKind,\n\
         \x20   /// The OCL that has to hold, as the metamodel writes it.\n\
         \x20   pub ocl: &'static str,\n\
         \x20   /// What the specification says it means.\n\
         \x20   pub says: &'static str,\n\
         }}\n"
    )
    .unwrap();
    writeln!(
        w,
        "/// Every constraint the KerML and SysML abstract syntax states,\n\
         /// in the order the metamodel states them."
    )
    .unwrap();
    writeln!(w, "pub const RULES: &[Rule] = &[").unwrap();
    for rule in rules {
        writeln!(w, "    Rule {{").unwrap();
        writeln!(w, "        name: {:?},", rule.name).unwrap();
        writeln!(w, "        metaclass: ElementKind::{},", rule.metaclass).unwrap();
        writeln!(w, "        ocl: {:?},", rule.ocl).unwrap();
        writeln!(w, "        says: {:?},", rule.says).unwrap();
        writeln!(w, "    }},").unwrap();
    }
    writeln!(w, "];").unwrap();
}

/// The metamodel's features as typed accessors on `Model`, one per
/// distinct feature name: `model.declared_name(id)` instead of
/// `model.get(id, "declaredName")?.as_str()`, and the property name
/// spelled once here rather than at every call.
///
/// A name declared with different multiplicities by different metaclasses
/// takes the slice form, which answers for both. A name that would
/// collide with one of `Model`'s own methods is left out and said so, so
/// that the hand-written meaning wins.
fn accessors(classes: &BTreeMap<String, Class>, enums: &BTreeMap<String, Enum>, w: &mut String) {
    /// `Model`'s hand-written inherent methods, which a generated
    /// accessor must not shadow.
    const TAKEN: &[&str] = &[
        "add_owned",
        "create",
        "descendants",
        "get",
        "ids",
        "is_empty",
        "kind",
        "len",
        "member_role",
        "member_visibility",
        "name",
        "new",
        "owned",
        "owner",
        "props",
        "set",
        "set_member_role",
        "set_member_visibility",
    ];

    // by feature name: the shapes it is declared with, and one metaclass
    // that declares it (for the documentation)
    let mut features: BTreeMap<&str, Declared<'_>> = BTreeMap::new();
    for (class_name, class) in classes {
        for f in &class.features {
            let shape = match &f.ty {
                FeatureTy::Data(d) => (*d).to_string(),
                FeatureTy::Named(n) if enums.contains_key(n) => "Enumeration".to_string(),
                FeatureTy::Named(_) => "Class".to_string(),
            };
            let declared = features
                .entry(f.name.as_str())
                .or_insert_with(|| (BTreeSet::new(), class_name.as_str()));
            // The generated test writes each value through the metaclass
            // named here, and the model checks it against that
            // metaclass's own declaration. Where one class declares the
            // feature as a list and another as a single value, name one
            // that declares the list -- it accepts both readings.
            if f.many && !declared.0.iter().any(|(_, many)| *many) {
                declared.1 = class_name.as_str();
            }
            declared.0.insert((shape, f.many));
        }
    }

    writeln!(w, "use crate::{{ElementId, Model, Value}};").unwrap();
    writeln!(w).unwrap();
    writeln!(w, "impl Model {{").unwrap();
    let mut written: Vec<Written<'_>> = Vec::new();
    for (name, (shapes, declared_by)) in &features {
        let method = snake(name);
        if TAKEN.contains(&method.as_str()) {
            writeln!(
                w,
                "    // `{name}` is left to `Model::{method}`, which is written by hand"
            )
            .unwrap();
            continue;
        }
        let kinds: BTreeSet<&str> = shapes.iter().map(|(kind, _)| kind.as_str()).collect();
        let many = shapes.iter().any(|(_, many)| *many);
        // one name, one type: a feature declared as two different things
        // has no one accessor to be
        if kinds.len() != 1 {
            writeln!(
                w,
                "    // `{name}` is declared as {} different types; no one accessor fits",
                kinds.len()
            )
            .unwrap();
            continue;
        }
        let kind = *kinds.iter().next().expect("just counted one");
        // a feature named `type` is still a feature; Rust just needs
        // telling that it is a name here
        let method = escape(&method);
        writeln!(w, "    /// `{name}`, as {declared_by} declares it.").unwrap();
        match (kind, many) {
            ("Class", false) => {
                writeln!(
                    w,
                    "    pub fn {method}(&self, id: ElementId) -> Option<ElementId> {{"
                )
                .unwrap();
                writeln!(
                    w,
                    "        match self.get(id, \"{name}\") {{ Some(Value::Ref(to)) => Some(*to), _ => None }}"
                )
                .unwrap();
            }
            ("Class", true) => {
                writeln!(
                    w,
                    "    pub fn {method}(&self, id: ElementId) -> &[ElementId] {{"
                )
                .unwrap();
                // the singular declaration of the same name answers as a
                // slice of one, so both readings hold
                writeln!(w, "        match self.get(id, \"{name}\") {{").unwrap();
                writeln!(w, "            Some(Value::RefList(list)) => list,").unwrap();
                writeln!(
                    w,
                    "            Some(Value::Ref(to)) => std::slice::from_ref(to),"
                )
                .unwrap();
                writeln!(w, "            _ => &[],").unwrap();
                writeln!(w, "        }}").unwrap();
            }
            ("Boolean", _) => {
                writeln!(w, "    pub fn {method}(&self, id: ElementId) -> bool {{").unwrap();
                writeln!(
                    w,
                    "        matches!(self.get(id, \"{name}\"), Some(Value::Bool(true)))"
                )
                .unwrap();
            }
            ("Integer" | "UnlimitedNatural", _) => {
                writeln!(
                    w,
                    "    pub fn {method}(&self, id: ElementId) -> Option<i64> {{"
                )
                .unwrap();
                writeln!(
                    w,
                    "        match self.get(id, \"{name}\") {{ Some(Value::Int(n)) => Some(*n), _ => None }}"
                )
                .unwrap();
            }
            ("Real", _) => {
                writeln!(
                    w,
                    "    pub fn {method}(&self, id: ElementId) -> Option<f64> {{"
                )
                .unwrap();
                writeln!(
                    w,
                    "        match self.get(id, \"{name}\") {{ Some(Value::Real(x)) => Some(*x), _ => None }}"
                )
                .unwrap();
            }
            // a string or an enumeration literal reads the same way, and
            // the model holds no list of either
            _ => {
                writeln!(
                    w,
                    "    pub fn {method}(&self, id: ElementId) -> Option<&str> {{"
                )
                .unwrap();
                writeln!(w, "        self.get(id, \"{name}\")?.as_str()").unwrap();
            }
        }
        writeln!(w, "    }}").unwrap();
        written.push(Written {
            method,
            feature: name,
            declared_by,
            kind,
            many,
        });
    }
    writeln!(w, "}}").unwrap();
    writeln!(w).unwrap();

    // Every accessor is answered for twice: once by an element that
    // declares nothing, and once by one carrying the value it reads.
    // That keeps generated code as executed as it is written, which is
    // what this repository gates on.
    writeln!(w, "#[cfg(test)]").unwrap();
    writeln!(w, "mod accessor_tests {{").unwrap();
    writeln!(w, "    use super::*;").unwrap();
    writeln!(w).unwrap();
    writeln!(w, "    #[test]").unwrap();
    writeln!(w, "    fn every_accessor_answers_for_an_empty_element() {{").unwrap();
    writeln!(w, "        let mut model = Model::new();").unwrap();
    writeln!(w, "        let id = model.create(ElementKind::Namespace);").unwrap();
    for entry in &written {
        writeln!(w, "        let _ = model.{}(id);", entry.method).unwrap();
    }
    writeln!(w, "    }}").unwrap();
    writeln!(w).unwrap();
    writeln!(w, "    #[test]").unwrap();
    writeln!(w, "    fn every_accessor_reads_back_what_was_set() {{").unwrap();
    writeln!(w, "        let mut model = Model::new();").unwrap();
    writeln!(
        w,
        "        let other = model.create(ElementKind::Namespace);"
    )
    .unwrap();
    for entry in &written {
        let (feature, declared_by, method) = (entry.feature, entry.declared_by, &entry.method);
        writeln!(
            w,
            "        let id = model.create(ElementKind::{declared_by});"
        )
        .unwrap();
        let value = match entry.kind {
            "Class" => "Value::Ref(other)".to_string(),
            "Boolean" => "Value::Bool(true)".to_string(),
            "Integer" | "UnlimitedNatural" => "Value::Int(1)".to_string(),
            "Real" => "Value::Real(1.0)".to_string(),
            // an enumerated property holds the literal it names, which
            // reads back as a string but is not one
            "Enumeration" => "Value::EnumLit(\"x\")".to_string(),
            _ => format!("Value::String(String::from({}))", "\"x\""),
        };
        if entry.kind == "Class" && entry.many {
            // a name declared each way is read both ways
            writeln!(
                w,
                "        model.set(id, \"{feature}\", Value::RefList(vec![other]));"
            )
            .unwrap();
            writeln!(w, "        let _ = model.{method}(id);").unwrap();
        }
        writeln!(w, "        model.set(id, \"{feature}\", {value});").unwrap();
        writeln!(w, "        let _ = model.{method}(id);").unwrap();
    }
    writeln!(w, "    }}").unwrap();
    writeln!(w, "}}").unwrap();
}

/// One generated accessor, and what a test needs in order to exercise it.
struct Written<'a> {
    method: String,
    feature: &'a str,
    declared_by: &'a str,
    kind: &'a str,
    many: bool,
}

/// How one feature name is declared across the metaclasses: every
/// (type, multiplicity) it appears with, and one metaclass declaring it.
type Declared<'a> = (BTreeSet<(String, bool)>, &'a str);

/// Out of the way of the words Rust has taken. The metamodel names
/// nothing `self`, `crate` or `super`, which are the words no `r#` can
/// rescue, so a raw identifier always serves.
fn escape(name: &str) -> String {
    const RESERVED: [&str; 47] = [
        "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "do",
        "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "if", "impl", "in", "let",
        "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref", "return",
        "static", "struct", "trait", "true", "try", "type", "typeof", "unsafe", "unsized", "use",
        "virtual", "where", "while", "yield",
    ];
    if RESERVED.contains(&name) {
        format!("r#{name}")
    } else {
        name.to_string()
    }
}

/// `declaredName` -> `declared_name`.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (at, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if at > 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

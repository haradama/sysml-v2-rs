//! Generates `crates/sysml-model/src/generated.rs` from the normative
//! machine-readable metamodel published by the OMG with the KerML 1.0 and
//! SysML 2.0 specifications (`vendor/metamodel/KerML.xmi` and
//! `vendor/metamodel/SysML.xmi`).
//!
//! Run with `cargo run -p sysml-codegen`. The output is committed, so this
//! only needs to run again when the vendored metamodel is updated; a test
//! keeps the committed file in sync with the vendored metamodel.

use std::collections::BTreeMap;
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

    // transitive ancestors (excluding self), name-sorted for determinism
    let mut ancestors: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for name in classes.keys() {
        let mut acc = Vec::new();
        collect_ancestors(name, &classes, &mut acc);
        acc.sort();
        acc.dedup();
        ancestors.insert(name.clone(), acc);
    }

    generate(&classes, &enums, &ancestors)
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
    let many = attr
        .children()
        .find(|c| c.has_tag_name("upperValue"))
        .and_then(|u| u.attribute("value"))
        .is_some_and(|v| v == "-1" || v == "*");
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
) -> String {
    let mut o = String::new();
    let w = &mut o;

    writeln!(
        w,
        "//! GENERATED by `cargo run -p sysml-codegen` from the OMG normative"
    )
    .unwrap();
    writeln!(
        w,
        "//! metamodel (vendor/metamodel/KerML.xmi + SysML.xmi). Do not edit by hand."
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

    o
}

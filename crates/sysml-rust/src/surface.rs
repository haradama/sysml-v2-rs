//! An existing API surface, stated in no language in particular, and the
//! SysML package it is written as.
//!
//! A [`Surface`] is what a tool that reads some language's code has to
//! say about it: the records, enumerations, interfaces and functions,
//! each with the name the code gives it and the path to it.
//! [`to_sysml`] is the one writer that turns any of them into a package
//! whose every definition carries the `@code` binding [`crate::binding`]
//! spells -- one writer rather than one per language, because it owns
//! everything a second importer would get subtly wrong: a field named
//! `filter` quoted because it is a reserved word, two members that want
//! one name told apart, a `*/` in a doc kept from ending it, a signature
//! naming a type nothing declares refused rather than written.
//!
//! ```
//! use sysml_rust::surface::{Field, Item, Record, Surface};
//!
//! let surface = Surface {
//!     language: "python".into(),
//!     module: "inventory".into(),
//!     items: vec![Item::Record(Record {
//!         name: "StockQuery".into(),
//!         path: "inventory.StockQuery".into(),
//!         fields: vec![Field {
//!             name: "sku".into(),
//!             of: "String".into(),
//!             ..Field::default()
//!         }],
//!         ..Record::default()
//!     })],
//!     ..Surface::default()
//! };
//! let imported = sysml_rust::surface::to_sysml(&surface, None);
//! assert!(imported.sysml.contains("package InventoryApi {"));
//! assert!(imported.sysml.contains("attribute sku : String;"));
//! assert!(imported.sysml.contains(":>> writtenIn = \"python\";"));
//! ```

use std::collections::HashSet;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::binding;

/// What a tool read off some language's code: the part of it a model
/// can bind to.
///
/// Every string in it is spelled the way the language spells it. The
/// writer decides how each becomes SysML -- which names are quoted,
/// which are CamelCased, which are told apart -- so that two front ends
/// never decide differently.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Surface {
    /// The language every binding is written in: `rust`, `python`. A
    /// generator for one language takes a binding in its own language as
    /// something to call, and one in any other as something that merely
    /// exists.
    pub language: String,
    /// The unit the surface is of -- a crate, a package, a module, by
    /// whatever name the language gives one -- spelled as the language
    /// spells it. Every binding carries it, and the package is named
    /// after it where nothing else names the package.
    pub module: String,
    /// What the language calls such a unit, for the comment at the top
    /// of the package: `crate`, `package`. Left out, `module`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// What wrote this surface, named at the top of the package so that
    /// a reader knows what to run again. Left out, `sysml import-code`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// The name of the package to write. Left out, the module's name in
    /// CamelCase with `Api` after it: `inventory_store` becomes
    /// `InventoryStoreApi`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// The language's own scalars, each declared as a kind of the SysML
    /// one it is nearest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scalars: Vec<Scalar>,
    /// What the unit declares, in the order it declares it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Item>,
    /// What the tool could not state, a line each, so that what is left
    /// to model by hand is listed rather than missing. The package says
    /// them in comments at its end.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<String>,
}

/// The SysML scalars a generator reads back as a language's own.
///
/// Anything a signature is typed with bottoms out in one of these: as
/// itself, or through a [`Scalar`] declared as a kind of one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Primitive {
    /// `true` or `false`.
    Boolean,
    /// Text.
    String,
    /// A number with a fractional part.
    Real,
    /// A whole number.
    Integer,
    /// A whole number that is not negative.
    Natural,
}

impl Primitive {
    /// Every one of them, in the order the library declares them.
    pub const ALL: [Primitive; 5] = [
        Primitive::Boolean,
        Primitive::String,
        Primitive::Real,
        Primitive::Integer,
        Primitive::Natural,
    ];

    /// The name the standard library declares it under.
    pub fn name(self) -> &'static str {
        match self {
            Primitive::Boolean => "Boolean",
            Primitive::String => "String",
            Primitive::Real => "Real",
            Primitive::Integer => "Integer",
            Primitive::Natural => "Natural",
        }
    }

    /// The one that answers to a name, if any does.
    pub fn named(name: &str) -> Option<Primitive> {
        Primitive::ALL.into_iter().find(|it| it.name() == name)
    }
}

/// A scalar of the language that no SysML one stands for exactly.
///
/// The five [`Primitive`]s come back out of a generator as the
/// language's nearest type -- a `Natural` as a `u64`, say. Anything
/// narrower is declared as a kind of one of them, so that a `u8` returns
/// from the round trip as a `u8` and not as the `u64` it would otherwise
/// have read back as.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Scalar {
    /// The name the package declares it under: `RustU8`.
    pub name: String,
    /// The SysML scalar it is a kind of.
    pub of: Primitive,
    /// The type in the language: `u8`.
    pub path: String,
    /// What the type can already do, by the names the language gives
    /// such things -- traits, interfaces, protocols. A generator writes
    /// nothing about a type it did not write unless the binding says
    /// what it can do.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

/// One thing the unit declares.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Item {
    /// A type with named fields: a struct, a class, a dataclass. Written
    /// as an `item def`, each field an attribute.
    Record(Record),
    /// A type that is one of a fixed set of names. Written as an
    /// `enum def`.
    Enumeration(Enumeration),
    /// A set of operations something provides: a trait, a protocol, an
    /// abstract class. Written as a `port def`, with each method an
    /// `action def` beside it.
    Interface(Interface),
    /// A callable that belongs to no type. Written as an `action def`.
    Function(Callable),
}

impl Item {
    /// The name a signature may refer to this by, where it declares a
    /// type. A function declares none.
    fn type_name(&self) -> Option<&str> {
        match self {
            Item::Record(record) => Some(&record.name),
            Item::Enumeration(enumeration) => Some(&enumeration.name),
            Item::Interface(interface) => Some(&interface.name),
            Item::Function(_) => None,
        }
    }
}

/// A type with named fields.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Record {
    /// The name the code declares it under, kept as the definition's:
    /// a signature refers to it.
    pub name: String,
    /// The path to it in the language: `inventory_store::StockQuery`,
    /// `inventory.StockQuery`.
    pub path: String,
    /// What the code says it is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    /// What the type can already do, by the names the language gives
    /// such things.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    /// Its fields, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Field>,
}

/// A type that is one of a fixed set of names.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Enumeration {
    /// The name the code declares it under, kept as the definition's.
    pub name: String,
    /// The path to it in the language.
    pub path: String,
    /// What the code says it is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    /// What the type can already do, by the names the language gives
    /// such things.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    /// The names it may be, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
}

/// A set of operations something provides.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Interface {
    /// The name the code declares it under, kept as the definition's.
    pub name: String,
    /// The path to it in the language.
    pub path: String,
    /// What the code says it is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    /// What the type can already do, by the names the language gives
    /// such things.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    /// Its operations, in declaration order. Each is written as an
    /// `action def` beside the port, named after the operation -- with
    /// the interface's name in front where another member has taken it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<Callable>,
}

/// Something that can be called: a function, or a method of an
/// [`Interface`].
///
/// Written as an `action def` named after it in CamelCase --
/// `get_stock` becomes `GetStock` -- with an `in` parameter for each
/// parameter, an `out result` for what it returns and an `out error`
/// for what it fails with.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Callable {
    /// The name the code declares it under.
    pub name: String,
    /// The path to it in the language:
    /// `inventory_store::InventoryStore::get_stock`.
    pub path: String,
    /// What the code says it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    /// How it takes the thing it is called on, spelled as the language
    /// spells it -- `&self`, `&mut self`, `self` in Rust -- and empty
    /// for a function that is called on nothing.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub receiver: String,
    /// Whether calling it is asynchronous.
    #[serde(default, rename = "async", skip_serializing_if = "std::ops::Not::not")]
    pub is_async: bool,
    /// Whether it can fail in the way its language has of failing -- a
    /// `Result`, an exception, an error return -- whether or not
    /// [`error`](Callable::error) says with what.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fallible: bool,
    /// What it takes, in order, not counting the receiver.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<Field>,
    /// What it hands back, where it hands anything back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<TypeRef>,
    /// What it fails with, where that is a type of its own: the name of
    /// a type this surface declares or a scalar. At most one of it is
    /// ever handed back, so it carries no multiplicity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A name with a type: a field of a [`Record`], or a parameter of a
/// [`Callable`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Field {
    /// The name the code declares it under, kept as the attribute's --
    /// quoted where it is one of SysML's reserved words.
    pub name: String,
    /// What the code says it is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    /// What it is typed with: one of the [`Primitive`]s by name, a
    /// [`Scalar`] this surface declares, or a type this surface
    /// declares. Anything else has no place in the package, and the
    /// field -- or the whole callable -- is left out and listed.
    #[serde(rename = "type")]
    pub of: String,
    /// How many of it there are.
    #[serde(default, skip_serializing_if = "Multiplicity::is_one")]
    pub multiplicity: Multiplicity,
}

/// A type with a multiplicity and no name: what a [`Callable`] hands
/// back.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TypeRef {
    /// What it is typed with, under the same rule as a
    /// [`Field`](Field::of).
    #[serde(rename = "type")]
    pub of: String,
    /// How many of it there are.
    #[serde(default, skip_serializing_if = "Multiplicity::is_one")]
    pub multiplicity: Multiplicity,
}

/// How many of a type a field or parameter holds.
///
/// A container becomes a multiplicity on the element type, and a
/// multiplicity is said once: a container holding another -- a list of
/// lists, an optional list -- has no second place to say it in, and
/// folding the two into one would claim the code takes something it
/// does not. A front end states such a field as skipped rather than
/// choosing one of the two.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Multiplicity {
    /// Exactly one, which is what nothing written means.
    #[default]
    One,
    /// One or none: `Option<T>`, `T | None`. Written `[0..1]`.
    Optional,
    /// Any number: `Vec<T>`, `list[T]`. Written `[*]`.
    Many,
}

impl Multiplicity {
    /// Whether it is the one nothing written means.
    pub fn is_one(&self) -> bool {
        *self == Multiplicity::One
    }

    /// The notation, after the type it is of.
    pub fn written(self) -> &'static str {
        match self {
            Multiplicity::One => "",
            Multiplicity::Optional => "[0..1]",
            Multiplicity::Many => "[*]",
        }
    }
}

/// What is wrong with text that was meant to be a surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceError {
    /// The text is not JSON at all.
    NotJson(String),
    /// The JSON is not a surface: a field it does not have, a word it
    /// does not use, a value of the wrong shape -- named, so that
    /// whoever wrote the front end sees which.
    NotASurface(String),
}

impl std::fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SurfaceError::NotJson(why) => write!(f, "the input is not JSON: {why}"),
            SurfaceError::NotASurface(why) => write!(f, "the input is not a surface: {why}"),
        }
    }
}

impl std::error::Error for SurfaceError {}

/// A surface, read from its JSON.
///
/// Strict: a field the surface has no name for is refused rather than
/// ignored, since a front end that misspells `multiplicity` would
/// otherwise write every list as one thing and be told nothing.
pub fn read(json: &str) -> Result<Surface, SurfaceError> {
    serde_json::from_str(json).map_err(|err| match err.classify() {
        serde_json::error::Category::Data => SurfaceError::NotASurface(err.to_string()),
        _ => SurfaceError::NotJson(err.to_string()),
    })
}

/// The JSON schema of a [`Surface`], for whoever has to write one.
#[cfg(feature = "schema")]
pub fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Surface)).expect("a schema is JSON")
}

/// A surface as SysML, and what of it had no place there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Imported {
    /// The package.
    pub sysml: String,
    /// Every item that could not be stated, and why: what the front end
    /// listed, and after it what the writer refused -- a signature
    /// naming a type the surface never declared, a second type under a
    /// name already taken. The package says them in comments too; this
    /// is for whoever has to model them by hand, and a list to work
    /// through is a poor thing to have to find by reading.
    pub skipped: Vec<String>,
}

/// The SysML package for a surface.
///
/// `package` names the package written; left out, the surface names it,
/// and failing that its module does. Deterministic: the same surface
/// writes the same bytes.
///
/// Every name a signature uses is held to what the package declares:
/// a field typed with something the surface never declared is left out
/// and listed, and a callable with one is left out whole, so that every
/// name in the package resolves. A second type under a name the package
/// already has -- or under one of the scalars it imports -- is refused
/// the same way, since a signature naming it would mean the first.
pub fn to_sysml(surface: &Surface, package: Option<&str>) -> Imported {
    let package_name = package
        .map(String::from)
        .or_else(|| surface.package.clone())
        .unwrap_or_else(|| format!("{}Api", camel(&surface.module)));
    let mut out = String::new();
    writeln!(
        out,
        "// generated from {} `{}` by `{}` -- DO NOT EDIT",
        surface.unit.as_deref().unwrap_or("module"),
        surface.module,
        surface.tool.as_deref().unwrap_or("sysml import-code")
    )
    .expect("writing to a String cannot fail");
    writeln!(out, "package {} {{", quoted(&package_name)).unwrap();
    writeln!(out, "\tprivate import ScalarValues::*;\n").unwrap();
    writeln!(
        out,
        "\t// how each definition binds to the code it came from"
    )
    .unwrap();
    writeln!(out, "\tmetadata def {} {{", binding::DEF).unwrap();
    for (name, ty) in binding::ALL {
        writeln!(out, "\t\tattribute {name} : {ty};").unwrap();
    }
    writeln!(out, "\t}}").unwrap();

    // Which names a signature may use, settled before anything is
    // written: a callable written above a type that is then refused
    // would name something nothing answers to.
    let mut skipped = surface.skipped.clone();
    let mut known: HashSet<String> = Primitive::ALL
        .iter()
        .map(|it| it.name().to_string())
        .collect();
    let scalars: Vec<&Scalar> = surface
        .scalars
        .iter()
        .filter(|scalar| claim(&mut known, &scalar.name, &mut skipped))
        .collect();
    let accepted: Vec<bool> = surface
        .items
        .iter()
        .map(|item| match item.type_name() {
            Some(name) => claim(&mut known, name, &mut skipped),
            None => true,
        })
        .collect();

    let mut writer = Writer {
        language: &surface.language,
        module: &surface.module,
        taken: known.clone(),
        known,
        out,
        skipped,
    };
    if !scalars.is_empty() {
        writeln!(
            writer.out,
            "\n\t// the language's own scalars, each a kind of the SysML one it is nearest"
        )
        .unwrap();
        for scalar in scalars {
            writer.scalar(scalar);
        }
    }
    for (item, &write) in surface.items.iter().zip(&accepted) {
        match item {
            Item::Record(record) if write => writer.record(record),
            Item::Enumeration(enumeration) if write => writer.enumeration(enumeration),
            Item::Interface(interface) if write => writer.interface(interface),
            Item::Function(function) => writer.callable(function, None),
            _ => {}
        }
    }
    let Writer {
        mut out, skipped, ..
    } = writer;
    if !skipped.is_empty() {
        writeln!(out, "\n\t// not imported (no monomorphic SysML shape):").unwrap();
        for line in &skipped {
            writeln!(out, "\t//   {line}").unwrap();
        }
    }
    writeln!(out, "}}").unwrap();
    Imported {
        sysml: out,
        skipped,
    }
}

/// Take a name for the package to declare, or say why it cannot be.
fn claim(known: &mut HashSet<String>, name: &str, skipped: &mut Vec<String>) -> bool {
    if Primitive::named(name).is_some() {
        skipped.push(format!(
            "{name} -- the name of a scalar the package imports"
        ));
        return false;
    }
    if !known.insert(name.to_string()) {
        skipped.push(format!("{name} -- declared twice; the first is kept"));
        return false;
    }
    true
}

/// What every emitter needs at hand.
struct Writer<'a> {
    language: &'a str,
    module: &'a str,
    /// The names a signature may be typed with.
    known: HashSet<String>,
    /// Every name the package declares, so that the second member to
    /// want one is renamed rather than left to shadow the first.
    taken: HashSet<String>,
    out: String,
    skipped: Vec<String>,
}

impl Writer<'_> {
    /// `attribute def RustU8 :> Natural { @code { ... } }`.
    fn scalar(&mut self, scalar: &Scalar) {
        write!(
            self.out,
            "\tattribute def {} :> {} {{ @{} {{ :>> {} = {}; :>> {} = {};",
            quoted(&scalar.name),
            scalar.of.name(),
            binding::DEF,
            binding::LANGUAGE,
            literal(self.language),
            binding::ITEM,
            literal(&scalar.path)
        )
        .unwrap();
        if !scalar.capabilities.is_empty() {
            write!(
                self.out,
                " :>> {} = {};",
                binding::CAPABILITIES,
                literal(&scalar.capabilities.join(", "))
            )
            .unwrap();
        }
        writeln!(self.out, " }} }}").unwrap();
    }

    /// A record -> `item def`, each field an attribute.
    fn record(&mut self, record: &Record) {
        docs(&mut self.out, record.doc.as_deref(), 1, None);
        writeln!(self.out, "\titem def {} {{", quoted(&record.name)).unwrap();
        self.binding(&record.path, &record.capabilities, None, 2);
        for field in &record.fields {
            match self.typed(&field.of, field.multiplicity) {
                Some(ty) => {
                    docs(&mut self.out, field.doc.as_deref(), 2, None);
                    writeln!(self.out, "\t\tattribute {} : {ty};", quoted(&field.name)).unwrap();
                }
                None => self.skipped.push(format!(
                    "{}.{} -- names `{}`, which the surface does not declare",
                    record.name, field.name, field.of
                )),
            }
        }
        writeln!(self.out, "\t}}").unwrap();
    }

    /// An enumeration -> `enum def`.
    fn enumeration(&mut self, enumeration: &Enumeration) {
        docs(&mut self.out, enumeration.doc.as_deref(), 1, None);
        writeln!(self.out, "\tenum def {} {{", quoted(&enumeration.name)).unwrap();
        self.binding(&enumeration.path, &enumeration.capabilities, None, 2);
        for value in &enumeration.values {
            writeln!(self.out, "\t\tenum {};", quoted(value)).unwrap();
        }
        writeln!(self.out, "\t}}").unwrap();
    }

    /// An interface -> `port def`, each method an `action def` beside it.
    fn interface(&mut self, interface: &Interface) {
        docs(&mut self.out, interface.doc.as_deref(), 1, None);
        writeln!(self.out, "\tport def {} {{", quoted(&interface.name)).unwrap();
        self.binding(&interface.path, &interface.capabilities, None, 2);
        writeln!(self.out, "\t}}").unwrap();
        for method in &interface.methods {
            self.callable(method, Some(&interface.name));
        }
    }

    /// A callable -> `action def` with `in`/`out` parameters, or nothing
    /// at all where its signature names what the surface does not
    /// declare.
    fn callable(&mut self, callable: &Callable, owner: Option<&str>) {
        let mut lines = Vec::new();
        for parameter in &callable.parameters {
            let Some(ty) = self.typed(&parameter.of, parameter.multiplicity) else {
                self.refuse(callable, &parameter.of);
                return;
            };
            if let Some(doc) = &parameter.doc {
                lines.push(format!("doc /* {} */", doc_text(doc)));
            }
            lines.push(format!("in {} : {ty};", quoted(&parameter.name)));
        }
        if let Some(result) = &callable.result {
            let Some(ty) = self.typed(&result.of, result.multiplicity) else {
                self.refuse(callable, &result.of);
                return;
            };
            lines.push(format!("out result : {ty};"));
        }
        if let Some(error) = &callable.error {
            let Some(ty) = self.typed(error, Multiplicity::Optional) else {
                self.refuse(callable, error);
                return;
            };
            lines.push(format!("out error : {ty};"));
        }

        let wanted = camel(&callable.name);
        let called = unclaimed(&mut self.taken, &wanted, owner);
        let renamed = (called != wanted).then(|| {
            format!(
                "`{}` is imported as `{called}`: `{wanted}` is another member's name.",
                callable.path
            )
        });
        docs(
            &mut self.out,
            callable.doc.as_deref(),
            1,
            renamed.as_deref(),
        );
        writeln!(self.out, "\taction def {} {{", quoted(&called)).unwrap();
        self.binding(&callable.path, &[], Some(callable), 2);
        for line in lines {
            writeln!(self.out, "\t\t{line}").unwrap();
        }
        writeln!(self.out, "\t}}").unwrap();
    }

    /// Say that a callable is left out for naming `what`.
    fn refuse(&mut self, callable: &Callable, what: &str) {
        self.skipped.push(format!(
            "{} -- names `{what}`, which the surface does not declare",
            callable.path
        ));
    }

    /// The `@code { ... }` usage binding one definition to what it
    /// stands for.
    fn binding(
        &mut self,
        path: &str,
        capabilities: &[String],
        callable: Option<&Callable>,
        indent: usize,
    ) {
        let tabs = "\t".repeat(indent);
        write!(
            self.out,
            "{tabs}@{} {{ :>> {} = {}; :>> {} = {};",
            binding::DEF,
            binding::LANGUAGE,
            literal(self.language),
            binding::ITEM,
            literal(path)
        )
        .unwrap();
        if !self.module.is_empty() {
            write!(
                self.out,
                " :>> {} = {};",
                binding::MODULE,
                literal(self.module)
            )
            .unwrap();
        }
        if !capabilities.is_empty() {
            write!(
                self.out,
                " :>> {} = {};",
                binding::CAPABILITIES,
                literal(&capabilities.join(", "))
            )
            .unwrap();
        }
        if let Some(callable) = callable {
            write!(
                self.out,
                " :>> {} = {}; :>> {} = {}; :>> {} = {};",
                binding::TAKES_SELF,
                literal(&callable.receiver),
                binding::IS_ASYNC,
                callable.is_async,
                binding::IS_FALLIBLE,
                callable.fallible
            )
            .unwrap();
        }
        writeln!(self.out, " }}").unwrap();
    }

    /// A type under a multiplicity, as a signature writes it -- or
    /// nothing, where the surface never declared the type.
    fn typed(&self, of: &str, multiplicity: Multiplicity) -> Option<String> {
        self.known
            .contains(of)
            .then(|| format!("{}{}", quoted(of), multiplicity.written()))
    }
}

/// The name to write a member under, given what the package has taken
/// already. A method that cannot have its own name reads best under its
/// interface's -- `Session::label` as `SessionLabel` -- and a function,
/// having no interface to be told apart by, is numbered.
fn unclaimed(taken: &mut HashSet<String>, wanted: &str, owner: Option<&str>) -> String {
    let base = match owner {
        Some(interface) if taken.contains(wanted) => format!("{}{wanted}", camel(interface)),
        _ => wanted.to_string(),
    };
    let mut name = base.clone();
    let mut at = 1;
    while !taken.insert(name.clone()) {
        at += 1;
        name = format!("{base}{at}");
    }
    name
}

/// A doc comment as a SysML `doc` block, with whatever the writer had to
/// say about the item after it.
fn docs(out: &mut String, doc: Option<&str>, indent: usize, note: Option<&str>) {
    let said = [
        doc_text(doc.unwrap_or_default()),
        note.unwrap_or_default().to_string(),
    ]
    .join(" ")
    .trim()
    .to_string();
    if said.is_empty() {
        return;
    }
    writeln!(out, "{}doc /* {said} */", "\t".repeat(indent)).unwrap();
}

/// Text as it can sit inside `/* ... */`: on one line, and with no `*/`
/// in it to end the block early.
fn doc_text(text: &str) -> String {
    text.replace("*/", "*\\/").trim().replace('\n', " ")
}

/// Text as a SysML string literal.
fn literal(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A name from the code as a SysML name: as it is, quoted when it is one
/// of the reserved words.
fn quoted(name: &str) -> String {
    if sysml_syntax::SyntaxKind::from_keyword(name).is_some() {
        format!("'{name}'")
    } else {
        name.to_string()
    }
}

/// `get_stock` -> `GetStock`, for the actions a callable becomes.
fn camel(name: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for ch in name.chars() {
        if ch == '_' || ch == '-' {
            upper = true;
        } else if upper {
            out.extend(ch.to_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

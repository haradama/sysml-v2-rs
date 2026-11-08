//! An existing Rust API surface, imported as a SysML package.
//!
//! The input is what `rustdoc` writes with `--output-format json`: the one
//! machine-readable statement of a crate's public API, re-exports resolved
//! and macros expanded. [`rustdoc_to_surface`] reads it as a
//! [`Surface`] -- what any language's reader says about its code -- and
//! [`rustdoc_to_sysml`] writes that as a package in which every definition
//! carries a `@code { ... }` metadata usage naming the Rust item it binds
//! to, so a model can `import` the package, type its ports and `perform`
//! its actions, and a generator can call the real API instead of
//! inventing parallel types. Only the reading is Rust's own: what a
//! surface becomes in SysML is decided in [`crate::surface`], once, for
//! every language.
//!
//! The mapping, deliberately monomorphic:
//!
//! - `struct` -> record, each field a typed name; `Vec<T>` becomes `many`,
//!   `Option<T>` becomes `optional`, `Box`/`Arc`/`Rc` vanish
//! - plain `enum` -> enumeration
//! - `trait` -> interface, each method a callable with its parameters,
//!   its result and -- for `Result` returns -- its error
//! - free `fn` -> function
//! - generic items, and anything else, are skipped and listed rather
//!   than dropped silently
//!
//! A scalar keeps the width the crate declared: `sysml rustgen` reads a
//! `Natural` back as a `u64`, so a `u32` folded into `Natural` returns
//! from the round trip as a type the crate's own functions refuse. Every
//! other Rust scalar gets a scalar of its own. For the same reason a
//! borrow reads as a value only for `&str`, which is how Rust passes a
//! string.
//!
//! A container nested in a container -- `Option<Vec<u8>>` -- is refused:
//! SysML says how many of a type there are once, and folding the two
//! levels into one would say something the crate does not.
//!
//! A `pub use` re-export is followed to the item it names. Nothing skipped
//! is ever named by a signature: a definition whose parameters or result
//! have no shape is skipped in turn, so every name resolves.
//!
//! rustdoc's JSON is versioned, and this reads the format that spells a
//! function's parameters as `sig`; an older document is refused rather
//! than imported with no parameters at all.
//!
//! Regenerate an input with:
//!
//! ```sh
//! cargo +nightly rustdoc -- -Zunstable-options --output-format json
//! ```

use std::collections::HashSet;

use crate::surface::{
    Callable, Enumeration, Field, Imported, Interface, Item, Multiplicity, Primitive, Record,
    Scalar, Surface, TypeRef,
};
use serde_json::Value as Json;

/// What can be wrong with a rustdoc JSON input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    /// The text is not JSON at all.
    NotJson(String),
    /// The JSON has no piece this importer needs.
    Malformed(String),
    /// The JSON is of a rustdoc format older than the one this importer
    /// reads, carrying the version it says it is.
    OldFormat(u64),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::NotJson(why) => write!(f, "the input is not JSON: {why}"),
            ImportError::Malformed(what) => write!(f, "the input is missing {what}"),
            ImportError::OldFormat(version) => write!(
                f,
                "the input is rustdoc JSON format {version}, which spells a function's \
                 parameters as `decl`; this importer reads `sig`. Regenerate it with a \
                 current toolchain"
            ),
        }
    }
}

impl std::error::Error for ImportError {}

/// The SysML package for one crate's rustdoc JSON.
///
/// `package` names the generated package; left out, the crate names it
/// (`inventory_store` -> `InventoryStoreApi`).
pub fn rustdoc_to_sysml(json: &str, package: Option<&str>) -> Result<Imported, ImportError> {
    Ok(crate::surface::to_sysml(
        &rustdoc_to_surface(json)?,
        package,
    ))
}

/// One crate's rustdoc JSON as a surface: what the package will be
/// written from, and what a reader of any other language writes.
pub fn rustdoc_to_surface(json: &str) -> Result<Surface, ImportError> {
    let doc: Json = serde_json::from_str(json).map_err(|e| ImportError::NotJson(e.to_string()))?;
    let index = doc
        .get("index")
        .and_then(Json::as_object)
        .ok_or_else(|| ImportError::Malformed("its item index".to_string()))?;
    // rustdoc's JSON is versioned, and a function's parameters moved
    // from `decl` to `sig`. An older input would import as actions with
    // no parameters at all and say nothing about it, which is worse than
    // not importing it.
    if index.values().any(|entry| {
        let function = &entry["inner"]["function"];
        function.get("decl").is_some() && function.get("sig").is_none()
    }) {
        let version = doc.get("format_version").and_then(Json::as_u64);
        return Err(ImportError::OldFormat(version.unwrap_or_default()));
    }
    let root = doc
        .get("root")
        .map(id_key)
        .ok_or_else(|| ImportError::Malformed("its root module".to_string()))?;
    let item = |id: &Json| index.get(&id_key(id));
    let root_item = index
        .get(&root)
        .ok_or_else(|| ImportError::Malformed("the root module item".to_string()))?;
    let crate_name = root_item
        .get("name")
        .and_then(Json::as_str)
        .ok_or_else(|| ImportError::Malformed("the crate's name".to_string()))?
        .to_string();
    let listed = root_item["inner"]["module"]["items"]
        .as_array()
        .ok_or_else(|| ImportError::Malformed("the root module's members".to_string()))?;
    // A crate that keeps its types in private modules and re-exports them
    // -- `pub use graph::Diagram;` -- lists a `use` here, not the struct.
    // Following it is what makes such a crate importable at all: without
    // it only the items spelled out in `lib.rs` are seen, while the
    // signatures that mention the rest are written regardless.
    let mut members: Vec<Json> = Vec::new();
    for id in listed {
        match item(id).map(|entry| &entry["inner"]) {
            Some(inner) if inner.get("use").is_some() => {
                let reexport = &inner["use"];
                if reexport["is_glob"].as_bool() != Some(true) {
                    if let Some(target) = reexport.get("id") {
                        if index.contains_key(&id_key(target)) {
                            members.push(target.clone());
                        }
                    }
                }
            }
            _ => members.push(id.clone()),
        }
    }

    // Which types will end up in the surface. A signature may name one
    // that has no SysML shape -- an error enum carrying a payload, a
    // newtype over an id -- and writing `out error : ImportError` for a
    // definition that was refused leaves a name nothing answers to. So
    // settle the set first and let the signatures respect it.
    let settled = settled(index, &members);
    let mut ctx = Context {
        index,
        crate_name: &crate_name,
        written: &settled,
        exact: HashSet::new(),
        skipped: Vec::new(),
    };
    let mut items = Vec::new();
    for id in &members {
        let Some(entry) = item(id) else {
            continue;
        };
        let Some(name) = entry.get("name").and_then(Json::as_str) else {
            continue;
        };
        let Some(inner) = entry.get("inner").and_then(Json::as_object) else {
            continue;
        };
        let made = match inner.keys().next().map(String::as_str).unwrap_or_default() {
            "struct" => ctx.record(name, entry).map(Item::Record),
            "enum" => ctx.enumeration(name, entry).map(Item::Enumeration),
            "trait" => ctx.interface(name, entry).map(Item::Interface),
            "function" => {
                let made = ctx.callable(name, entry, None).map(Item::Function);
                if made.is_none() {
                    ctx.skipped.push(format!("{name} -- unsupported signature"));
                }
                made
            }
            other => {
                ctx.skipped.push(format!("{name} -- {other}"));
                None
            }
        };
        items.extend(made);
    }

    // The scalars the signatures turned out to name, each under the
    // SysML scalar it is one of, in one order however the crate wrote.
    let Context { exact, skipped, .. } = ctx;
    let scalars = EXACT
        .iter()
        .filter(|(_, name, _)| exact.contains(name))
        .map(|(rust, name, scalar)| Scalar {
            name: (*name).to_string(),
            of: *scalar,
            path: (*rust).to_string(),
            capabilities: DERIVES.iter().map(|it| (*it).to_string()).collect(),
        })
        .collect();
    Ok(Surface {
        language: crate::binding::RUST.to_string(),
        module: crate_name,
        unit: Some("crate".to_string()),
        tool: Some("sysml import-rust".to_string()),
        package: None,
        scalars,
        items,
        skipped,
    })
}

/// The Rust scalars this importer gives a name of their own, each with
/// the SysML scalar it is one of. `Boolean`, `String`, `Real`, `Integer`
/// and `Natural` come back out of the generator as `bool`, `String`,
/// `f64`, `i64` and `u64`; anything else has to say what it is or it
/// returns from the round trip as the wrong type.
const EXACT: &[(&str, &str, Primitive)] = &[
    ("u8", "RustU8", Primitive::Natural),
    ("u16", "RustU16", Primitive::Natural),
    ("u32", "RustU32", Primitive::Natural),
    ("u128", "RustU128", Primitive::Natural),
    ("usize", "RustUsize", Primitive::Natural),
    ("i8", "RustI8", Primitive::Integer),
    ("i16", "RustI16", Primitive::Integer),
    ("i32", "RustI32", Primitive::Integer),
    ("i128", "RustI128", Primitive::Integer),
    ("isize", "RustIsize", Primitive::Integer),
    ("f32", "RustF32", Primitive::Real),
    ("char", "RustChar", Primitive::String),
    ("&str", "RustStr", Primitive::String),
];

/// What every Rust scalar can do. The generator says nothing about a
/// type it did not write unless the model says what that type is
/// capable of, and a primitive is capable of all of it.
const DERIVES: [&str; 4] = ["Debug", "Clone", "PartialEq", "Default"];

/// The names of the crate's types that become definitions: the
/// non-generic plain structs, plain enums and traits. Decided by the same
/// tests the emitters apply, so that the two answers are the same.
fn settled(index: &serde_json::Map<String, Json>, members: &[Json]) -> HashSet<String> {
    let mut names = HashSet::new();
    for id in members {
        let Some(entry) = index.get(&id_key(id)) else {
            continue;
        };
        let (Some(name), Some(inner)) = (
            entry.get("name").and_then(Json::as_str),
            entry.get("inner").and_then(Json::as_object),
        ) else {
            continue;
        };
        let made = match inner.keys().next().map(String::as_str) {
            Some("struct") => {
                !is_generic(&inner["struct"])
                    && inner["struct"]["kind"]["plain"]["fields"].is_array()
            }
            Some("enum") => !is_generic(&inner["enum"]) && plain_variants(index, entry).is_some(),
            Some("trait") => !is_generic(&inner["trait"]),
            _ => false,
        };
        if made {
            names.insert(name.to_string());
        }
    }
    names
}

/// The names of an enum's variants, where every one of them is a bare
/// name -- a variant carrying a payload is a shape SysML's enumerations
/// have not got.
fn plain_variants(index: &serde_json::Map<String, Json>, entry: &Json) -> Option<Vec<String>> {
    let mut names = Vec::new();
    for id in entry["inner"]["enum"]["variants"].as_array()? {
        let variant = index.get(&id_key(id))?;
        if variant["inner"]["variant"]["kind"].as_str() != Some("plain") {
            return None;
        }
        names.push(variant.get("name")?.as_str()?.to_string());
    }
    Some(names)
}

/// The pieces every reader needs at hand.
struct Context<'a> {
    index: &'a serde_json::Map<String, Json>,
    crate_name: &'a str,
    /// The type names that will be in the surface.
    written: &'a HashSet<String>,
    /// Which of [`EXACT`] the signatures read so far have named.
    exact: HashSet<&'static str>,
    /// What had no shape, and why, in the order it was met.
    skipped: Vec<String>,
}

impl<'a> Context<'a> {
    fn item(&self, id: &Json) -> Option<&'a Json> {
        self.index.get(&id_key(id))
    }

    /// `struct` -> record, each field a typed name.
    fn record(&mut self, name: &str, entry: &Json) -> Option<Record> {
        if is_generic(&entry["inner"]["struct"]) {
            self.skipped.push(format!("{name} -- generic"));
            return None;
        }
        let Some(fields) = entry["inner"]["struct"]["kind"]["plain"]["fields"].as_array() else {
            self.skipped.push(format!("{name} -- not a plain struct"));
            return None;
        };
        let mut record = Record {
            name: name.to_string(),
            path: format!("{}::{name}", self.crate_name),
            doc: docs(entry),
            ..Record::default()
        };
        for id in fields {
            let Some(field) = self.item(id) else { continue };
            let Some(field_name) = field.get("name").and_then(Json::as_str) else {
                continue;
            };
            match self.attribute_type(&field["inner"]["struct_field"]) {
                Some((of, multiplicity)) => record.fields.push(Field {
                    name: field_name.to_string(),
                    doc: docs(field),
                    of,
                    multiplicity,
                }),
                None => self
                    .skipped
                    .push(format!("{name}.{field_name} -- unmappable type")),
            }
        }
        Some(record)
    }

    /// A plain `enum` -> enumeration.
    fn enumeration(&mut self, name: &str, entry: &Json) -> Option<Enumeration> {
        if is_generic(&entry["inner"]["enum"]) {
            self.skipped.push(format!("{name} -- generic"));
            return None;
        }
        let Some(values) = plain_variants(self.index, entry) else {
            self.skipped.push(format!("{name} -- not a plain enum"));
            return None;
        };
        Some(Enumeration {
            name: name.to_string(),
            path: format!("{}::{name}", self.crate_name),
            doc: docs(entry),
            capabilities: Vec::new(),
            values,
        })
    }

    /// A `trait` -> interface, each method a callable.
    fn interface(&mut self, name: &str, entry: &Json) -> Option<Interface> {
        if is_generic(&entry["inner"]["trait"]) {
            self.skipped.push(format!("{name} -- generic"));
            return None;
        }
        let mut interface = Interface {
            name: name.to_string(),
            path: format!("{}::{name}", self.crate_name),
            doc: docs(entry),
            ..Interface::default()
        };
        for id in entry["inner"]["trait"]["items"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let Some(method) = self.item(id) else {
                continue;
            };
            let Some(method_name) = method.get("name").and_then(Json::as_str) else {
                continue;
            };
            // a trait holds associated types and constants as well as
            // methods, and only a method is something to perform
            if method["inner"].get("function").is_none() {
                self.skipped
                    .push(format!("{name}::{method_name} -- not a method"));
                continue;
            }
            match self.callable(method_name, method, Some(name)) {
                Some(callable) => interface.methods.push(callable),
                None => self
                    .skipped
                    .push(format!("{name}::{method_name} -- unsupported signature")),
            }
        }
        Some(interface)
    }

    /// A callable with its parameters and what it returns, or nothing
    /// where the signature cannot be written monomorphically.
    fn callable(&mut self, name: &str, entry: &Json, owner: Option<&str>) -> Option<Callable> {
        let function = &entry["inner"]["function"];
        if is_generic(function) {
            return None;
        }
        let mut callable = Callable {
            name: name.to_string(),
            path: match owner {
                Some(interface) => format!("{}::{interface}::{name}", self.crate_name),
                None => format!("{}::{name}", self.crate_name),
            },
            doc: docs(entry),
            is_async: function["header"]["is_async"].as_bool() == Some(true),
            ..Callable::default()
        };
        for input in function["sig"]["inputs"].as_array().into_iter().flatten() {
            let (Some(param), Some(ty)) = (input.get(0).and_then(Json::as_str), input.get(1))
            else {
                continue;
            };
            if param == "self" {
                callable.receiver = receiver(ty).to_string();
                continue;
            }
            let (of, multiplicity) = self.attribute_type(ty)?;
            callable.parameters.push(Field {
                name: param.to_string(),
                doc: None,
                of,
                multiplicity,
            });
        }
        let output = &function["sig"]["output"];
        if !output.is_null() {
            let (value, error) = match split_result(output) {
                Some((value, error)) => {
                    callable.fallible = true;
                    (value, Some(error))
                }
                None => (output, None),
            };
            let (of, multiplicity) = self.attribute_type(value)?;
            callable.result = Some(TypeRef { of, multiplicity });
            if let Some(error) = error {
                let (of, _) = self.attribute_type(error)?;
                callable.error = Some(of);
            }
        }
        Some(callable)
    }

    /// A rustdoc type as `(SysML type, multiplicity)`, or `None` for what
    /// has no monomorphic SysML shape.
    fn attribute_type(&mut self, ty: &Json) -> Option<(String, Multiplicity)> {
        if let Some(primitive) = ty.get("primitive").and_then(Json::as_str) {
            return Some((self.scalar(primitive)?, Multiplicity::One));
        }
        // A `&str` is a type in its own right -- how Rust passes a
        // string -- and reads as one. Any other borrow is refused: what
        // the model has to say is a value, and a signature taking the
        // value where the crate takes a reference to it does not
        // compile, so there is nothing here to write down.
        if let Some(borrowed) = ty.get("borrowed_ref") {
            if borrowed["type"].get("primitive").and_then(Json::as_str) == Some("str")
                && borrowed["is_mutable"].as_bool() != Some(true)
            {
                return Some((self.scalar("&str")?, Multiplicity::One));
            }
            return None;
        }
        if let Some(path) = ty.get("resolved_path") {
            let name = path.get("path").and_then(Json::as_str)?;
            let base = name.rsplit("::").next().unwrap_or(name);
            let argument = path["args"]["angle_bracketed"]["args"]
                .as_array()
                .and_then(|args| args.first())
                .map(|arg| &arg["type"]);
            return match base {
                "String" => Some(("String".to_string(), Multiplicity::One)),
                // the container becomes a multiplicity on the element
                // type, and a multiplicity is said once: a container
                // holding another has no second place to say it in, and
                // folding the two into one would claim the crate takes
                // something it does not
                "Vec" => self.element(argument?, Multiplicity::Many),
                "Option" => self.element(argument?, Multiplicity::Optional),
                "Box" | "Arc" | "Rc" => self.attribute_type(argument?),
                "Result" => None,
                _ => {
                    // a type of this crate, by the name its item declares
                    let local = self.item(&path["id"])?;
                    let declared = local.get("name")?.as_str()?;
                    self.written
                        .contains(declared)
                        .then(|| (declared.to_string(), Multiplicity::One))
                }
            };
        }
        None
    }

    /// What a container holds, under the multiplicity the container
    /// asks for, so long as the element wants no multiplicity of its own.
    fn element(
        &mut self,
        argument: &Json,
        multiplicity: Multiplicity,
    ) -> Option<(String, Multiplicity)> {
        let (inner, nested) = self.attribute_type(argument)?;
        nested.is_one().then_some((inner, multiplicity))
    }

    /// The SysML type one Rust scalar reads as, remembering the ones
    /// this package will have to define for itself.
    fn scalar(&mut self, rust: &str) -> Option<String> {
        if let Some((_, name, _)) = EXACT.iter().find(|(spelled, _, _)| *spelled == rust) {
            self.exact.insert(name);
            return Some((*name).to_string());
        }
        Some(
            match rust {
                "bool" => "Boolean",
                "f64" => "Real",
                "i64" => "Integer",
                "u64" => "Natural",
                _ => return None,
            }
            .to_string(),
        )
    }
}

/// Whether an item declares parameters of its own -- type, lifetime or
/// const. A generic item has no monomorphic SysML shape: `Wrapper<T>`
/// cannot be spelled as a path, so a binding naming it would send the
/// generator to write a reference to something that is not a type.
fn is_generic(inner: &Json) -> bool {
    inner["generics"]["params"]
        .as_array()
        .is_some_and(|params| !params.is_empty())
}

/// `Result<T, E>` split into its two sides, if `ty` is one.
fn split_result(ty: &Json) -> Option<(&Json, &Json)> {
    let path = ty.get("resolved_path")?;
    if path.get("path").and_then(Json::as_str)?.rsplit("::").next() != Some("Result") {
        return None;
    }
    let args = path["args"]["angle_bracketed"]["args"].as_array()?;
    match args.as_slice() {
        [value, error] => Some((&value["type"], &error["type"])),
        _ => None,
    }
}

/// `&self`, `&mut self` or `self`, the way the signature spells it.
fn receiver(ty: &Json) -> &'static str {
    match ty.get("borrowed_ref") {
        Some(borrowed) if borrowed["is_mutable"].as_bool() == Some(true) => "&mut self",
        Some(_) => "&self",
        None => "self",
    }
}

/// The item's doc comment, where it has one.
fn docs(entry: &Json) -> Option<String> {
    entry
        .get("docs")
        .and_then(Json::as_str)
        .map(str::to_string)
        .filter(|text| !text.trim().is_empty())
}

/// Rustdoc ids appear both as numbers and as strings; the index keys are
/// strings.
fn id_key(id: &Json) -> String {
    match id {
        Json::String(text) => text.clone(),
        other => other.to_string(),
    }
}

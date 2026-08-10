//! An existing Rust API surface, imported as a SysML package.
//!
//! The input is what `rustdoc` writes with `--output-format json`: the one
//! machine-readable statement of a crate's public API, re-exports resolved
//! and macros expanded. [`rustdoc_to_sysml`] turns it into a SysML package
//! in which every definition carries a `@rust { ... }` metadata usage
//! naming the Rust item it binds to -- so a systems model can `import` the
//! package, type its ports and `perform` its actions, and a code generator
//! can later call the real API instead of inventing parallel types.
//!
//! The mapping, deliberately monomorphic:
//!
//! - `struct` -> `item def`, each field an attribute; `Vec<T>` becomes
//!   `[*]`, `Option<T>` becomes `[0..1]`, `Box`/`Arc`/`Rc` vanish
//! - plain `enum` -> `enum def`
//! - `trait` -> `port def`, each method an `action def` with `in`
//!   parameters, an `out result` and -- for `Result` returns -- an
//!   `out error [0..1]`
//! - free `fn` -> `action def`
//! - generic items, and anything else, are skipped and listed at the end
//!   of the package rather than dropped silently
//!
//! A `pub use` re-export is followed to the item it names, so a crate
//! that keeps its types in modules imports as what it publishes rather
//! than as what its `lib.rs` happens to spell out. Nothing that was
//! skipped is ever named by a signature: a definition whose parameters
//! or result have no shape is skipped in turn, so every name in the
//! package resolves.
//!
//! Names that collide with SysML keywords are quoted (`'filter'`). The
//! output is deterministic: declaration order in, declaration order out.
//!
//! Regenerate an input with:
//!
//! ```sh
//! cargo +nightly rustdoc -- -Zunstable-options --output-format json
//! ```

use std::collections::HashSet;
use std::fmt::Write as _;

use serde_json::Value as Json;

/// What can be wrong with a rustdoc JSON input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    /// The text is not JSON at all.
    NotJson(String),
    /// The JSON has no piece this importer needs.
    Malformed(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::NotJson(why) => write!(f, "the input is not JSON: {why}"),
            ImportError::Malformed(what) => write!(f, "the input is missing {what}"),
        }
    }
}

impl std::error::Error for ImportError {}

/// The SysML package for one crate's rustdoc JSON.
///
/// `package` names the generated package; left out, the crate names it
/// (`inventory_store` -> `InventoryStoreApi`).
pub fn rustdoc_to_sysml(json: &str, package: Option<&str>) -> Result<String, ImportError> {
    let doc: Json = serde_json::from_str(json).map_err(|e| ImportError::NotJson(e.to_string()))?;
    let index = doc
        .get("index")
        .and_then(Json::as_object)
        .ok_or_else(|| ImportError::Malformed("its item index".to_string()))?;
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
    let members = &members;

    let package_name = package.map_or_else(|| format!("{}Api", camel(&crate_name)), String::from);
    let mut out = String::new();
    let mut skipped: Vec<String> = Vec::new();
    writeln!(
        out,
        "// generated from crate `{crate_name}` by `sysml import-rust` -- DO NOT EDIT"
    )
    .expect("writing to a String cannot fail");
    writeln!(out, "package {package_name} {{").unwrap();
    writeln!(out, "\tprivate import ScalarValues::*;\n").unwrap();
    writeln!(
        out,
        "\t// how each definition binds to the Rust it came from"
    )
    .unwrap();
    writeln!(out, "\tmetadata def rust {{").unwrap();
    writeln!(out, "\t\tattribute path : String;").unwrap();
    writeln!(out, "\t\tattribute crateName : String;").unwrap();
    writeln!(out, "\t\tattribute takesSelf : String;").unwrap();
    writeln!(out, "\t\tattribute isAsync : Boolean;").unwrap();
    writeln!(out, "\t\tattribute isFallible : Boolean;").unwrap();
    writeln!(out, "\t}}").unwrap();

    // Which types will end up in the model. A signature may name one
    // that has no SysML shape -- an error enum carrying a payload, a
    // newtype over an id -- and writing `out error : ImportError` for a
    // definition that was refused leaves a name nothing answers to. So
    // settle the set first and let the signatures respect it.
    let settled = written_types(index, &crate_name, members);

    for id in members {
        let Some(entry) = item(id) else {
            continue;
        };
        let Some(name) = entry.get("name").and_then(Json::as_str) else {
            continue;
        };
        let Some(inner) = entry.get("inner").and_then(Json::as_object) else {
            continue;
        };
        let kind = inner.keys().next().cloned().unwrap_or_default();
        let ctx = Context {
            index,
            crate_name: &crate_name,
            written: Some(&settled),
        };
        match kind.as_str() {
            "struct" => match ctx.item_def(&mut out, name, entry, &mut skipped) {
                Some(()) => {}
                None => skipped.push(format!("{name} -- not a plain struct")),
            },
            "enum" => match ctx.enum_def(&mut out, name, entry) {
                Some(()) => {}
                None => skipped.push(format!("{name} -- not a plain enum")),
            },
            "trait" => ctx.port_def(&mut out, name, entry, &mut skipped),
            "function" => {
                if !ctx.action_def(&mut out, name, entry, None) {
                    skipped.push(format!("{name} -- unsupported signature"));
                }
            }
            other => skipped.push(format!("{name} -- {other}")),
        }
    }

    if !skipped.is_empty() {
        writeln!(out, "\n\t// not imported (no monomorphic SysML shape):").unwrap();
        for line in &skipped {
            writeln!(out, "\t//   {line}").unwrap();
        }
    }
    writeln!(out, "}}").unwrap();
    Ok(out)
}

/// The names of the crate's types that become definitions, found by
/// writing them and keeping the ones that came out. Asking the emitters
/// rather than re-deciding here is what keeps the two answers the same.
fn written_types(
    index: &serde_json::Map<String, Json>,
    crate_name: &str,
    members: &[Json],
) -> HashSet<String> {
    let ctx = Context {
        index,
        crate_name,
        written: None,
    };
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
        let mut scratch = String::new();
        let mut ignored = Vec::new();
        let made = match inner.keys().next().map(String::as_str) {
            Some("struct") => ctx
                .item_def(&mut scratch, name, entry, &mut ignored)
                .is_some(),
            Some("enum") => ctx.enum_def(&mut scratch, name, entry).is_some(),
            Some("trait") => {
                ctx.port_def(&mut scratch, name, entry, &mut ignored);
                true
            }
            _ => false,
        };
        if made {
            names.insert(name.to_string());
        }
    }
    names
}

/// The pieces every emitter needs at hand.
struct Context<'a> {
    index: &'a serde_json::Map<String, Json>,
    crate_name: &'a str,
    /// The type names that will be in the model, or `None` while that is
    /// still being worked out.
    written: Option<&'a HashSet<String>>,
}

impl Context<'_> {
    fn item(&self, id: &Json) -> Option<&Json> {
        self.index.get(&id_key(id))
    }

    /// `struct` -> `item def`, each field an attribute.
    fn item_def(
        &self,
        out: &mut String,
        name: &str,
        entry: &Json,
        skipped: &mut Vec<String>,
    ) -> Option<()> {
        let fields = entry["inner"]["struct"]["kind"]["plain"]["fields"].as_array()?;
        docs(out, entry, 1);
        writeln!(out, "\titem def {} {{", quoted(name)).unwrap();
        self.binding(out, &format!("{}::{name}", self.crate_name), None, 2);
        for id in fields {
            let Some(field) = self.item(id) else { continue };
            let Some(field_name) = field.get("name").and_then(Json::as_str) else {
                continue;
            };
            match self.attribute_type(&field["inner"]["struct_field"]) {
                Some((ty, multiplicity)) => {
                    docs(out, field, 2);
                    writeln!(
                        out,
                        "\t\tattribute {} : {ty}{multiplicity};",
                        quoted(field_name)
                    )
                    .unwrap();
                }
                None => skipped.push(format!("{name}.{field_name} -- unmappable type")),
            }
        }
        writeln!(out, "\t}}").unwrap();
        Some(())
    }

    /// A plain `enum` -> `enum def`.
    fn enum_def(&self, out: &mut String, name: &str, entry: &Json) -> Option<()> {
        let variants = entry["inner"]["enum"]["variants"].as_array()?;
        let mut names = Vec::new();
        for id in variants {
            let variant = self.item(id)?;
            if variant["inner"]["variant"]["kind"].as_str() != Some("plain") {
                return None;
            }
            names.push(variant.get("name")?.as_str()?.to_string());
        }
        docs(out, entry, 1);
        writeln!(out, "\tenum def {} {{", quoted(name)).unwrap();
        self.binding(out, &format!("{}::{name}", self.crate_name), None, 2);
        for variant in names {
            writeln!(out, "\t\tenum {};", quoted(&variant)).unwrap();
        }
        writeln!(out, "\t}}").unwrap();
        Some(())
    }

    /// A `trait` -> `port def`, each method an `action def` beside it.
    fn port_def(&self, out: &mut String, name: &str, entry: &Json, skipped: &mut Vec<String>) {
        docs(out, entry, 1);
        writeln!(out, "\tport def {} {{", quoted(name)).unwrap();
        self.binding(out, &format!("{}::{name}", self.crate_name), None, 2);
        writeln!(out, "\t}}").unwrap();
        let Some(methods) = entry["inner"]["trait"]["items"].as_array() else {
            return;
        };
        for id in methods {
            let Some(method) = self.item(id) else {
                continue;
            };
            let Some(method_name) = method.get("name").and_then(Json::as_str) else {
                continue;
            };
            if !self.action_def(out, method_name, method, Some(name)) {
                skipped.push(format!("{name}::{method_name} -- unsupported signature"));
            }
        }
    }

    /// A callable -> `action def` with `in`/`out` parameters. `false` when
    /// the signature cannot be written monomorphically.
    fn action_def(&self, out: &mut String, name: &str, entry: &Json, owner: Option<&str>) -> bool {
        let function = &entry["inner"]["function"];
        if function["generics"]["params"]
            .as_array()
            .is_some_and(|params| !params.is_empty())
        {
            return false;
        }
        let mut lines = Vec::new();
        let mut takes_self = "";
        for input in function["sig"]["inputs"].as_array().into_iter().flatten() {
            let (Some(param), Some(ty)) = (input.get(0).and_then(Json::as_str), input.get(1))
            else {
                continue;
            };
            if param == "self" {
                takes_self = receiver(ty);
                continue;
            }
            match self.attribute_type(ty) {
                Some((mapped, multiplicity)) => {
                    lines.push(format!("in {} : {mapped}{multiplicity};", quoted(param)));
                }
                None => return false,
            }
        }
        let output = &function["sig"]["output"];
        let mut fallible = false;
        if !output.is_null() {
            let (value, error) = match split_result(output) {
                Some((value, error)) => {
                    fallible = true;
                    (value, Some(error))
                }
                None => (output, None),
            };
            match self.attribute_type(value) {
                Some((ty, multiplicity)) => lines.push(format!("out result : {ty}{multiplicity};")),
                None => return false,
            }
            if let Some(error) = error {
                match self.attribute_type(error) {
                    Some((ty, _)) => lines.push(format!("out error : {ty}[0..1];")),
                    None => return false,
                }
            }
        }

        let path = match owner {
            Some(port) => format!("{}::{port}::{name}", self.crate_name),
            None => format!("{}::{name}", self.crate_name),
        };
        docs(out, entry, 1);
        writeln!(out, "\taction def {} {{", quoted(&camel(name))).unwrap();
        let is_async = function["header"]["is_async"].as_bool() == Some(true);
        self.binding(out, &path, Some((takes_self, is_async, fallible)), 2);
        for line in lines {
            writeln!(out, "\t\t{line}").unwrap();
        }
        writeln!(out, "\t}}").unwrap();
        true
    }

    /// The `@rust { ... }` usage binding one definition to its Rust item.
    fn binding(
        &self,
        out: &mut String,
        path: &str,
        callable: Option<(&str, bool, bool)>,
        indent: usize,
    ) {
        let tabs = "\t".repeat(indent);
        write!(
            out,
            "{tabs}@rust {{ :>> path = \"{path}\"; :>> crateName = \"{}\";",
            self.crate_name
        )
        .unwrap();
        if let Some((takes_self, is_async, fallible)) = callable {
            write!(
                out,
                " :>> takesSelf = \"{takes_self}\"; :>> isAsync = {is_async}; :>> isFallible = {fallible};"
            )
            .unwrap();
        }
        writeln!(out, " }}").unwrap();
    }

    /// A rustdoc type as `(SysML type, multiplicity)`, or `None` for what
    /// has no monomorphic SysML shape.
    fn attribute_type(&self, ty: &Json) -> Option<(String, String)> {
        if let Some(primitive) = ty.get("primitive").and_then(Json::as_str) {
            let mapped = match primitive {
                "bool" => "Boolean",
                "f32" | "f64" => "Real",
                "i8" | "i16" | "i32" | "i64" | "i128" | "isize" => "Integer",
                "u8" | "u16" | "u32" | "u64" | "u128" | "usize" => "Natural",
                "char" | "str" => "String",
                _ => return None,
            };
            return Some((mapped.to_string(), String::new()));
        }
        // `&T` reads as `T`: the model has values, not borrows
        if let Some(borrowed) = ty.get("borrowed_ref") {
            return self.attribute_type(&borrowed["type"]);
        }
        if let Some(path) = ty.get("resolved_path") {
            let name = path.get("path").and_then(Json::as_str)?;
            let base = name.rsplit("::").next().unwrap_or(name);
            let first_argument = || {
                path["args"]["angle_bracketed"]["args"]
                    .as_array()
                    .and_then(|args| args.first())
                    .map(|arg| &arg["type"])
            };
            return match base {
                "String" => Some(("String".to_string(), String::new())),
                // the container becomes a multiplicity on the element type
                "Vec" => {
                    let (inner, _) = self.attribute_type(first_argument()?)?;
                    Some((inner, "[*]".to_string()))
                }
                "Option" => {
                    let (inner, _) = self.attribute_type(first_argument()?)?;
                    Some((inner, "[0..1]".to_string()))
                }
                "Box" | "Arc" | "Rc" => self.attribute_type(first_argument()?),
                "Result" => None,
                _ => {
                    // a type of this crate, by the name its item declares
                    let local = self.item(&path["id"])?;
                    let declared = local.get("name")?.as_str()?;
                    match self.written {
                        Some(written) if !written.contains(declared) => None,
                        _ => Some((quoted(declared), String::new())),
                    }
                }
            };
        }
        None
    }
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

/// The item's doc comment, as a SysML `doc` block.
fn docs(out: &mut String, entry: &Json, indent: usize) {
    let Some(text) = entry.get("docs").and_then(Json::as_str) else {
        return;
    };
    let tabs = "\t".repeat(indent);
    // `*/` inside would end the block early
    let safe = text.replace("*/", "*\\/");
    writeln!(out, "{tabs}doc /* {} */", safe.trim().replace('\n', " ")).unwrap();
}

/// A Rust identifier as a SysML name: as it is, quoted when it collides
/// with a keyword.
fn quoted(name: &str) -> String {
    if sysml_syntax::SyntaxKind::from_keyword(name).is_some() {
        format!("'{name}'")
    } else {
        name.to_string()
    }
}

/// `get_stock` -> `GetStock`, for the actions a method becomes.
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

/// Rustdoc ids appear both as numbers and as strings; the index keys are
/// strings.
fn id_key(id: &Json) -> String {
    match id {
        Json::String(text) => text.clone(),
        other => other.to_string(),
    }
}

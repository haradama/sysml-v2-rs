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
//! A scalar keeps the width the crate declared. `sysml rustgen` reads a
//! `Natural` back as a `u64` and a `Real` as an `f64`, so a `u32` folded
//! into `Natural` returns from the round trip as a type the crate's own
//! functions refuse. Every Rust scalar that is not one of those five
//! gets an `attribute def` of its own, bound to the type it stands for.
//! For the same reason a borrow reads as a value only for `&str`, which
//! is how Rust passes a string: a signature taking `T` where the crate
//! takes `&T` does not compile, so every other `&T` is refused.
//!
//! A container nested in a container -- `Option<Vec<u8>>` -- is refused
//! as well. SysML says how many of a type there are once, and folding
//! the two levels into one would say something the crate does not.
//!
//! Rust keeps types and functions in namespaces of their own and SysML
//! does not, so `struct Config` and `fn config` both want to be
//! `Config`. The type keeps the name, since signatures refer to it, and
//! the action takes the name of the trait it belongs to as a prefix --
//! or a number, where it belongs to no trait -- and says so in its
//! `doc`.
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
//! rustdoc's JSON is versioned, and this importer reads the format that
//! spells a function's parameters as `sig`. An older document is refused
//! rather than imported: every action in it would come out with no
//! parameters at all, and nothing would say so.
//!
//! Regenerate an input with:
//!
//! ```sh
//! cargo +nightly rustdoc -- -Zunstable-options --output-format json
//! ```

use std::collections::HashSet;
use std::fmt::Write as _;

use serde_json::Value as Json;

use crate::binding;

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
pub fn rustdoc_to_sysml(json: &str, package: Option<&str>) -> Result<String, ImportError> {
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
    writeln!(out, "\tmetadata def {} {{", binding::DEF).unwrap();
    for (name, ty) in binding::ALL {
        writeln!(out, "\t\tattribute {name} : {ty};").unwrap();
    }
    writeln!(out, "\t}}").unwrap();

    // Which types will end up in the model. A signature may name one
    // that has no SysML shape -- an error enum carrying a payload, a
    // newtype over an id -- and writing `out error : ImportError` for a
    // definition that was refused leaves a name nothing answers to. So
    // settle the set first and let the signatures respect it.
    let settled = written_types(index, &crate_name, members);
    // Every name the package will declare, so that the second member to
    // want one is renamed rather than left to shadow the first.
    let mut taken: HashSet<String> = settled.clone();
    taken.extend(EXACT.iter().map(|(_, name, _)| name.to_string()));

    // The members go into a buffer of their own because what they name
    // decides what has to be declared above them: which Rust scalars
    // the crate turned out to use is known only once they are written.
    let mut body = String::new();
    let mut ctx = Context {
        index,
        crate_name: &crate_name,
        written: Some(&settled),
        exact: HashSet::new(),
    };
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
        match inner.keys().next().map(String::as_str).unwrap_or_default() {
            "struct" => {
                ctx.item_def(&mut body, name, entry, &mut skipped);
            }
            "enum" => {
                ctx.enum_def(&mut body, name, entry, &mut skipped);
            }
            "trait" => {
                ctx.port_def(&mut body, name, entry, &mut skipped, &mut taken);
            }
            "function" => {
                if !ctx.action_def(&mut body, name, entry, None, &mut taken) {
                    skipped.push(format!("{name} -- unsupported signature"));
                }
            }
            other => skipped.push(format!("{name} -- {other}")),
        }
    }

    if !ctx.exact.is_empty() {
        writeln!(
            out,
            "\n\t// the Rust scalars this crate uses that no SysML one stands for exactly"
        )
        .unwrap();
        for (rust, name, scalar) in EXACT.iter().filter(|(_, name, _)| ctx.exact.contains(name)) {
            writeln!(
                out,
                "\tattribute def {name} :> {scalar} {{ @{} {{ :>> {} = \"{rust}\"; :>> {} = \"{DERIVES}\"; }} }}",
                binding::DEF,
                binding::PATH,
                binding::DERIVES
            )
            .unwrap();
        }
    }
    out.push_str(&body);

    if !skipped.is_empty() {
        writeln!(out, "\n\t// not imported (no monomorphic SysML shape):").unwrap();
        for line in &skipped {
            writeln!(out, "\t//   {line}").unwrap();
        }
    }
    writeln!(out, "}}").unwrap();
    Ok(out)
}

/// The Rust scalars this importer gives a name of their own, each with
/// the SysML scalar it is one of. `Boolean`, `String`, `Real`, `Integer`
/// and `Natural` come back out of the generator as `bool`, `String`,
/// `f64`, `i64` and `u64`; anything else has to say what it is or it
/// returns from the round trip as the wrong type.
const EXACT: &[(&str, &str, &str)] = &[
    ("u8", "RustU8", "Natural"),
    ("u16", "RustU16", "Natural"),
    ("u32", "RustU32", "Natural"),
    ("u128", "RustU128", "Natural"),
    ("usize", "RustUsize", "Natural"),
    ("i8", "RustI8", "Integer"),
    ("i16", "RustI16", "Integer"),
    ("i32", "RustI32", "Integer"),
    ("i128", "RustI128", "Integer"),
    ("isize", "RustIsize", "Integer"),
    ("f32", "RustF32", "Real"),
    ("char", "RustChar", "String"),
    ("&str", "RustStr", "String"),
];

/// What every Rust scalar can do. The generator says nothing about a
/// type it did not write unless the model says what that type is
/// capable of, and a primitive is capable of all of it.
const DERIVES: &str = "Debug, Clone, PartialEq, Default";

/// The names of the crate's types that become definitions, found by
/// writing them and keeping the ones that came out. Asking the emitters
/// rather than re-deciding here is what keeps the two answers the same.
fn written_types(
    index: &serde_json::Map<String, Json>,
    crate_name: &str,
    members: &[Json],
) -> HashSet<String> {
    let mut ctx = Context {
        index,
        crate_name,
        written: None,
        exact: HashSet::new(),
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
        // written to and thrown away: this pass asks the emitters what
        // they make of an item, not what they write
        let mut scratch = String::new();
        let mut ignored = Vec::new();
        let mut taken = HashSet::new();
        let made = match inner.keys().next().map(String::as_str) {
            Some("struct") => ctx.item_def(&mut scratch, name, entry, &mut ignored),
            Some("enum") => ctx.enum_def(&mut scratch, name, entry, &mut ignored),
            Some("trait") => ctx.port_def(&mut scratch, name, entry, &mut ignored, &mut taken),
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
    /// Which of [`EXACT`] the signatures written so far have named.
    exact: HashSet<&'static str>,
}

impl<'a> Context<'a> {
    fn item(&self, id: &Json) -> Option<&'a Json> {
        self.index.get(&id_key(id))
    }

    /// `struct` -> `item def`, each field an attribute.
    fn item_def(
        &mut self,
        out: &mut String,
        name: &str,
        entry: &Json,
        skipped: &mut Vec<String>,
    ) -> bool {
        if is_generic(&entry["inner"]["struct"]) {
            skipped.push(format!("{name} -- generic"));
            return false;
        }
        let Some(fields) = entry["inner"]["struct"]["kind"]["plain"]["fields"].as_array() else {
            skipped.push(format!("{name} -- not a plain struct"));
            return false;
        };
        docs(out, entry, 1, None);
        writeln!(out, "\titem def {} {{", quoted(name)).unwrap();
        self.binding(out, &format!("{}::{name}", self.crate_name), None, 2);
        for id in fields {
            let Some(field) = self.item(id) else { continue };
            let Some(field_name) = field.get("name").and_then(Json::as_str) else {
                continue;
            };
            match self.attribute_type(&field["inner"]["struct_field"]) {
                Some((ty, multiplicity)) => {
                    docs(out, field, 2, None);
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
        true
    }

    /// A plain `enum` -> `enum def`.
    fn enum_def(
        &self,
        out: &mut String,
        name: &str,
        entry: &Json,
        skipped: &mut Vec<String>,
    ) -> bool {
        if is_generic(&entry["inner"]["enum"]) {
            skipped.push(format!("{name} -- generic"));
            return false;
        }
        let Some(names) = self.plain_variants(entry) else {
            skipped.push(format!("{name} -- not a plain enum"));
            return false;
        };
        docs(out, entry, 1, None);
        writeln!(out, "\tenum def {} {{", quoted(name)).unwrap();
        self.binding(out, &format!("{}::{name}", self.crate_name), None, 2);
        for variant in names {
            writeln!(out, "\t\tenum {};", quoted(&variant)).unwrap();
        }
        writeln!(out, "\t}}").unwrap();
        true
    }

    /// The names of an enum's variants, where every one of them is a
    /// bare name -- a variant carrying a payload is a shape SysML's
    /// enumerations have not got.
    fn plain_variants(&self, entry: &Json) -> Option<Vec<String>> {
        let mut names = Vec::new();
        for id in entry["inner"]["enum"]["variants"].as_array()? {
            let variant = self.item(id)?;
            if variant["inner"]["variant"]["kind"].as_str() != Some("plain") {
                return None;
            }
            names.push(variant.get("name")?.as_str()?.to_string());
        }
        Some(names)
    }

    /// A `trait` -> `port def`, each method an `action def` beside it.
    fn port_def(
        &mut self,
        out: &mut String,
        name: &str,
        entry: &Json,
        skipped: &mut Vec<String>,
        taken: &mut HashSet<String>,
    ) -> bool {
        if is_generic(&entry["inner"]["trait"]) {
            skipped.push(format!("{name} -- generic"));
            return false;
        }
        docs(out, entry, 1, None);
        writeln!(out, "\tport def {} {{", quoted(name)).unwrap();
        self.binding(out, &format!("{}::{name}", self.crate_name), None, 2);
        writeln!(out, "\t}}").unwrap();
        let Some(methods) = entry["inner"]["trait"]["items"].as_array() else {
            return true;
        };
        for id in methods {
            let Some(method) = self.item(id) else {
                continue;
            };
            let Some(method_name) = method.get("name").and_then(Json::as_str) else {
                continue;
            };
            // a trait holds associated types and constants as well as
            // methods, and only a method is something to perform
            if method["inner"].get("function").is_none() {
                skipped.push(format!("{name}::{method_name} -- not a method"));
                continue;
            }
            if !self.action_def(out, method_name, method, Some(name), taken) {
                skipped.push(format!("{name}::{method_name} -- unsupported signature"));
            }
        }
        true
    }

    /// A callable -> `action def` with `in`/`out` parameters. `false` when
    /// the signature cannot be written monomorphically.
    fn action_def(
        &mut self,
        out: &mut String,
        name: &str,
        entry: &Json,
        owner: Option<&str>,
        taken: &mut HashSet<String>,
    ) -> bool {
        let function = &entry["inner"]["function"];
        if is_generic(function) {
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
        let wanted = camel(name);
        let called = unclaimed(taken, &wanted, owner);
        let renamed = (called != wanted).then(|| {
            format!("`{path}` is imported as `{called}`: `{wanted}` is another member's name.")
        });
        docs(out, entry, 1, renamed.as_deref());
        writeln!(out, "\taction def {} {{", quoted(&called)).unwrap();
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
            "{tabs}@{} {{ :>> {} = \"{path}\"; :>> {} = \"{}\";",
            binding::DEF,
            binding::PATH,
            binding::CRATE,
            self.crate_name
        )
        .unwrap();
        if let Some((takes_self, is_async, fallible)) = callable {
            write!(
                out,
                " :>> {} = \"{takes_self}\"; :>> {} = {is_async}; :>> {} = {fallible};",
                binding::TAKES_SELF,
                binding::IS_ASYNC,
                binding::IS_FALLIBLE
            )
            .unwrap();
        }
        writeln!(out, " }}").unwrap();
    }

    /// A rustdoc type as `(SysML type, multiplicity)`, or `None` for what
    /// has no monomorphic SysML shape.
    fn attribute_type(&mut self, ty: &Json) -> Option<(String, String)> {
        if let Some(primitive) = ty.get("primitive").and_then(Json::as_str) {
            return Some((self.scalar(primitive)?, String::new()));
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
                return Some((self.scalar("&str")?, String::new()));
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
                "String" => Some(("String".to_string(), String::new())),
                // the container becomes a multiplicity on the element
                // type, and a multiplicity is said once: a container
                // holding another has no second place to say it in, and
                // folding the two into one would claim the crate takes
                // something it does not
                "Vec" => self.element(argument?, "[*]"),
                "Option" => self.element(argument?, "[0..1]"),
                "Box" | "Arc" | "Rc" => self.attribute_type(argument?),
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

    /// What a container holds, under the multiplicity the container
    /// asks for, so long as the element wants no multiplicity of its own.
    fn element(&mut self, argument: &Json, multiplicity: &str) -> Option<(String, String)> {
        let (inner, nested) = self.attribute_type(argument)?;
        nested.is_empty().then(|| (inner, multiplicity.to_string()))
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

/// The name to write a member under, given what the package has taken
/// already. A trait method that cannot have its own name reads best
/// under the trait's -- `Session::label` as `SessionLabel` -- and a free
/// function, having no trait to be told apart by, is numbered.
fn unclaimed(taken: &mut HashSet<String>, wanted: &str, owner: Option<&str>) -> String {
    let base = match owner {
        Some(trait_name) if taken.contains(wanted) => format!("{}{wanted}", camel(trait_name)),
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

/// The item's doc comment, as a SysML `doc` block, with whatever this
/// importer had to say about the item after it.
fn docs(out: &mut String, entry: &Json, indent: usize, note: Option<&str>) {
    let text = entry.get("docs").and_then(Json::as_str).unwrap_or_default();
    // `*/` inside would end the block early
    let safe = text.replace("*/", "*\\/");
    let said = [
        safe.trim().replace('\n', " "),
        note.unwrap_or_default().into(),
    ]
    .join(" ")
    .trim()
    .to_string();
    if said.is_empty() {
        return;
    }
    writeln!(out, "{}doc /* {said} */", "\t".repeat(indent)).unwrap();
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

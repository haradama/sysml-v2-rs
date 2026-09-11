//! What a definition says about the code it stands for.
//!
//! [`crate::import`] writes this metadata and [`crate::generate`] reads
//! it. Spelling the names here rather than on each side is the point of
//! keeping the two together: a property one writes and the other reads
//! under another name is a seam that cannot drift if there is only one
//! list of them.
//!
//! The notation says which language it is talking about rather than being
//! one language's own. It was `@rust { path = "..."; }`, which made every
//! other language a second vocabulary to invent. What is Rust's here is
//! the *content*: a receiver spelled `&mut self`, capabilities named after
//! traits.

/// The metadata definition a binding is a usage of.
pub const DEF: &str = "code";

/// Which language the binding is about: `"rust"`, and whatever else a
/// model binds to. A `@code` that does not say is about no language in
/// particular, and no generator should take it for its own.
///
/// Not `language`, which is a reserved word -- `language "kerml"`
/// introduces a textual representation -- and so cannot name a property.
pub const LANGUAGE: &str = "writtenIn";

/// What this crate answers for.
pub const RUST: &str = "rust";

/// The path of the item in that language:
/// `inventory_store::InventoryStore::get_stock`.
///
/// Not `item`, which reads well and is a reserved word: `item def Fuel;`
/// declares one, and a metadata property spelled that way does not
/// parse.
pub const ITEM: &str = "path";

/// The compilation unit it belongs to -- a crate, a package, a module,
/// by whatever name the language gives one.
pub const MODULE: &str = "module";

/// How the call takes its receiver: in Rust `&self`, `&mut self`,
/// `self`, or empty for a free function.
pub const TAKES_SELF: &str = "takesSelf";

/// Whether the call is asynchronous.
pub const IS_ASYNC: &str = "isAsync";

/// Whether the call can fail in the way its language has of failing --
/// a `Result`, an exception, an error return.
pub const IS_FALLIBLE: &str = "isFallible";

/// What the bound type can already do, comma-separated. In Rust these
/// are traits: `"Debug, Clone, PartialEq"`. A generator writes nothing
/// about a type it did not write, so a struct holding one claims nothing
/// of it -- unless the model says what that type can do, which only the
/// model knows.
pub const CAPABILITIES: &str = "capabilities";

/// Every property, in the order the metadata definition declares them.
pub const ALL: &[(&str, &str)] = &[
    (LANGUAGE, "String"),
    (ITEM, "String"),
    (MODULE, "String"),
    (TAKES_SELF, "String"),
    (IS_ASYNC, "Boolean"),
    (IS_FALLIBLE, "Boolean"),
    (CAPABILITIES, "String"),
];

/// Whether the binding claims the bound type can do `what`.
pub fn claims(bound: &std::collections::HashMap<String, String>, what: &str) -> bool {
    bound
        .get(CAPABILITIES)
        .into_iter()
        .flat_map(|list| list.split(','))
        .any(|claimed| claimed.trim() == what)
}

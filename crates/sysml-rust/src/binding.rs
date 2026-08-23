//! What a definition says about the Rust item it stands for.
//!
//! [`crate::import`] writes this metadata and [`crate::generate`] reads
//! it. Spelling the names here rather than on each side is the point of
//! keeping the two together: a property one writes and the other never
//! reads, or reads under another name, is a seam that cannot drift if
//! there is only one list of them.

/// The metadata definition a binding is a usage of.
pub const DEF: &str = "rust";

/// The Rust path of the item: `inventory_store::InventoryStore::get_stock`.
pub const PATH: &str = "path";

/// The crate the item belongs to.
pub const CRATE: &str = "crateName";

/// How the method takes its receiver: `&self`, `&mut self`, `self`, or
/// empty for a free function.
pub const TAKES_SELF: &str = "takesSelf";

/// Whether the call is `async`.
pub const IS_ASYNC: &str = "isAsync";

/// Whether the call returns a `Result`.
pub const IS_FALLIBLE: &str = "isFallible";

/// The traits the bound type already has, comma-separated:
/// `"Debug, Clone, PartialEq"`. The generator writes nothing about a
/// type it did not write, so a struct holding one derives nothing --
/// unless the model says what that type can do, which only the model
/// knows.
pub const DERIVES: &str = "derives";

/// Every property, in the order the metadata definition declares them.
pub const ALL: &[(&str, &str)] = &[
    (PATH, "String"),
    (CRATE, "String"),
    (TAKES_SELF, "String"),
    (IS_ASYNC, "Boolean"),
    (IS_FALLIBLE, "Boolean"),
    (DERIVES, "String"),
];

/// Whether the binding claims the bound type has `trait_name`.
pub fn claims(bound: &std::collections::HashMap<String, String>, trait_name: &str) -> bool {
    bound
        .get(DERIVES)
        .into_iter()
        .flat_map(|list| list.split(','))
        .any(|claimed| claimed.trim() == trait_name)
}

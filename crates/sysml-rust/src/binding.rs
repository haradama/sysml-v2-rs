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

/// Every property, in the order the metadata definition declares them.
pub const ALL: &[(&str, &str)] = &[
    (PATH, "String"),
    (CRATE, "String"),
    (TAKES_SELF, "String"),
    (IS_ASYNC, "Boolean"),
    (IS_FALLIBLE, "Boolean"),
];

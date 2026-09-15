//! The Rust side of a SysML model, both ways.
//!
//! [`import`] reads what `rustdoc` writes about an existing crate and
//! states it as a SysML package; [`generate`] reads a resolved model and
//! writes the Rust that calls back into such a crate. The two meet at one
//! thing -- the `@code { ... }` metadata a definition carries to say which
//! Rust item it stands for -- and that is why they are one crate: the
//! names in that metadata are spelled once, in one private module, instead
//! of as string literals on each side of a boundary that nothing checks.

mod binding;
mod expr;
mod generate;
/// Reading an existing crate's rustdoc JSON as a SysML package.
pub mod import;

pub use generate::{generate, Generated, Open, OpenKind, RustgenError};
pub use import::{rustdoc_to_sysml, ImportError, Imported};

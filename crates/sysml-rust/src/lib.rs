//! The Rust side of a SysML model, both ways.
//!
//! [`import`] reads what `rustdoc` writes about an existing crate and
//! states it as a SysML package; [`generate`] reads a resolved model and
//! writes the Rust that calls back into such a crate. The two meet at one
//! thing -- the `@rust { ... }` metadata a definition carries to say which
//! Rust item it stands for -- and that is why they are one crate: the
//! names in that metadata are spelled once, in [`binding`], instead of as
//! string literals on each side of a boundary that nothing checks.

mod binding;
mod expr;
mod generate;
pub mod import;

pub use generate::{generate, RustgenError};
pub use import::{rustdoc_to_sysml, ImportError};

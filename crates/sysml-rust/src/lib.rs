//! The Rust side of a SysML model, both ways -- and the seam through
//! which any other language comes and goes.
//!
//! [`import`] reads what `rustdoc` writes about an existing crate and
//! states it as a SysML package; [`generate`] reads a resolved model and
//! writes the Rust that calls back into such a crate. The two meet at one
//! thing -- the `@code { ... }` metadata a definition carries to say which
//! Rust item it stands for -- and its names are spelled once, in
//! [`binding`], instead of as string literals on each side of a boundary
//! that nothing checks.
//!
//! Rust is the language that ships. Every other is yours to add, through
//! the two halves of the seam, each stated as data in no language in
//! particular: a [`surface`] is what some code declares, and the import
//! goes through one -- only the reading of rustdoc is Rust's own -- so a
//! program that writes the same JSON for another language is an importer
//! for it; a [`plan`] is what a model implies for code, and a program that
//! reads the JSON and writes the code is a generator for its language.
//! `sysml import-code` and `sysml codegen` find both on `PATH`, the way
//! Doxygen finds a filter.

// Each of these says what it is for at the top of its own file. A
// second line here would be a second place to keep it right, and
// rustdoc resolves the links of the two in different scopes.
pub mod binding;
mod expr;
mod generate;
pub mod import;
pub mod plan;
pub mod surface;

pub use generate::{generate, Generated, Open, OpenKind, RustgenError};
pub use import::{rustdoc_to_surface, rustdoc_to_sysml, ImportError};
pub use surface::Imported;

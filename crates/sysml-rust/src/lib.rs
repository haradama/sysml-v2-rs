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
mod quantity;
pub mod surface;

pub use generate::{generate, Generated, Open, OpenKind, RustgenError};
pub use import::{rustdoc_to_surface, rustdoc_to_sysml, ImportError};
pub use surface::Imported;

/// Whether `id`, owned by a connector -- a succession, a transition's
/// succession -- is one of its ends. In SysML a connector end is a
/// `ReferenceUsage` (`ConnectorEnd : ReferenceUsage` in the grammar) and
/// in KerML a `Feature`, and the model builder has written both: a check
/// for the one only found no ends at all in a model written the other
/// way, and every transition and succession dropped out of what is
/// generated and planned.
pub(crate) fn connector_end(model: &sysml_model::Model, id: sysml_model::ElementId) -> bool {
    matches!(
        model.kind(id),
        sysml_model::ElementKind::Feature | sysml_model::ElementKind::ReferenceUsage
    )
}

#[cfg(test)]
mod tests {
    use sysml_model::{ElementKind, Model};

    #[test]
    fn an_end_is_a_feature_in_kerml_and_a_reference_usage_in_sysml() {
        let mut model = Model::new();
        let kerml = model.create(ElementKind::Feature);
        let sysml = model.create(ElementKind::ReferenceUsage);
        let other = model.create(ElementKind::PartUsage);
        assert!(super::connector_end(&model, kerml));
        assert!(super::connector_end(&model, sysml));
        assert!(!super::connector_end(&model, other));
    }
}

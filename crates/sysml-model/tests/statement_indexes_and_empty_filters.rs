//! What a bracket written in a statement counts, and an import whose
//! bracketed filter holds nothing.

use sysml_model::{build_model, ElementKind};

fn ranges(source: &str) -> usize {
    let (model, _) = build_model(&sysml_syntax::parse(source));
    model
        .ids()
        .filter(|&id| model.kind(id) == ElementKind::MultiplicityRange)
        .count()
}

/// An index on the name a control statement stands on counts that step;
/// the same bracket after a keyword that takes an expression does not.
#[test]
fn an_index_on_a_name_in_a_statement_counts_the_step() {
    for source in [
        "action def A { action a; if true then a[2]; }",
        "action def A { action a; then merge m[2]; }",
        "state def S { state a; transition first a accept s[2] then a; }",
        "state def S { state a; entry a[2]; }",
        "state def S { state a; accept s[2]; then a; }",
    ] {
        assert_eq!(ranges(source), 1, "{source}");
    }
    assert_eq!(
        ranges("state def S { state a; accept after 5 [s] then a; }"),
        0
    );
}

/// A filter with no condition in it is built as the private membership it
/// is, owning nothing.
#[test]
fn an_empty_filter_is_a_membership_with_no_condition() {
    let (model, roots) = build_model(&sysml_syntax::parse("package P { import Q::*[]; }"));
    let import = model.owned(roots[0])[0];
    let package = model.owned(import)[0];
    let filters: Vec<_> = model
        .owned(package)
        .iter()
        .copied()
        .filter(|&it| model.kind(it) == ElementKind::ElementFilterMembership)
        .collect();
    for filter in filters {
        assert!(model.owned(filter).is_empty());
    }
}

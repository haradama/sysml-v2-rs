//! Models that are wrong, and the constraints that say so.
//!
//! The official corpus is the ground truth for the other direction: every
//! file the OMG publishes is well-formed, so a constraint that fires on
//! one is this checker being wrong. What that proves is that the checker
//! does not cry wolf. It proves nothing at all about whether it barks --
//! a constraint that can never fire passes the corpus perfectly.
//!
//! So each model below is meant to be rejected, and the test is which
//! constraints notice. Every one of them was checked against the
//! specification before it was written down: `verify` belongs inside an
//! `objective`, as `VerificationTest.sysml` writes it, and an `enum def`
//! is a variation, so what it owns must be variants.
//!
//! Skipped when the submodule is not checked out: the constraints are
//! written against the standard library, and without it they report what
//! is missing rather than what is wrong.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sysml_semantics::Workspace;

/// One wrong model, and every constraint that is expected to fire on it.
/// The set is exact in both directions: a rule that stops noticing fails
/// here, and so does one that starts complaining about something else.
const REJECTED: &[(&str, &str, &[&str])] = &[
    (
        "an objective belongs to a case",
        "package P {\n\tpart def E {\n\t\tobjective o;\n\t}\n}\n",
        &["validateObjectiveMembershipOwningType"],
    ),
    (
        "and so does a subject",
        "package P {\n\tpart def E {\n\t\tsubject s;\n\t}\n}\n",
        &["validateParameterMembershipOwningType"],
    ),
    (
        "a requirement has one subject or none",
        "package P {\n\trequirement def R {\n\t\tsubject a;\n\t\tsubject b;\n\t}\n}\n",
        &["validateRequirementDefinitionOnlyOneSubject"],
    ),
    (
        "a calculation returns once",
        "package P {\n\tcalc def C {\n\t\treturn x;\n\t\treturn y;\n\t}\n}\n",
        &["validateFunctionResultParameterMembership"],
    ),
    (
        "a variant belongs to a variation",
        "package P {\n\tpart def Q;\n\tpart v : Q {\n\t\tvariant part w : Q;\n\t}\n}\n",
        &["validateVariantMembershipOwningNamespace"],
    ),
    (
        "an enumeration is a variation, so what it owns must be variants",
        "package P {\n\tenum def E {\n\t\tpart inside;\n\t}\n}\n",
        &["validateDefinitionVariationOwnedFeatureMembership"],
    ),
    (
        "an assumption belongs to a requirement",
        "package P {\n\tpart def E {\n\t\tassume constraint c;\n\t}\n}\n",
        &["validateRequirementConstraintMembershipOwningType"],
    ),
    (
        "and so does what it requires",
        "package P {\n\tpart def E {\n\t\trequire constraint c;\n\t}\n}\n",
        &["validateRequirementConstraintMembershipOwningType"],
    ),
    (
        "and the concern it frames",
        "package P {\n\tpart def E {\n\t\tframe concern c;\n\t}\n}\n",
        &["validateRequirementConstraintMembershipOwningType"],
    ),
    (
        "an actor belongs to a requirement or a case",
        "package P {\n\tpart def E {\n\t\tactor a;\n\t}\n}\n",
        &[
            "validateActorMembershipOwningType",
            "validateParameterMembershipOwningType",
        ],
    ),
    (
        "and so does a stakeholder",
        "package P {\n\tpart def E {\n\t\tstakeholder s;\n\t}\n}\n",
        &[
            "validateParameterMembershipOwningType",
            "validateStakeholderMembershipOwningType",
        ],
    ),
    (
        "only something that returns has a return parameter",
        "package P {\n\tpart def E {\n\t\treturn x;\n\t}\n}\n",
        &[
            "validateParameterMembershipOwningType",
            "validateReturnParameterMembershipOwningType",
        ],
    ),
    (
        "an expose belongs to a view",
        "package P {\n\tpart def Q;\n\texpose Q;\n}\n",
        &["validateExposeOwningNamespace"],
    ),
    (
        "a rendering belongs to a view",
        "package P {\n\tpart def Q;\n\tpart q : Q {\n\t\trender asTree;\n\t}\n}\n",
        &["validateViewRenderingMembershipOwningType"],
    ),
    (
        "a verification verifies inside its objective, not beside it",
        "package P {\n\trequirement def R;\n\tverification def V {\n\t\tverify requirement R;\n\t}\n}\n",
        &[
            "validateRequirementConstraintMembershipOwningType",
            "validateRequirementVerificationMembershipOwningType",
        ],
    ),
];

fn library() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/sysml-v2-release");
    if root.join("sysml.library").is_dir() {
        return Some(root.join("sysml.library"));
    }
    eprintln!("skipping: {} not checked out", root.display());
    None
}

#[test]
fn a_model_the_standard_rejects_is_reported_as_rejected() {
    let Some(library) = library() else { return };
    // The library is resolved once and every model is asked against it:
    // asking costs about a millisecond, and this is the second the whole
    // file spends.
    let mut ws = Workspace::new();
    ws.load_dir(&library).expect("the library loads");
    ws.resolve_all();

    for (says, source, expected) in REJECTED {
        let file = ws.add_file("wrong.sysml", source);
        ws.resolve_files(&[file]);
        let found = ws.findings(&[file]);
        assert!(found.syntax.is_empty(), "{says}: does not parse {found:?}");
        assert!(found.names.is_empty(), "{says}: does not resolve {found:?}");

        let fired: BTreeSet<&str> = ws
            .check_rules(&[file])
            .violations
            .iter()
            .map(|violation| violation.rule)
            .collect();
        let wanted: BTreeSet<&str> = expected.iter().copied().collect();
        assert_eq!(fired, wanted, "{says}");
    }
}

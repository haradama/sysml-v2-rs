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

/// The same question asked of the corpus, by breaking it on purpose.
///
/// Writing a wrong model by hand reaches the constraints one at a time,
/// and it reaches only the ones whose shape is already understood. The
/// corpus is 403 files of rich, correct SysML: change one thing in one
/// of them and it is still a model, still parses, still resolves -- and
/// is now wrong in a way somebody's editor could be wrong.
///
/// Three sweeps got here. Fifty keyword swaps over every file left 1674
/// such models and tripped twenty-six constraints, and found a panic:
/// `connector ps : P ([0..*] myCart, ...)` counted the `*` of the bound
/// as a step of the name beside it. A second widened the swaps past the
/// definition keywords to expressions, multiplicity, visibility, time
/// structure and the KerML relationship words -- 4257 models, and
/// thirty-nine constraints. A third broke one place at a time rather
/// than every place at once, and took whole lines out and put them in
/// twice: 12040 models, and fourteen more. Neither of the last two
/// found a panic.
///
/// Fifty-eight of the hundred and seventy answered constraints are
/// demonstrated to fire, counting the five that only a hand-written
/// model reaches.
///
/// Each row is one break, the file it is made in, and constraints that
/// must notice. Other constraints may notice too -- one keyword can be
/// wrong in several ways at once -- so the named ones must be among
/// what fires rather than all of it.
#[derive(Clone, Copy)]
enum Break {
    /// every occurrence of the first, written as the second
    Swap(&'static str, &'static str),
    /// only the occurrence at that index, counted from zero
    SwapNth(&'static str, &'static str, usize),
    /// one line taken out, counted from one
    Drop(usize),
    /// one line written twice
    Repeat(usize),
}

use Break::{Drop, Repeat, Swap, SwapNth};

const MUTATED: &[(&str, Break, &[&str])] = &[
    (
        "UseCaseTest.sysml",
        Swap("use case def ", "part def "),
        &["validateActorMembershipOwningType"],
    ),
    (
        "CauseAndEffectExample.sysml",
        Swap("end ", ""),
        &[
            "validateAssociationRelatedTypes",
            "validateConnectorRelatedFeatures",
        ],
    ),
    (
        "EnumerationTest.sysml",
        Swap("attribute def ", "part def "),
        &[
            "validateAttributeDefinitionFeatures",
            "validateDataTypeSpecialization",
        ],
    ),
    (
        "Vehicle Analysis Demo.sysml",
        Swap("attribute def ", "part def "),
        &["validateAttributeUsageFeatures"],
    ),
    (
        "Connectors.kerml",
        Swap("end ", ""),
        &["validateBindingConnectorIsBinary"],
    ),
    (
        "Model Library Example.sysml",
        Swap("connection def ", "part def "),
        &["validateClassSpecialization"],
    ),
    (
        "ConnectionTest.sysml",
        Swap("then ", "; //"),
        &["validateConnectorBinarySpecialization"],
    ),
    (
        "ControlNodeTest.sysml",
        Swap("action def ", "part def "),
        &["validateControlNodeOwningType"],
    ),
    (
        "Features.kerml",
        Swap("composite ", "portion "),
        &["validateFeaturePortionNotVariable"],
    ),
    (
        "Conditional Succession Example-2.sysml",
        Swap("then ", "; //"),
        &["validateIfActionUsageParameters"],
    ),
    (
        "RootPackageTest.sysml",
        Swap("private ", "public "),
        &["validateImportTopLevelVisibility"],
    ),
    (
        "Dynamics.sysml",
        Swap("calc def ", "part def "),
        &[
            "validateInvocationExpressionInstantiatedType",
            "validateParameterMembershipOwningType",
            "validateReturnParameterMembershipOwningType",
        ],
    ),
    (
        "Dynamics.sysml",
        Swap("\n\t\tin ", "\n\t\tout "),
        &["validateInvocationExpressionParameterRedefinition"],
    ),
    (
        "ActionTest.sysml",
        Swap("first ", ""),
        &["validateMergeNodeOutgoingSuccessions"],
    ),
    (
        "Vehicle Analysis Demo.sysml",
        Swap("analysis def ", "part def "),
        &["validateObjectiveMembershipOwningType"],
    ),
    (
        "A-3-8-ChangingFeatureValues.kerml",
        Swap("\n\t\tin ", "\n\t\tout "),
        &["validateRedefinitionDirectionConformance"],
    ),
    (
        "Vehicle Analysis Demo.sysml",
        Swap("requirement def ", "part def "),
        &["validateRequirementConstraintMembershipOwningType"],
    ),
    (
        "Turbojet Stage Analysis.sysml",
        Swap("calc def ", "part def "),
        &["validateResultExpressionMembershipOwningType"],
    ),
    (
        "ViewTest.sysml",
        Swap("concern def ", "part def "),
        &["validateStakeholderMembershipOwningType"],
    ),
    (
        "AssignmentTest.sysml",
        Swap("state def ", "part def "),
        &["validateStateSubactionMembershipOwningType"],
    ),
    (
        "VariabilityTest.sysml",
        Swap("variation ", ""),
        &["validateVariantMembershipOwningNamespace"],
    ),
    (
        "ViewTest.sysml",
        Swap("view def ", "part def "),
        &["validateViewRenderingMembershipOwningType"],
    ),
    (
        "CarWithShapeAndCSG.sysml",
        Swap("part def ", "action def "),
        &["validateBehaviorSpecialization"],
    ),
    (
        "A-3-7-DecisionsAndMerges.kerml",
        Swap("behavior ", "struct "),
        &["validateStructureSpecialization"],
    ),
    (
        "ProductSelection_N_ary.kerml",
        Swap("member ", ""),
        &["validateFeatureCrossFeatureSpecialization"],
    ),
    (
        "AnalysisIndividualExample.sysml",
        Swap(" = ", " := "),
        &["validateFeatureIsVariable"],
    ),
    (
        "AnalysisIndividualExample.sysml",
        Swap("part def ", "attribute def "),
        &["validateOccurrenceUsageIndividualUsage"],
    ),
    (
        "JohnIndividualExample.sysml",
        Swap("individual ", ""),
        &["validateOccurrenceUsagePortionKind"],
    ),
    (
        "StructuredControlTest.sysml",
        Swap("in ", "inout "),
        &["validateForLoopActionUsageParameters"],
    ),
    (
        "TradeStudyTest.sysml",
        Swap("in ", "inout "),
        &["validateParameterMembershipParameterDirection"],
    ),
    (
        "ControlNodeTest.sysml",
        Swap("join", "fork"),
        &["validateForkNodeIncomingSuccessions"],
    ),
    (
        "ActionTest.sysml",
        Swap("if ", "while "),
        &["validateWhileLoopActionUsage"],
    ),
    (
        "Metadata Example-1.sysml",
        Swap(" : ", " :> "),
        &["validateMetadataFeatureAnnotatedElement"],
    ),
    (
        "Classifiers.kerml",
        Swap("specializes ", "conjugates "),
        &["validateSpecificationSpecificNotConjugated"],
    ),
    (
        "JohnIndividualExample.kerml",
        Swap("specializes ", "conjugates "),
        &["validateTypeAtMostOneConjugator"],
    ),
    (
        "ProductSelection_UnownedEnds.kerml",
        SwapNth("end ", "", 0),
        &["validateCrossSubsettingCrossingFeature"],
    ),
    (
        "FeatureChains.kerml",
        SwapNth("chains ", "subsets ", 1),
        &["validateFeatureChainingFeatureNotOne"],
    ),
    (
        "A-3-6-Sequences.kerml",
        Repeat(37),
        &["validateAssociationBinarySpecialization"],
    ),
    (
        "VerificationTest.sysml",
        Repeat(16),
        &["validateCaseDefinitionOnlyOneSubject"],
    ),
    (
        "10c-Fuel Economy Analysis.sysml",
        Drop(79),
        &["validateCaseDefinitionSubjectParameterPosition"],
    ),
    (
        "Trade Study Analysis Example.sysml",
        Repeat(25),
        &["validateCaseUsageOnlyOneObjective"],
    ),
    (
        "10c-Fuel Economy Analysis.sysml",
        Repeat(148),
        &["validateCaseUsageOnlyOneSubject"],
    ),
    (
        "10c-Fuel Economy Analysis.sysml",
        Drop(148),
        &["validateCaseUsageSubjectParameterPosition"],
    ),
    (
        "ExtendedOccurrences.kerml",
        Repeat(19),
        &["validateExpressionResultParameterMembership"],
    ),
    (
        "SysML v2 Spec Annex A SimpleVehicleModel.sysml",
        Drop(817),
        &["validateFlowPayloadFeature"],
    ),
    (
        "Fork Join Example.sysml",
        Repeat(22),
        &["validateJoinNodeOutgoingSuccessions"],
    ),
    (
        "ViewTest.sysml",
        Drop(10),
        &["validateRequirementDefinitionSubjectParameterPosition"],
    ),
    (
        "VehicleRequirementDerivation.sysml",
        Repeat(31),
        &["validateRequirementUsageOnlyOneSubject"],
    ),
    (
        "Viewpoint Example.sysml",
        Drop(22),
        &["validateRequirementUsageSubjectParameterPosition"],
    ),
];

impl Break {
    /// The model this break makes of that text, or nothing where the
    /// text does not have what it breaks -- which is a corpus that has
    /// moved under the test, and is reported as such.
    fn made_of(self, text: &str) -> Option<String> {
        match self {
            Swap(from, to) => text.contains(from).then(|| text.replace(from, to)),
            SwapNth(from, to, nth) => {
                let mut at = 0;
                for _ in 0..nth {
                    at += text[at..].find(from)? + from.len();
                }
                let found = at + text[at..].find(from)?;
                Some(format!(
                    "{}{to}{}",
                    &text[..found],
                    &text[found + from.len()..]
                ))
            }
            Drop(line) => {
                let mut lines: Vec<&str> = text.lines().collect();
                (line <= lines.len()).then(|| {
                    lines.remove(line - 1);
                    lines.join("\n")
                })
            }
            Repeat(line) => {
                let mut lines: Vec<&str> = text.lines().collect();
                (line <= lines.len()).then(|| {
                    lines.insert(line - 1, lines[line - 1]);
                    lines.join("\n")
                })
            }
        }
    }
}

#[test]
fn a_corpus_file_broken_on_purpose_is_reported_as_broken() {
    let Some(library) = library() else { return };
    let root = library.parent().expect("the library sits in the release");
    let mut corpus = sysml_semantics::model_files(&root.join("sysml/src"));
    corpus.extend(sysml_semantics::model_files(&root.join("kerml/src")));

    let mut ws = Workspace::new();
    ws.load_dir(&library).expect("the library loads");
    ws.resolve_all();

    for (name, broken, expected) in MUTATED {
        let path = corpus
            .iter()
            .find(|it| it.file_name().is_some_and(|it| it == *name))
            .unwrap_or_else(|| panic!("{name} is in the corpus"));
        let text = std::fs::read_to_string(path).expect("the corpus reads");
        let mutated = broken
            .made_of(&text)
            .unwrap_or_else(|| panic!("{name} no longer has what this breaks"));

        // one break at a time, each against a library that was resolved
        // once: cloning it costs a tenth of what loading it does
        let mut ws = ws.clone();
        let kerml = path.extension().is_some_and(|it| it == "kerml");
        let file = ws.add_file(
            if kerml {
                "mutated.kerml"
            } else {
                "mutated.sysml"
            },
            &mutated,
        );
        ws.resolve_files(&[file]);
        let found = ws.findings(&[file]);
        assert!(found.syntax.is_empty(), "{name}: does not parse");
        assert!(found.names.is_empty(), "{name}: does not resolve {found:?}");

        let fired: BTreeSet<&str> = ws
            .check_rules(&[file])
            .violations
            .iter()
            .map(|violation| violation.rule)
            .collect();
        for rule in *expected {
            assert!(fired.contains(rule), "{name}: {fired:?}");
        }
    }
}

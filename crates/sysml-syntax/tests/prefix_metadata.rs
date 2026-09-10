//! Every place the specification's BNF lets a `#keyword` stand.
//!
//! A user-defined keyword is a prefix metadata annotation, and the
//! grammar admits one almost everywhere a definition or a usage begins:
//! `DefinitionExtensionKeyword*` and `UsageExtensionKeyword*` appear in
//! seventeen productions of the SysML notation, and KerML's
//! `FeaturePrefix` ends in `( ownedRelationship += PrefixMetadataMember
//! )*`. A parser that takes them in most places and not the rest reads
//! a sound model as broken, which is worse than reading nothing: the
//! modeller is told the language forbids what it spells out.
//!
//! Four of these were rejected. Three are the members whose production
//! puts the keyword after the word that names the membership --
//! `SubjectUsage : ReferenceUsage = 'subject' UsageExtensionKeyword*
//! Usage`, and `actor` and `stakeholder` the same way -- and the fourth
//! is `ObjectiveRequirementUsage`, which is nothing but
//! `UsageExtensionKeyword* ConstraintUsageDeclaration RequirementBody`.
//! The KerML one is a feature whose prefix carries a multiplicity: `end
//! [0..1] #M feature x : X;`, where the `[0..1]` is the cross feature
//! `FeaturePrefix` allows an `end` before the metadata that follows it.
//!
//! They were found by running the OMG pilot implementation's own
//! parsing tests against this parser.

use sysml_syntax::{parse, parse_dialect, Dialect};

/// One shape the BNF admits, and the production that admits it.
const ADMITTED: &[(&str, &str)] = &[
    (
        "ExtendedDefinition : BasicDefinitionPrefix? DefinitionExtensionKeyword+ 'def' Definition",
        "#tag def Extended;",
    ),
    (
        "ExtendedUsage : UnextendedUsagePrefix UsageExtensionKeyword+ Usage",
        "#tag extended;",
    ),
    (
        "MetadataUsage : UsageExtensionKeyword* ( '@' | 'metadata' ) MetadataUsageDeclaration",
        "#tag @ Tag;",
    ),
    (
        "OccurrenceDefinitionPrefix ... DefinitionExtensionKeyword*",
        "#tag part def Tagged;",
    ),
    (
        "SubjectUsage : 'subject' UsageExtensionKeyword* Usage",
        "requirement r { subject #tag s; }",
    ),
    (
        "ActorUsage : 'actor' UsageExtensionKeyword* Usage",
        "requirement r { actor #tag a; }",
    ),
    (
        "StakeholderUsage : 'stakeholder' UsageExtensionKeyword* Usage",
        "requirement r { stakeholder #tag s; }",
    ),
    (
        "ObjectiveRequirementUsage : UsageExtensionKeyword* ConstraintUsageDeclaration \
         RequirementBody",
        "case c { objective #tag o; }",
    ),
    (
        "RequirementConstraintUsage : ( UsageExtensionKeyword* 'constraint' | \
         UsageExtensionKeyword+ ) ConstraintUsageDeclaration CalculationBody",
        "requirement r { assume #tag constraint c; require #tag constraint d; }",
    ),
    (
        "FramedConcernUsage : ( UsageExtensionKeyword* 'concern' | UsageExtensionKeyword+ ) ...",
        "requirement r { frame #tag concern c; }",
    ),
    (
        "RequirementVerificationUsage : ( UsageExtensionKeyword* 'requirement' | \
         UsageExtensionKeyword+ ) ...",
        "verification v { objective o { verify #tag requirement r; } }",
    ),
    (
        "ViewRenderingUsage : ( UsageExtensionKeyword* 'rendering' | UsageExtensionKeyword+ ) Usage",
        "view v { render #tag rendering r; }",
    ),
    (
        "VariantUsageMember : MemberPrefix 'variant' VariantUsageElement",
        "part def V { variant #tag part w; }",
    ),
    (
        "ReturnParameterMember : MemberPrefix? 'return' UsageElement",
        "calc def C { return #tag x; }",
    ),
];

/// The same for the KerML notation, whose `FeaturePrefix` puts its
/// `PrefixMetadataMember`s last -- after the direction, the modifiers,
/// and the cross feature an `end` may carry.
const ADMITTED_KERML: &[(&str, &str)] = &[
    (
        "FeaturePrefix : BasicFeaturePrefix ( PrefixMetadataMember )*",
        "#tag feature f : X;",
    ),
    (
        "FeaturePrefix : EndFeaturePrefix ( PrefixMetadataMember )*",
        "assoc A { end #tag feature x : X; end feature y : X; }",
    ),
    (
        "FeaturePrefix : EndFeaturePrefix OwnedCrossFeatureMember ( PrefixMetadataMember )*",
        "assoc A { end [0..1] #tag feature x : X; end [0..1] feature y : X; }",
    ),
    (
        "BasicFeaturePrefix direction, then the metadata",
        "behavior B { in #tag feature p : X; }",
    ),
];

#[test]
fn a_user_defined_keyword_stands_wherever_the_bnf_says_it_does() {
    for (production, shape) in ADMITTED {
        let text = format!("package P {{ metadata def tag; {shape} }}\n");
        let parsed = parse(&text);
        assert!(
            parsed.ok(),
            "{production}\n  {shape}\n  {:?}",
            parsed.errors()
        );
    }
    for (production, shape) in ADMITTED_KERML {
        let text = format!("package P {{ metaclass tag; classifier X; {shape} }}\n");
        let parsed = parse_dialect(&text, Dialect::KerML);
        assert!(
            parsed.ok(),
            "{production}\n  {shape}\n  {:?}",
            parsed.errors()
        );
    }
}

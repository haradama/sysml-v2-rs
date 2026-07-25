//! Exercises the generated metamodel exhaustively: every metaclass's name
//! round-trip, hierarchy, feature tables, and every enumeration literal.

use sysml_model::generated::{
    FeatureDirectionKind, PortionKind, RequirementConstraintKind, StateSubactionKind,
    TransitionFeatureKind, TriggerKind, VisibilityKind, ELEMENT_KINDS,
};
use sysml_model::{ElementKind, FeatureType, PrimitiveType};

#[test]
fn every_metaclass_is_consistent() {
    assert_eq!(ELEMENT_KINDS.len(), 175);
    let mut abstract_count = 0;
    let mut string_typed_features = 0;
    for &kind in ELEMENT_KINDS {
        // name round-trip
        assert_eq!(ElementKind::from_name(kind.name()), Some(kind));
        // everything is an Element, and the hierarchy is acyclic
        assert!(kind.is_a(ElementKind::Element), "{kind:?}");
        let ancestors = kind.ancestors();
        assert!(!ancestors.contains(&kind), "{kind:?} inherits itself");
        for sup in kind.direct_supertypes() {
            assert!(ancestors.contains(sup), "{kind:?} missing {sup:?}");
        }
        if kind.is_abstract() {
            abstract_count += 1;
        }
        // feature tables are well-formed
        for feature in kind.own_features() {
            assert!(!feature.name.is_empty());
            if feature.ty == FeatureType::Data(PrimitiveType::String) {
                string_typed_features += 1;
            }
            // and findable through the lookup API
            assert!(
                kind.feature(feature.name).is_some(),
                "{kind:?}.{}",
                feature.name
            );
        }
        assert!(kind.feature("definitelyNotAFeature").is_none());
    }
    // the metamodel has an abstract layer (Element, Relationship, ...) and
    // several string-typed attributes (declaredName, body, ...)
    assert_eq!(abstract_count, 8);
    assert!(string_typed_features > 5, "{string_typed_features}");
    assert_eq!(ElementKind::from_name("NotAClass"), None);
    assert!(!ElementKind::Element.is_a(ElementKind::Feature));
}

#[test]
fn every_enum_literal_round_trips() {
    use FeatureDirectionKind as F;
    use PortionKind as P;
    use RequirementConstraintKind as R;
    use StateSubactionKind as S;
    use TransitionFeatureKind as T;
    use TriggerKind as G;
    use VisibilityKind as V;

    macro_rules! roundtrip {
        ($ty:ident, [$($variant:expr),+ $(,)?]) => {
            for v in [$($variant),+] {
                assert_eq!($ty::from_literal(v.literal()), Some(v));
            }
            assert_eq!($ty::from_literal("definitely-not-a-literal"), None);
        };
    }

    roundtrip!(V, [V::Private, V::Protected, V::Public]);
    roundtrip!(F, [F::In, F::Inout, F::Out]);
    roundtrip!(P, [P::Timeslice, P::Snapshot]);
    roundtrip!(R, [R::Assumption, R::Requirement]);
    roundtrip!(S, [S::Entry, S::Do, S::Exit]);
    roundtrip!(T, [T::Guard, T::Effect, T::Trigger]);
    roundtrip!(G, [G::When, G::At, G::After]);
}

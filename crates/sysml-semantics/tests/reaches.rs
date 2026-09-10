//! Which names reach what, on the cases the OMG's own tools pin.
//!
//! The corpus says this resolver does not cry wolf: every reference in
//! the 403 published files resolves, so nothing here reports a sound
//! model as broken. It says nothing about the other direction. A
//! resolver that answered every name with the first element of that
//! name anywhere in the workspace would pass the corpus perfectly and
//! be wrong about every private member in it.
//!
//! The OMG pilot implementation carries a suite that asks exactly that
//! other question. Each of its Xpect tests is a small model with the
//! answers written beside it: this name resolves to that element, this
//! one resolves to nothing. Running the whole of it here is what found
//! the bugs the later models below are about. The models are
//! written fresh rather than copied -- the pilot is published under the
//! EPL and this is not -- but each states a rule the pilot states, in
//! its own words, and each is quoted back to the specification the
//! pilot and this implement.
//!
//! Nothing here loads the standard library: these are questions about
//! visibility and imports, which the library has no part in, and the
//! implicit semantic bases it would supply would only add names that
//! are not what is being asked about.

use sysml_model::ElementId;
use sysml_semantics::Workspace;

/// One model, and every name written in it that must resolve to
/// nothing. Exact in both directions: a name that stops resolving fails
/// here, and so does one that starts.
const REACHES: &[(&str, &str, &[&str])] = &[
    (
        // KerML 8.2.3.5: a qualified name reaches a namespace's public
        // members. `VisibilityTests_Import_Invalid_*` in the pilot's
        // suite is thirteen files saying so of a package.
        "what a package keeps to itself is not reachable from outside it",
        "package Vault {
    private class Sealed {
        public class Inside;
    }
    public class Open {
        private class Hidden;
        protected class Shared;
        public class Shown;
    }
}
package Outside {
    private import Vault::Sealed::*;
    class a :> Vault::Open::Shown;
    class b :> Vault::Sealed;
    class c :> Vault::Open::Hidden;
    class d :> Vault::Open::Shared;
}
",
        &[
            "Vault::Sealed",
            "Vault::Sealed",
            "Vault::Open::Hidden",
            "Vault::Open::Shared",
        ],
    ),
    (
        // The pilot's `VisibilityTests_ProtectedImport_*` say this in as
        // many words: a protected member answers to the specializing
        // type's own body and not to a qualified name written there.
        "a protected member is inherited, and still not reachable by a qualified name",
        "package Vault {
    public class Open {
        protected class Shared;
        public class Shown;
    }
}
package Heir {
    class Inherits :> Vault::Open {
        class byInheritance :> Shared;
        class andPublicToo :> Shown;
        class byQualifiedName :> Inherits::Shared;
    }
}
",
        &["Inherits::Shared"],
    ),
    (
        // `import A::Alias;` imports the membership, and a membership
        // carries the name the import wrote. Offering the element's own
        // name as well would resolve a name this file never wrote.
        "an alias is imported under the name the import wrote",
        "package Named {
    public class Original;
    public alias Nickname for Original;
}
package Borrower {
    private import Named::Nickname;
    class a :> Nickname;
    class b :> Original;
}
",
        &["Original"],
    ),
    (
        // Short names are names: the pilot's `ShortName_*` files write
        // every reference both ways round and expect the same element.
        "a short name answers wherever the declared name does",
        "package Short {
    class <'K1'> First {
        class <'K2'> Inner;
    }
    class second :> K1;
    class third :> First::K2;
    class fourth :> K1::Inner;
    class fifth :> K1::K2;
}
",
        &[],
    ),
    (
        // KerML 8.3.2.4.4: the importedMemberships of a recursive
        // MembershipImport "return at least the importedMembership",
        // and then, the member being a namespace, everything below it.
        // A NamespaceImport made recursive returns what is inside the
        // namespace and not the namespace. Reading `A::B::**` as the
        // second left `import P::C::**; feature x : C;` with nothing
        // named `C` -- which is what the pilot's `Import_Recursive6`
        // asks about.
        "`X::**` imports X as well as what is under it, and `X::*::**` does not",
        "package Deep {
    package Outer {
        class Held {
            class Nested;
        }
    }
}
package TakesTheMemberToo {
    private import Deep::Outer::Held::**;
    class a :> Held;
    class b :> Nested;
}
package TakesOnlyWhatIsInside {
    private import Deep::Outer::Held::*::**;
    class a :> Nested;
    class b :> Held;
}
",
        &["Held"],
    ),
    (
        // KerML 8.4.4.14: "All filterConditions are checked against
        // every Membership that would otherwise be imported into the
        // Package ... A Membership shall be imported if and only if
        // every filterCondition evaluates to true ... with any
        // MetadataFeature of the memberElement of the Membership as the
        // target Element." The pilot's `Import_Filtered` is
        // twenty-nine assertions saying it of the two spellings.
        //
        // The published models only ever write one annotation, or two
        // joined by `and` or `or`, so the operators below the last three
        // packages are the specification's rather than the corpus's.
        "a filter says which of the imported names to keep",
        "package Fleet {
    metaclass Safety;
    metaclass Security;
    classifier Group {
        classifier belt { @Safety; }
        classifier lock { @Security; }
        classifier alarm { @Safety; @Security; }
        classifier seat;
    }
    package ByFilter {
        public import Group::*;
        filter @Safety;
        classifier a :> belt;
        classifier b :> seat;
    }
    package ByBracket {
        public import Group::*[@Safety];
        classifier a :> belt;
        classifier b :> seat;
    }
    package Unfiltered {
        public import Group::*;
        classifier a :> belt;
        classifier b :> seat;
    }
    package BothAtOnce {
        public import Group::*;
        filter @Safety and @Security;
        classifier a :> alarm;
        classifier b :> belt;
    }
    package OneOrTheOther {
        public import Group::*;
        filter @Safety xor @Security;
        classifier a :> lock;
        classifier b :> alarm;
    }
    package NeitherOfThem {
        public import Group::*;
        filter not @Safety;
        classifier a :> seat;
        classifier b :> belt;
    }
}
",
        &["seat", "seat", "belt", "alarm", "belt"],
    ),
    (
        // The declaration is kept out of the walk so that a feature
        // with no name of its own cannot answer for the name it
        // redefines. That is about where the walk ends: a name may
        // still pass through the declaration on its way somewhere else.
        "a name may be reached through the very declaration it is written in",
        "package Through {
    classifier Outer specializes Outer::Inner {
        classifier Inner;
    }
    classifier Itself specializes Itself;
}
",
        &["Itself"],
    ),
    (
        // KerML 8.2.3.5.1: "the metaclass of the memberElement must
        // conform to the expected metaclass in the context of the name
        // resolution ... if the resulting Element does not have the
        // proper type for its context, then the qualified name has no
        // resolution". `Subsetting::subsettedFeature` and
        // `Redefinition::redefinedFeature` are both declared `Feature`.
        "what a feature subsets is a feature, however plainly the name is there",
        "package Kinds {
    classifier NotAFeature;
    classifier Holder {
        feature real;
        feature ok subsets real;
        feature wrong subsets NotAFeature;
        feature alsoWrong redefines NotAFeature;
    }
    classifier Fine specializes NotAFeature;
}
",
        &["NotAFeature", "NotAFeature"],
    ),
    (
        // An import that resolves to nothing used to be no finding at
        // all: `check` called the file sound and every name the file
        // expected from that import failed somewhere else, or -- in a
        // package that only re-exports -- went quietly missing. The
        // pilot reports each one where it is written, and so does this
        // now.
        "an import that names nothing is said so, where it is written",
        "package Real {
    public class Thing;
}
package Typo {
    private import Reall::*;
    private import Real::NoSuchThing;
    private import Real::*;
    class uses :> Thing;
}
",
        &["Reall", "Real::NoSuchThing"],
    ),
    (
        // `ConjugationPart = ( 'conjugates' | '~' ) OwnedConjugation`:
        // the two spellings are one relationship, and a conjugated type
        // has the features of the type it conjugates, directions
        // reversed. Read as an expression, `feature g ~ B::f;` declared
        // no `g` and named no `B::f` -- which the corpus writes twice,
        // in `Conjugation.kerml` and `Features.kerml`, and the pilot's
        // `MemberNameTests_NamedMemberFromConjugation` asks about.
        "a conjugation written `~` is the one written `conjugates`",
        "package Mirror {
    classifier <'A_Id'> A {
        in feature <'f_Id'> f;
    }
    classifier B conjugates A;
    feature g ~ B::f;
    feature h ~ B::f_Id;
    feature k ~ B::nothing;
    feature m :> g;
}
",
        &["B::nothing"],
    ),
    (
        // `checkMetadataFeatureSemanticSpecialization`: what semantic
        // metadata annotates specializes the metaclass's `baseType`,
        // whether the annotation is a keyword in front of the
        // declaration or `@B;` in its body. A classifier annotated with
        // a feature base specializes the feature's *types*: `C1` is a
        // `C`, and `y`, which only the feature `f` declares, is not
        // something a `C` has. The pilot's
        // `MetadataTests_SemanticMetadata_valid` writes the body form.
        "semantic metadata in a body makes its owner specialize the base type",
        "package Semantic {
    class C {
        feature x;
    }
    feature f : C {
        feature y;
    }
    metaclass Type;
    abstract metaclass SemanticMetadata {
        feature baseType;
    }
    abstract metaclass A :> SemanticMetadata {
        feature :>> baseType;
    }
    metaclass B :> A {
        feature :>> baseType = f meta Type;
    }
    metaclass D :> SemanticMetadata {
        feature :>> baseType = C meta Type;
    }
    class C1 {
        @B;
        feature :>> x;
        feature :>> y;
    }
    feature f1 {
        @B;
        feature :>> x;
        feature :>> y;
    }
    class C2 {
        @D;
        feature :>> x;
    }
    feature f2 {
        @D;
        feature :>> x;
    }
}
",
        &["y"],
    ),
    (
        // `checkFeatureValuationSpecialization`: a feature bound to a
        // value, with no type, direction or specialization of its own,
        // subsets what the value comes to -- so `c` in `b.c` is looked
        // up in what `a#(1).b` is. And `checkIndexExpressionResult
        // Specialization`: `a#(1)` is one of the `a`s, unless `a` is a
        // collection, whose `#` picks an element out of it -- an
        // element of an array has no `dimensions`. The pilot's
        // `ParsingTests_Indexing` writes both the index and the array.
        "a feature bound to a value has the members the value has",
        "package Indexing {
    classifier A {
        feature b : B;
    }
    classifier B {
        feature c;
    }
    feature a : A[*];
    feature b = a#(1).b;
    feature c = b.c;
    feature d = a.b;
    feature e = d.c;
    feature n = c.nothing;
    feature arr : Collections::Array;
    feature one = arr#(1, 3);
    feature dims = one.dimensions;
    feature all = arr.dimensions;
}
package Collections {
    classifier Collection;
    classifier Array :> Collection {
        feature elements;
        feature dimensions;
    }
}
",
        &["c::nothing", "one::dimensions"],
    ),
];

#[test]
fn a_name_reaches_what_the_pilot_implementation_says_it_reaches() {
    for (rule, source, expected) in REACHES {
        let mut ws = Workspace::new();
        let file = ws.add_file("reaches.kerml", source);
        ws.resolve_all();
        assert!(ws.file_parse(file).ok(), "{rule}: does not parse");
        let missing: Vec<&str> = ws.unresolved().iter().map(|it| it.name.as_str()).collect();
        assert_eq!(missing, *expected, "{rule}");
    }
}

/// The model behind a rule in the table above, resolved.
fn resolved(rule: &str) -> Workspace {
    let (_, source, _) = REACHES
        .iter()
        .find(|(it, ..)| *it == rule)
        .expect("a rule the table states");
    let mut ws = Workspace::new();
    ws.add_file("reaches.kerml", source);
    ws.resolve_all();
    ws
}

/// The pilot's `linkedName at B::f --> test.A.f`: what a conjugated
/// type is asked for is what the original declares, by either of its
/// names.
#[test]
fn a_member_asked_of_a_conjugated_type_is_the_original_type_s() {
    let ws = resolved("a conjugation written `~` is the one written `conjugates`");
    let to_f = ws
        .references()
        .iter()
        .filter(|it| ws.qualified_name_of(it.target) == "Mirror::A::f")
        .count();
    // `B::f` and `B::f_Id`
    assert_eq!(to_f, 2);
}

/// What the standard asks for is written into the model as well as
/// used to answer names: a classifier annotated with a feature base
/// specializes the feature's type, and the feature beside it subsets
/// the feature.
#[test]
fn semantic_metadata_in_a_body_is_materialized_as_the_standard_maps_it() {
    let mut ws = resolved("semantic metadata in a body makes its owner specialize the base type");
    ws.materialize_implied();
    let model = ws.model();
    let named = |name: &str| {
        model
            .ids()
            .find(|&id| model.name(id) == Some(name))
            .unwrap()
    };
    let implied = |of: &str, kind: sysml_model::ElementKind, to: &str| -> Vec<ElementId> {
        model
            .owned(named(of))
            .iter()
            .copied()
            .filter(|&child| {
                model.kind(child) == kind
                    && model.maybe(child, "isImplied") == Some(&sysml_model::Value::Bool(true))
                    && model.get(child, to) == Some(&sysml_model::Value::Ref(named("C")))
            })
            .collect()
    };
    use sysml_model::ElementKind::{FeatureTyping, Subclassification, Subsetting};
    assert_eq!(implied("C1", Subclassification, "superclassifier").len(), 1);
    assert_eq!(implied("C2", Subclassification, "superclassifier").len(), 1);
    assert_eq!(implied("f2", FeatureTyping, "type").len(), 1);
    // `f1` subsets `f`; `C1`, which is not a feature, relates to `f`
    // in no way at all
    let f = sysml_model::Value::Ref(named("f"));
    let to_f = |of: &str| {
        model
            .owned(named(of))
            .iter()
            .filter(|&&it| {
                model.kind(it) == Subsetting && model.get(it, "subsettedFeature") == Some(&f)
            })
            .count()
    };
    assert_eq!(to_f("f1"), 1);
    assert_eq!(to_f("C1"), 0);
    assert!(model
        .owned(named("C1"))
        .iter()
        .all(|&it| ["superclassifier", "type"]
            .iter()
            .all(|prop| model.maybe(it, prop) != Some(&f))));
}

/// `checkIndexExpressionResultSpecialization`, in the model: the result
/// of `a#(1)` subsets what `a` comes to, and the result of `arr#(1, 3)`
/// does not, `arr` being an array.
#[test]
fn an_index_expression_comes_to_one_of_what_it_indexes() {
    let ws = resolved("a feature bound to a value has the members the value has");
    let model = ws.model();
    let out = |of: ElementId| {
        model
            .owned(of)
            .iter()
            .copied()
            .find(|&it| model.maybe(it, "direction") == Some(&sysml_model::Value::EnumLit("out")))
            .expect("every expression hands its value back through a parameter")
    };
    let mut seen = Vec::new();
    for index in model
        .ids()
        .filter(|&it| model.kind(it) == sysml_model::ElementKind::IndexExpression)
    {
        // the sequence the expression indexes, and what it names
        let sequence = model
            .owned(index)
            .iter()
            .copied()
            .find(|&it| model.maybe(it, "direction") == Some(&sysml_model::Value::EnumLit("in")))
            .and_then(|argument| {
                model
                    .owned(argument)
                    .iter()
                    .copied()
                    .find(|&it| model.kind(it) == sysml_model::ElementKind::FeatureValue)
            })
            .and_then(|value| {
                model
                    .maybe(value, "value")
                    .and_then(sysml_model::Value::as_id)
            })
            .expect("an index expression indexes something");
        let named = model
            .maybe(sequence, "referent")
            .and_then(sysml_model::Value::as_id)
            .and_then(|it| model.name(it))
            .expect("what it indexes is a feature it refers to");
        let subsets_the_sequence = model.owned(out(index)).iter().any(|&it| {
            model.kind(it) == sysml_model::ElementKind::Subsetting
                && model.get(it, "subsettedFeature")
                    == Some(&sysml_model::Value::Ref(out(sequence)))
        });
        seen.push((named.to_string(), subsets_the_sequence));
    }
    seen.sort();
    assert_eq!(seen, [("a".to_string(), true), ("arr".to_string(), false)]);
}

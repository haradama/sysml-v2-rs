//! Edge-case coverage for the resolution machinery: getters, guards,
//! cycles, and the IDE query APIs.

use sysml_model::ElementKind;
use sysml_semantics::Workspace;
use sysml_syntax::TextSize;

/// Byte offset of `needle` within `text` (for cursor positions in tests).
fn offset_of(text: &str, needle: &str) -> TextSize {
    TextSize::from(text.find(needle).expect("needle present") as u32)
}

fn ws(files: &[(&str, &str)]) -> Workspace {
    let mut ws = Workspace::default();
    for (name, text) in files {
        ws.add_file(*name, text);
    }
    ws.resolve_all();
    ws
}

#[test]
fn getters_expose_workspace_structure() {
    let mut ws = ws(&[("a.sysml", "package A { part def X; }")]);
    assert_eq!(ws.file_count(), 1);
    assert_eq!(ws.file_name(0), "a.sysml");
    assert_eq!(ws.file_roots(0).len(), 1);
    assert!(ws.file_parse(0).ok());
    let root = ws.root();
    assert_eq!(ws.model().kind(root), ElementKind::Namespace);
    assert_eq!(ws.qualified_name_of(root), "");
    let pkg = ws.file_roots(0)[0];
    assert_eq!(ws.qualified_name_of(pkg), "A");
    assert!(ws.documentation_of(pkg).is_none());
    assert!(ws.element_ranges(root).is_none());
    // empty segments never resolve
    assert_eq!(ws.resolve_from(pkg, &[]), None);
    // resolve_files with no files is a no-op
    let stats = ws.resolve_files(&[]);
    assert_eq!(stats.resolved + stats.unresolved, 0);
}

#[test]
fn reference_and_definition_queries() {
    let text = "package P {\n    part def Vehicle;\n    part car : Vehicle;\n}\n";
    let ws = ws(&[("m.sysml", text)]);

    // the file contains exactly one resolved reference: `car : Vehicle`
    assert_eq!(ws.references().len(), 1);
    let hit = *ws
        .reference_at(0, offset_of(text, ": Vehicle") + TextSize::from(2))
        .unwrap();
    assert_eq!(ws.model().name(hit.target), Some("Vehicle"));
    assert_eq!(ws.references_to(hit.target).count(), 1);
    assert!(ws.reference_at(0, TextSize::from(0)).is_none());

    // definition_at on the declaration name, and a miss
    let name_pos = offset_of(text, "Vehicle;") + TextSize::from(1);
    assert_eq!(
        ws.definition_at(0, name_pos)
            .and_then(|d| ws.model().name(d).map(String::from)),
        Some("Vehicle".to_string())
    );
    assert!(ws.definition_at(0, TextSize::from(0)).is_none());
}

#[test]
fn callable_at_misses() {
    let mut ws = ws(&[(
        "m.sysml",
        "package P { calc def Sum { in a; } attribute s = Sum(1); attribute t = (2); }",
    )]);
    let text = "package P { calc def Sum { in a; } attribute s = Sum(1); attribute t = (2); }";
    // inside `Sum(1)` — resolves
    let call_offset = TextSize::from(text.find("(1)").unwrap() as u32 + 1);
    let (target, active) = ws.callable_at(0, call_offset).unwrap();
    assert_eq!(ws.model().name(target), Some("Sum"));
    assert_eq!(active, 0);
    assert_eq!(ws.parameters_of(target), vec!["in a".to_string()]);
    // inside `(2)` — a parenthesized expression, not a call
    let paren_offset = TextSize::from(text.find("(2)").unwrap() as u32 + 1);
    assert!(ws.callable_at(0, paren_offset).is_none());
    // offset 0 — no surrounding arg list
    assert!(ws.callable_at(0, TextSize::from(0)).is_none());
    // out-of-file index
    assert!(ws.callable_at(9, TextSize::from(0)).is_none());
}

#[test]
fn parameters_render_directions_and_types() {
    let mut ws = ws(&[(
        "m.sysml",
        "package P { attribute def Real; calc def F { in a : Real; out b; inout c : Real; attribute plain; } }",
    )]);
    let pkg = ws.file_roots(0)[0];
    let f = ws.resolve_from(pkg, &["P".into(), "F".into()]).unwrap();
    assert_eq!(
        ws.parameters_of(f),
        vec!["in a : Real", "out b", "inout c : Real"]
    );
}

#[test]
fn visible_names_and_shadowing() {
    let text = "package Lib { part def Widget; part def Hidden; }\npackage App {\n    import Lib::Widget;\n    part def Local;\n    part def Widget;\n    part inner : Local {\n        part leaf;\n    }\n}\n";
    let mut ws = ws(&[("m.sysml", text)]);
    let names = ws.visible_names(0, offset_of(text, "part inner"));
    let labels: Vec<&str> = names.iter().map(|(n, _)| n.as_str()).collect();
    assert!(labels.contains(&"Local"), "{labels:?}");
    assert!(labels.contains(&"Widget"), "{labels:?}");
    // shadowing: only one Widget entry survives deduplication
    assert_eq!(labels.iter().filter(|n| **n == "Widget").count(), 1);
}

#[test]
fn protected_members_inherit_but_stay_hidden() {
    let ws = ws(&[(
        "m.sysml",
        "package P {\n    attribute def Real;\n    part def Base { protected attribute guarded : Real; }\n    part def Sub :> Base { attribute :>> guarded; }\n    part x : P::Base::guarded;\n}\n",
    )]);
    // inherited redefinition of the protected member resolved; the external
    // qualified path did not
    assert_eq!(ws.unresolved().len(), 1);
    assert!(ws.unresolved()[0].name.contains("guarded"));
}

#[test]
fn a_dependency_names_its_clients_and_suppliers() {
    let ws = ws(&[(
        "m.sysml",
        "package P {\n    part def A;\n    part def B;\n    dependency Use from A to B;\n    dependency A to NotThere;\n}\n",
    )]);
    // the supplier of the second one is a reference like any other, and
    // saying nothing about a name that is not there would leave the
    // dependency pointing at nothing
    assert_eq!(ws.unresolved().len(), 1);
    assert_eq!(ws.unresolved()[0].name, "NotThere");
}

#[test]
fn recursive_imports_reexport_descendants() {
    let ws = ws(&[
        ("a.sysml", "package A { package Deep { part def Buried; } }"),
        ("b.sysml", "package B { public import A::**; }"),
        ("c.sysml", "package C { part x : B::Buried; }"),
    ]);
    assert_eq!(ws.unresolved().len(), 0, "{:?}", ws.unresolved());
}

#[test]
fn cycles_do_not_hang() {
    let mut ws = ws(&[(
        "m.sysml",
        "package A { public import B::*; alias L for M; alias M for L; }\npackage B { public import A::*; }\npackage C { part x : A::Nothing; part y : A::L; }",
    )]);
    // both lookups terminate (unresolved, but no hang / stack overflow),
    // and enumeration through the cyclic imports terminates too. Four,
    // because an alias that names an alias that names it back names
    // nothing, and each of the two says so.
    assert_eq!(ws.unresolved().len(), 4, "{:?}", ws.unresolved());
    let names = ws.visible_names(0, TextSize::from(10));
    assert!(names.iter().any(|(n, _)| n == "L"), "{names:?}");
}

#[test]
fn semantic_metadata_without_base_type_is_ignored() {
    let ws = ws(&[(
        "m.sysml",
        "package P {\n    metadata def plain;\n    #plain part def X { attribute a; }\n    part def Marker { }\n}\n",
    )]);
    assert_eq!(ws.unresolved().len(), 0, "{:?}", ws.unresolved());
}

#[test]
fn callable_at_more_misses_and_index_expressions() {
    let mut empty = Workspace::new();
    empty.add_file("empty.sysml", "");
    assert!(empty.callable_at(0, TextSize::from(0)).is_none());

    let text = "package P { attribute z = a#(1); }";
    let mut ws = ws(&[("m.sysml", text)]);
    // inside `#(1)` — an index, not a call
    let offset = TextSize::from(text.find("(1)").unwrap() as u32 + 1);
    assert!(ws.callable_at(0, offset).is_none());
}

#[test]
fn parameters_skip_reified_and_nested_members() {
    let mut ws = ws(&[(
        "m.sysml",
        "package P { calc def Base; calc def F :> Base { part def Nested; in a; } }",
    )]);
    let pkg = ws.file_roots(0)[0];
    let f = ws.resolve_from(pkg, &["P".into(), "F".into()]).unwrap();
    // the reified Subclassification and the nested definition are skipped
    assert_eq!(ws.parameters_of(f), vec!["in a".to_string()]);
}

#[test]
fn visible_names_through_imports_and_ends() {
    let text = "package Lib { private classifier Secret; classifier Open; }\npackage A {\n    public import all Lib::*;\n    import Nowhere::*;\n    assoc R { end feature s : Open { feature nested; } feature marker; }\n}\n";
    let mut ws = ws(&[("k.kerml", text)]);
    let names = ws.visible_names(0, offset_of(text, "feature marker"));
    let labels: Vec<&str> = names.iter().map(|(n, _)| n.as_str()).collect();
    assert!(labels.contains(&"s"), "{labels:?}");
    assert!(labels.contains(&"marker"), "{labels:?}");
    // what an end declares is in the end's namespace and not the
    // association's -- `nested` is written `s::nested`
    assert!(!labels.contains(&"nested"), "{labels:?}");
    // `import all` exposes even private members
    assert!(labels.contains(&"Secret"), "{labels:?}");
}

/// `import Q::**` reaches what is nested in Q, and lookup has always
/// followed it there. What may be written at a point is the same
/// question asked a second way, and it used to stop at Q's own members
/// -- so a name the model resolves was one the editor never offered.
#[test]
fn a_recursive_import_offers_what_is_nested_in_it() {
    let text = "package Q {\n    class A;\n    package Q2 { class F; }\n}\npackage S {\n    public import Q::**;\n    class Z :> F;\n    class Y :> A;\n}\n";
    let mut deep = ws(&[("k.kerml", text)]);
    assert!(deep.unresolved().is_empty(), "{:?}", deep.unresolved());
    let names = deep.visible_names(0, offset_of(text, "class Z"));
    let labels: Vec<&str> = names.iter().map(|(n, _)| n.as_str()).collect();
    assert!(labels.contains(&"A"), "a member of Q: {labels:?}");
    assert!(labels.contains(&"F"), "nested in Q: {labels:?}");
    // a plain `::*` still stops at the members it names
    let shallow = "package Q {\n    class A;\n    package Q2 { class F; }\n}\npackage S {\n    public import Q::*;\n    class Y :> A;\n}\n";
    let mut only = ws(&[("k.kerml", shallow)]);
    let names = only.visible_names(0, offset_of(shallow, "class Y"));
    let labels: Vec<&str> = names.iter().map(|(n, _)| n.as_str()).collect();
    assert!(labels.contains(&"A"), "{labels:?}");
    assert!(!labels.contains(&"F"), "{labels:?}");
}

/// A feature with no name of its own answers to the name of what it
/// redefines -- which is the very thing being looked up when that
/// redefinition is resolved. Letting it match itself makes the answer
/// its own premise, and a model naming something that exists nowhere is
/// then reported as sound.
#[test]
fn a_borrowed_name_cannot_answer_the_question_it_came_from() {
    let ws = ws(&[(
        "m.sysml",
        "package P {\n\tpart def D { attribute :>> nowhere; }\n\tpart def E { attribute p4 :> p4; }\n}\n",
    )]);
    let names: Vec<&str> = ws.unresolved().iter().map(|u| u.name.as_str()).collect();
    // nothing anywhere is called `nowhere`, and saying so is the point
    assert!(names.contains(&"nowhere"), "{names:?}");
    // a feature that declares its own name may still refer to itself
    assert!(!names.contains(&"p4"), "{names:?}");
}

/// `$` is the root of the workspace; `'$'` is a package someone named
/// `$`. They unquote to the same three characters, so a resolver that
/// works in strings alone reads the second as the first and quietly
/// answers about the wrong package.
#[test]
fn a_package_named_like_the_root_is_not_the_root() {
    let ws = ws(&[(
        "k.kerml",
        "package Outer {\n    package Objects { class Object { feature here; } }\n    package '$' { class Objects { class Object { feature there; } } }\n    class A :> '$'::Objects::Object { feature :>> there; }\n    class B :> Objects::Object { feature :>> here; }\n}\n",
    )]);
    assert!(ws.unresolved().is_empty(), "{:?}", ws.unresolved());
}

/// A redefinition replaces what it redefines, so when two supertypes
/// both answer to a name and one redefines the other's answer, the
/// redefining one is the member -- whichever order they were written in.
#[test]
fn the_most_redefined_supertype_member_wins() {
    for order in ["A, B", "B, A"] {
        let ws = ws(&[(
            "k.kerml",
            &format!(
                "package R {{\n    classifier A {{ feature f; }}\n    classifier B specializes A {{ feature redefines f {{ feature g; }} }}\n    classifier C specializes {order} {{ feature subsets f {{ feature redefines g; }} }}\n}}\n"
            ),
        )]);
        assert!(
            ws.unresolved().is_empty(),
            "specializes {order}: {:?}",
            ws.unresolved()
        );
    }
}

/// Five ways a model says "the thing you already have", each of which
/// this resolver used to miss -- and miss quietly, because the feature
/// doing the saying answered to the name it was asking about.
#[test]
fn what_a_model_means_by_naming_something_it_already_has() {
    // `include x[0..*]` — the brackets are how many times, not which one
    let include_ws = ws(&[(
        "u.sysml",
        "package U {\n\tuse case def Fuel { actor fueler; }\n\tuse case def Drive { include Fuel[0..*] { actor :>> fueler; } }\n}\n",
    )]);
    assert!(
        include_ws.unresolved().is_empty(),
        "include: {:?}",
        include_ws.unresolved()
    );

    // `render asElementTable` renders by what it names, as `perform` does
    let render_ws = ws(&[(
        "v.sysml",
        "package V {\n\trendering def AsTable { view columnView; }\n\tview v { render AsTable { view :>> columnView; } }\n}\n",
    )]);
    assert!(
        render_ws.unresolved().is_empty(),
        "render: {:?}",
        render_ws.unresolved()
    );

    // a usage has one `objective`, so its own stands for its type's
    let objective_ws = ws(&[(
        "w.sysml",
        "package W {\n\trequirement def Mass;\n\tverification def MassTest { objective o { verify requirement massRequirement : Mass; } }\n\trequirement r : Mass;\n\tverification t : MassTest { objective mine { verify r :>> massRequirement; } }\n}\n",
    )]);
    assert!(
        objective_ws.unresolved().is_empty(),
        "objective: {:?}",
        objective_ws.unresolved()
    );

    // a feature redeclared by naming it the same redefines the inherited one
    let same_ws = ws(&[(
        "x.kerml",
        "package X {\n\tstruct S { feature q; }\n\tbehavior B { in p : S; }\n\tbehavior C specializes B { in p { feature redefines q; } }\n}\n",
    )]);
    assert!(
        same_ws.unresolved().is_empty(),
        "same name: {:?}",
        same_ws.unresolved()
    );

    // `variant x;` names a usage the model already has; `variant part x;`
    // declares a new one
    let variant_ws = ws(&[(
        "y.sysml",
        "package Y {\n\tpart def Engine;\n\tpart engine : Engine { port autoPort; }\n\tpart big :> engine;\n\tvariation part def Choices :> Engine {\n\t\tvariant big { port :>> autoPort; }\n\t\tvariant part fresh;\n\t}\n}\n",
    )]);
    assert!(
        variant_ws.unresolved().is_empty(),
        "variant: {:?}",
        variant_ws.unresolved()
    );
}

/// A keyword whose base type is chosen by a condition names both, and
/// which one an element gets is decided by an expression this resolver
/// does not evaluate. A name either of them declares is one the model
/// can mean, so both are taken.
#[test]
fn a_base_type_behind_a_condition_offers_every_side_of_it() {
    let ws = ws(&[(
        "m.sysml",
        "package P {\n    metadata def SemanticMetadata { attribute baseType; }\n    part def Left { attribute onlyLeft; }\n    part def Right { attribute onlyRight; }\n    metadata def Either :> SemanticMetadata {\n        :>> baseType = if true ? Left else Right;\n    }\n    #Either part def W { attribute :>> onlyLeft; attribute :>> onlyRight; }\n}\n",
    )]);
    assert!(ws.unresolved().is_empty(), "{:?}", ws.unresolved());
}

/// What is wrong with a model is worked out once, and the command line,
/// the language server and the MCP server each render that. They used to
/// work it out for themselves, and one of them left out the syntax half,
/// so `sysml check` called a file that does not parse sound.
#[test]
fn what_is_wrong_is_said_once_and_kept_apart() {
    let mut ws = ws(&[
        (
            "bad.sysml",
            "package P {\n\tpart def A;\n\tpart b : Missing;\n",
        ),
        (
            "good.sysml",
            "package Q {\n\tpart def C;\n\tpart d : C;\n}\n",
        ),
    ]);
    // `ws` has resolved once already; asking again must not double what
    // it found
    ws.resolve_all();

    let all = ws.findings(&[]);
    assert_eq!(all.syntax.len(), 1, "{:?}", all.syntax);
    assert!(all.syntax[0].what.contains("expected"), "{:?}", all.syntax);
    assert_eq!(all.names.len(), 1, "{:?}", all.names);
    assert_eq!(all.names[0].what, "Missing");

    // asked about one file, it answers about that file
    let good = ws.findings(&[1]);
    assert!(good.syntax.is_empty() && good.names.is_empty(), "{good:?}");
    let bad = ws.findings(&[0]);
    assert_eq!(bad.syntax.len(), 1);
    assert_eq!(bad.names.len(), 1);
    assert_eq!(bad.syntax[0].file, 0);
}

/// What is wrong with a model, asked once. Three front ends render this
/// -- the command line, the language server, the MCP server -- and when
/// each worked it out for itself one of them forgot the syntax half and
/// called a file that does not parse sound.
#[test]
fn findings_keep_the_syntax_apart_from_the_names() {
    let mut both = ws(&[
        ("broken.sysml", "package P {\n\tpart def A;\n"),
        ("named.sysml", "package Q {\n\tpart b : Missing;\n}\n"),
    ]);
    both.resolve_all();

    let all = both.findings(&[]);
    assert_eq!(all.syntax.len(), 1, "{:?}", all.syntax);
    assert!(all.syntax[0].what.contains("expected"), "{:?}", all.syntax);
    assert_eq!(all.syntax[0].file, 0);
    assert_eq!(all.names.len(), 1, "{:?}", all.names);
    assert_eq!(all.names[0].what, "Missing");
    assert_eq!(all.names[0].file, 1);

    // one file at a time is the same answer, narrowed
    assert_eq!(both.findings(&[0]).syntax, all.syntax);
    assert!(both.findings(&[0]).names.is_empty());
    assert!(both.findings(&[1]).syntax.is_empty());
    assert_eq!(both.findings(&[1]).names, all.names);

    // and a model with nothing wrong says so
    let mut clean = ws(&[("ok.sysml", "package R {\n\tpart def A;\n\tpart a : A;\n}\n")]);
    clean.resolve_all();
    assert_eq!(clean.findings(&[]), Default::default());
}

/// Finding a name is worked out once, so the language server's symbol
/// search and the MCP server's library search answer the same. They used
/// to sort differently, and only one of them put an exact match first --
/// asking for `Natural` and being handed two SI units before
/// `ScalarValues::Natural` is the difference between a useful answer and
/// one that has to be read through.
#[test]
fn a_search_puts_what_was_asked_for_first() {
    let ws = ws(&[(
        "s.sysml",
        "package P {\n\tpart def NaturalCapacity;\n\tpart def UnnaturalThing;\n\tpart def Natural;\n\tpart def NaturalX;\n}\n",
    )]);
    let names = |query: &str, limit: usize| -> Vec<String> {
        ws.search_names(query, limit)
            .into_iter()
            .map(|id| ws.model().name(id).unwrap_or_default().to_string())
            .collect()
    };
    // exact, then what starts with it (shorter first), then the rest
    assert_eq!(
        names("natural", 10),
        ["Natural", "NaturalX", "NaturalCapacity", "UnnaturalThing"]
    );
    // the limit takes the best, not the first the model happens to hold
    assert_eq!(names("natural", 2), ["Natural", "NaturalX"]);
    // an empty query is everything, which is what a symbol picker opens with
    assert_eq!(names("", 10).len(), 5);
    assert!(names("nothing here", 10).is_empty());
}

/// How visible a member is was worked out twice from the same syntax:
/// once into the model, once again by the resolver -- and they looked at
/// different parts of it. The model saw a `private` written on the
/// wrapper of a declaration; the resolver, which only read the
/// declaration's own first few tokens, did not. Now the resolver reads
/// what the model recorded, and adds only the rule that is its own: an
/// import with nothing written keeps to itself.
#[test]
fn how_visible_a_member_is_is_decided_once() {
    let ws = ws(&[(
        "v.sysml",
        "package Outer {\n\tpackage Lib {\n\t\tprivate part def Hidden;\n\t\tpart def Open;\n\t}\n\tpackage User {\n\t\tprivate import Lib::*;\n\t\tpart a : Open;\n\t}\n\tpackage Stranger {\n\t\tpart b : Lib::Hidden;\n\t}\n}\n",
    )]);
    let names: Vec<&str> = ws.unresolved().iter().map(|u| u.name.as_str()).collect();
    // what the library keeps to itself is not reachable from outside it
    assert!(names.contains(&"Lib::Hidden"), "{names:?}");
    // what it publishes is, through an import
    assert!(!names.contains(&"Open"), "{names:?}");
}

#[test]
fn broken_aliases_and_redefinitions_do_not_block_lookup() {
    let ws = ws(&[(
        "m.sysml",
        "package P {
    alias broken for Nothing;
    part def D { attribute :>> ; attribute good; }
    part d : D { attribute :>> good; }
    part u : broken;
}
",
    )]);
    // `good` resolves even though a sibling has an unresolvable effective
    // name; `broken` and the empty redefinition stay unresolved
    let names: Vec<&str> = ws.unresolved().iter().map(|u| u.name.as_str()).collect();
    assert!(names.contains(&"broken"), "{names:?}");
    assert!(!names.contains(&"good"), "{names:?}");
}

#[test]
fn self_annotating_metadata_terminates() {
    let mut ws = ws(&[(
        "m.sysml",
        "package P {
    metadata def SemanticMetadata { attribute baseType; }
    #m2 metadata def m2 :> SemanticMetadata { :>> baseType = q meta X; }
    #nope part def Y;
    part yy : Y { attribute :>> ghost; }
}
",
    )]);
    // enumeration must terminate despite the self-annotation, and the
    // package members stay visible
    let names = ws.visible_names(0, TextSize::from(20));
    assert!(names.iter().any(|(n, _)| n == "m2"), "{names:?}");
}

#[test]
fn implicit_supertypes_of_statement_features() {
    // succession / inv / binding trigger their implicit-supertype table rows
    let ws = ws(&[(
        "k.kerml",
        "package K {
    feature a;
    feature b;
    succession s { :>> zz; }
    inv i { :>> zz; }
    binding bi { :>> zz; }
}
",
    )]);
    // What matters here is that the succession/inv/binding
    // implicit-supertype rows executed. Nothing is called `zz`, and each
    // redefinition says so rather than answering to itself.
    let names: Vec<&str> = ws.unresolved().iter().map(|u| u.name.as_str()).collect();
    assert_eq!(names, ["zz", "zz", "zz"], "{:?}", ws.unresolved());
}

#[test]
fn load_dir_ignores_missing_directories() {
    let mut ws = Workspace::new();
    let count = ws
        .load_dir(std::path::Path::new("/definitely/not/a/dir"))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn plain_imports_hide_private_members_in_completion() {
    let mut ws = ws(&[(
        "m.sysml",
        "package Lib2 { private part def Hidden2; part def Shown; }\npackage B {\n    public import Lib2::*;\n    part here;\n}\n",
    )]);
    let names = ws.visible_names(0, sysml_syntax::TextSize::from(95));
    let labels: Vec<&str> = names.iter().map(|(n, _)| n.as_str()).collect();
    assert!(labels.contains(&"Shown"), "{labels:?}");
    assert!(!labels.contains(&"Hidden2"), "{labels:?}");
}

#[test]
fn degenerate_semantic_metadata_values() {
    let ws = ws(&[(
        "m.sysml",
        "package P {\n    metadata def SemanticMetadata { attribute baseType; }\n    part causes2;\n    metadata def c1 :> SemanticMetadata { :>> baseType = causes2 meta X; }\n    metadata def c2 :> SemanticMetadata { :>> baseType = causes2 meta X; }\n    #c1 #c2 part def Doubly { attribute da; }\n    part dd : Doubly { attribute :>> nowhere; }\n    metadata def selfish :> SemanticMetadata { :>> baseType = target meta X; }\n    part def Base2 { attribute deep; }\n    #selfish part def target :> Base2 { }\n    part tt : target { attribute :>> deep; }\n    metadata def weird :> SemanticMetadata { :>> baseType = 5.?{ }; }\n    #weird part def W { attribute inner; }\n    part w : W { attribute :>> nothere; }\n}\n",
    )]);
    // The self-referential base is skipped and the degenerate value
    // yields no base, so what those two usages redefine is nowhere to be
    // found -- which they now say. What matters is that both
    // semantic_base edge branches executed.
    let names: Vec<&str> = ws.unresolved().iter().map(|u| u.name.as_str()).collect();
    assert_eq!(names, ["nowhere", "nothere"], "{:?}", ws.unresolved());
}

#[test]
fn connector_statements_become_elements_with_resolved_ends() {
    let src = "part def Wheel { port hub; }\n\
               part def Axle { port mount; }\n\
               part def Car {\n\
               \tpart w : Wheel;\n\
               \tpart a : Axle;\n\
               \tconnect w.hub to a.mount;\n\
               \tallocate w to a;\n\
               \tbind w = a;\n\
               }\n";
    let ws = ws(&[("c.sysml", src)]);
    assert!(ws.unresolved().is_empty(), "{:?}", ws.unresolved());

    let model = ws.model();
    let kinds: Vec<ElementKind> = model
        .ids()
        .map(|id| model.kind(id))
        .filter(|k| {
            matches!(
                k,
                ElementKind::ConnectionUsage
                    | ElementKind::AllocationUsage
                    | ElementKind::BindingConnectorAsUsage
            )
        })
        .collect();
    assert_eq!(
        kinds,
        [
            ElementKind::ConnectionUsage,
            ElementKind::AllocationUsage,
            ElementKind::BindingConnectorAsUsage,
        ]
    );

    // each connector records what its operands resolved to
    let connection = model
        .ids()
        .find(|id| model.kind(*id) == ElementKind::ConnectionUsage)
        .unwrap();
    let Some(sysml_model::Value::RefList(ends)) = model.get(connection, "relatedFeature") else {
        panic!("no relatedFeature on the connection");
    };
    let names: Vec<&str> = ends.iter().filter_map(|e| model.name(*e)).collect();
    assert_eq!(names, ["hub", "mount"]);
}

#[test]
fn an_unresolvable_connector_end_is_reported() {
    let ws = ws(&[(
        "c.sysml",
        "part def P {\n\tconnect nowhere to alsoNowhere;\n}\n",
    )]);
    let names: Vec<&str> = ws.unresolved().iter().map(|u| u.name.as_str()).collect();
    assert_eq!(names, ["nowhere", "alsoNowhere"]);
}

#[test]
fn a_performed_usage_answers_to_the_performed_name() {
    let src = "action def GT;\n\
               action pp { action gt : GT; }\n\
               part def TG;\n\
               part tg : TG { perform pp.gt; }\n\
               part def Other;\n\
               part o : Other;\n\
               connect tg.gt to o;\n";
    let ws = ws(&[("p.sysml", src)]);
    assert!(ws.unresolved().is_empty(), "{:?}", ws.unresolved());
}

#[test]
fn an_inline_message_declaration_survives_its_succession() {
    // `then message m2 of T;` is parsed flat, so the statement itself has
    // to become the message rather than a succession that swallows it
    let ws = ws(&[(
        "m.sysml",
        "item def T;\n\
         occurrence def I {\n\
         \tref part a { event m2.x; }\n\
         \tmessage m1 of T;\n\
         \tthen message m2 of T;\n\
         }\n",
    )]);
    let model = ws.model();
    let interaction = model.ids().find(|id| model.name(*id) == Some("I")).unwrap();
    let messages: Vec<&str> = model
        .owned(interaction)
        .iter()
        .filter(|&&id| model.kind(id) == ElementKind::FlowUsage)
        .filter_map(|&id| model.name(id))
        .collect();
    assert_eq!(messages, ["m1", "m2"]);
}

#[test]
fn a_succession_chain_declares_the_nodes_it_names() {
    // the shorthand `then <declaration>;` both sequences the flow and
    // declares the node, which must belong to the enclosing behaviour
    let src = "action def A {\n\
               \taction seed;\n\
               \tfirst seed;\n\
               \tthen merge continue;\n\
               \tthen action b;\n\
               \tthen continue;\n\
               }\n";
    let ws = ws(&[("a.sysml", src)]);
    assert!(ws.unresolved().is_empty(), "{:?}", ws.unresolved());

    let model = ws.model();
    let action_def = model
        .ids()
        .find(|id| model.kind(*id) == ElementKind::ActionDefinition)
        .unwrap();
    let members: Vec<&str> = model
        .owned(action_def)
        .iter()
        .filter_map(|&id| model.name(id))
        .collect();
    assert_eq!(members, ["seed", "continue", "b"]);
    let merge = model
        .owned(action_def)
        .iter()
        .find(|&&id| model.name(id) == Some("continue"))
        .unwrap();
    assert_eq!(model.kind(*merge), ElementKind::MergeNode);
}

#[test]
fn a_kerml_connector_gets_its_library_base_type() {
    let ws = ws(&[(
        "c.kerml",
        "class C {\n\
         \tfeature a;\n\
         \tfeature b;\n\
         \tconnector c1 from a to b;\n\
         \tfeature d subsets c1.a;\n\
         }\n",
    )]);
    let model = ws.model();
    let connector = model
        .ids()
        .find(|&id| model.kind(id) == ElementKind::Connector)
        .expect("the connector became an element");
    assert_eq!(model.name(connector), Some("c1"));
    // reaching through `c1` asks for its supertypes, which is where the
    // implicit `Links::links` base comes in -- not loaded here, so the
    // chained lookup has nothing to find
    assert_eq!(ws.unresolved().len(), 1, "{:?}", ws.unresolved());
}

#[test]
fn only_a_reference_right_after_then_is_a_succession_end() {
    // `then accept sig ...` and `then timeslice t {}` declare rather than
    // point at something, so their names must not be resolved as ends
    let ws = ws(&[(
        "s.sysml",
        "attribute def S;\n\
         action def A {\n\
         \taction a;\n\
         \tthen a;\n\
         \tthen accept sig : S;\n\
         }\n",
    )]);
    assert!(ws.unresolved().is_empty(), "{:?}", ws.unresolved());
}

#[test]
fn an_unresolvable_trigger_payload_type_is_reported() {
    let ws = ws(&[(
        "t.sysml",
        "state def S {\n\
         \tstate a;\n\
         \tstate b;\n\
         \ttransition t1 first a accept pub : NoSuchSignal then b;\n\
         }\n",
    )]);
    let names: Vec<&str> = ws.unresolved().iter().map(|u| u.name.as_str()).collect();
    assert_eq!(names, ["NoSuchSignal"]);
}

#[test]
fn a_connection_written_as_a_usage_still_relates_its_ends() {
    // `connection c : L connect a to b;` is a usage, not a connector
    // statement, so its ends arrive by a different route
    let ws = ws(&[(
        "c.sysml",
        "part def W { port hub; }\n\
         part def A { port mount; }\n\
         connection def L;\n\
         part def Car {\n\
         \tpart w : W;\n\
         \tpart a : A;\n\
         \tconnection : L connect w.hub to a.mount;\n\
         }\n",
    )]);
    assert!(ws.unresolved().is_empty(), "{:?}", ws.unresolved());

    let model = ws.model();
    let connection = model
        .ids()
        .find(|&id| model.kind(id) == ElementKind::ConnectionUsage)
        .unwrap();
    let Some(sysml_model::Value::RefList(ends)) = model.get(connection, "relatedFeature") else {
        panic!("the usage form recorded no ends");
    };
    let names: Vec<&str> = ends.iter().filter_map(|&e| model.name(e)).collect();
    assert_eq!(names, ["hub", "mount"]);
}

#[test]
fn resolving_what_is_reached_says_the_same_as_resolving_everything() {
    // A library is loaded so that names resolve, not so that all of it
    // is worked through. What a model reaches has to be reified all the
    // same -- a type with no members shows none -- so this follows the
    // answers outward until nothing new is reached, and a package is
    // not followed: nothing is typed by one, and what is used out of it
    // records itself.
    let library = "package Lib {\n\
                   \tpart def Base { part inherited; }\n\
                   \tpart def Used :> Base { part own; }\n\
                   \tpart def NeverNamed { part unread; }\n\
                   }\n";
    let model = "package M {\n\tprivate import Lib::*;\n\tpart def Rig { part u : Used; }\n}\n";

    let mut reached = sysml_semantics::Workspace::new();
    let own = reached.add_file("m.sysml", model);
    reached.add_file("lib.sysml", library);
    reached.resolve_reached(&[own]);

    let mut everything = sysml_semantics::Workspace::new();
    everything.add_file("m.sysml", model);
    everything.add_file("lib.sysml", library);
    everything.resolve_all();

    // what the model reaches is reified the same either way
    let typed = |ws: &Workspace| {
        let model = ws.model();
        let usage = model
            .ids()
            .find(|&id| model.name(id) == Some("u"))
            .expect("`u` is declared");
        let used = model.type_of(usage).expect("`u` is typed");
        let supers: Vec<&str> = model
            .owned(used)
            .iter()
            .filter(|&&child| model.kind(child) == ElementKind::Subclassification)
            .filter_map(|&child| model.get(child, "superclassifier")?.as_id())
            .filter_map(|base| model.name(base))
            .collect();
        (
            model.name(used).unwrap_or_default().to_string(),
            supers.join(","),
        )
    };
    assert_eq!(typed(&reached), ("Used".to_string(), "Base".to_string()));
    assert_eq!(typed(&reached), typed(&everything));

    // and what it never names is left alone
    let untouched = reached.model();
    let never = untouched
        .ids()
        .find(|&id| untouched.name(id) == Some("NeverNamed"))
        .expect("declared all the same");
    assert!(
        untouched
            .owned(never)
            .iter()
            .all(|&child| untouched.kind(child) != ElementKind::FeatureTyping),
        "a definition nothing named was resolved anyway"
    );
}

#[test]
fn a_connector_relates_what_it_is_written_with() {
    // Six spellings, each of which used to relate nothing: the ends of
    // a `bind` sit around its `=`, an `interface` may name its first
    // end before `to` rather than after `connect`, a count may stand
    // between the keyword and the end, a list may be parenthesised, and
    // `then` takes an end on each side of itself.
    let ws = ws(&[(
        "c.sysml",
        "port def Pt;\n\
         part def A { port p : Pt; }\n\
         connection def Bus;\n\
         action def Step;\n\
         part def Rig {\n\
         \tpart a1 : A;\n\
         \tpart a2 : A;\n\
         \tpart a3 : A;\n\
         \tbind a1.p = a2.p;\n\
         \tinterface a1.p to a3.p;\n\
         \tconnection counted : Bus connect [1] a1 to [1] a2;\n\
         \tconnection listed : Bus connect (a1, a2, a3);\n\
         }\n\
         action def Flow {\n\
         \taction one : Step;\n\
         \tsuccession one then two;\n\
         \taction two : Step;\n\
         }\n",
    )]);
    assert!(ws.unresolved().is_empty(), "{:?}", ws.unresolved());

    let model = ws.model();
    let named = |want: &str| {
        model
            .ids()
            .find(|&id| model.name(id) == Some(want))
            .expect("declared")
    };
    // every connector says which features it holds together
    let related = |connector: sysml_model::ElementId| match model.get(connector, "relatedFeature") {
        Some(sysml_model::Value::RefList(features)) => features.len(),
        _ => 0,
    };
    let connectors: Vec<sysml_model::ElementId> = model
        .ids()
        .filter(|&id| model.kind(id).is_a(ElementKind::ConnectorAsUsage))
        .collect();
    assert_eq!(connectors.len(), 5, "one per statement");
    for connector in connectors {
        assert!(
            related(connector) >= 2,
            "{:?} relates {} feature(s)",
            model.kind(connector),
            related(connector)
        );
    }
    assert_eq!(related(named("listed")), 3, "a list of three ends");
}

#[test]
fn a_satisfaction_resolves_both_of_its_sides() {
    let ws = ws(&[(
        "s.sysml",
        "requirement def R;\n\
         part def P;\n\
         package K {\n\
         \trequirement r : R;\n\
         \tpart p : P;\n\
         \tsatisfy r by p;\n\
         }\n",
    )]);
    assert!(ws.unresolved().is_empty(), "{:?}", ws.unresolved());

    let model = ws.model();
    let assertion = model
        .ids()
        .find(|&id| model.kind(id) == ElementKind::SatisfyRequirementUsage)
        .unwrap();
    for (property, expected) in [("satisfiedRequirement", "r"), ("satisfyingFeature", "p")] {
        let Some(sysml_model::Value::Ref(target)) = model.get(assertion, property) else {
            panic!("{property} was not recorded");
        };
        assert_eq!(model.name(*target), Some(expected));
    }
}

#[test]
fn a_satisfaction_that_declares_its_requirement_still_names_one() {
    // `satisfy requirement r : R by p;` is what the corpus writes. The
    // requirement is not after the keyword there -- `r` is the name the
    // assertion is given -- so it has to be read off the typing, or the
    // model records what satisfies without recording what is satisfied.
    let ws = ws(&[(
        "s.sysml",
        "requirement def R;\n\
         part def P;\n\
         package K {\n\
         \tpart p : P;\n\
         \tsatisfy requirement r : R by p;\n\
         }\n",
    )]);
    assert!(ws.unresolved().is_empty(), "{:?}", ws.unresolved());

    let model = ws.model();
    let assertion = model
        .ids()
        .find(|&id| model.kind(id) == ElementKind::SatisfyRequirementUsage)
        .unwrap();
    for (property, expected) in [("satisfiedRequirement", "R"), ("satisfyingFeature", "p")] {
        let Some(sysml_model::Value::Ref(target)) = model.get(assertion, property) else {
            panic!("{property} was not recorded");
        };
        assert_eq!(model.name(*target), Some(expected));
    }
}

#[test]
fn a_satisfaction_that_declares_nothing_satisfies_nothing() {
    // `satisfy requirement viewpointConformance by that;`, from the
    // standard library: the name is the assertion's own, and no
    // requirement is named at all. Reading it as a reference would
    // report the library as broken.
    let ws = ws(&[(
        "s.sysml",
        "part def P {\n\
         \tsatisfy requirement conformance by that;\n\
         }\n",
    )]);
    assert!(ws.unresolved().is_empty(), "{:?}", ws.unresolved());

    let model = ws.model();
    let assertion = model
        .ids()
        .find(|&id| model.kind(id) == ElementKind::SatisfyRequirementUsage)
        .unwrap();
    assert_eq!(model.get(assertion, "satisfiedRequirement"), None);
}

#[test]
fn an_unresolvable_satisfaction_is_reported() {
    // a single name would match the assertion's own effective name, which
    // `resolve_from` allows for legal self-references; `that` outside any
    // type has nothing to stand for either
    let ws = ws(&[(
        "s.sysml",
        "part def P;\n\
         part p : P;\n\
         satisfy nowhere.deep by p;\n\
         satisfy p by that;\n",
    )]);
    let names: Vec<&str> = ws.unresolved().iter().map(|u| u.name.as_str()).collect();
    assert_eq!(names, ["nowhere::deep", "that"]);
}

#[test]
fn imported_members_walk_the_imports() {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file(
        "imports.sysml",
        "package A {\n\tpart def X;\n\tprivate part def Hidden;\n\tpackage Inner {\n\tpart def Y;\n}\n}\n\
         package B {\n\timport A::*;\n}\n\
         package C {\n\timport A::X;\n}\n\
         package D {\n\timport A::**;\n}\n\
         package E;\n",
    );
    ws.resolve_all();
    let named = |ws: &sysml_semantics::Workspace, name: &str| {
        ws.model()
            .ids()
            .find(|&id| ws.model().name(id) == Some(name))
            .unwrap()
    };
    let names_of = |ws: &mut sysml_semantics::Workspace, ns: &str| -> Vec<String> {
        let ns = named(ws, ns);
        let members = ws.imported_members(ns);
        members
            .into_iter()
            .map(|id| ws.model().name(id).unwrap_or("?").to_string())
            .collect()
    };

    // `A::*` sees the public members only; `A::X` just the one; `A::**`
    // reaches into the nested package as well
    assert_eq!(names_of(&mut ws, "B"), ["X", "Inner"]);
    // an import that never resolves brings nothing, and `import all`
    // reaches past the private member; the import inside A is not a member
    let mut ws2 = sysml_semantics::Workspace::new();
    ws2.add_file(
        "all.sysml",
        "package A {\n\tpart def X;\n\tprivate part def Hidden;\n\timport Nowhere::*;\n}\n\
         package F {\n\timport all A::*;\n}\n\
         package G {\n\timport Nowhere::*;\n}\n",
    );
    ws2.resolve_all();
    assert_eq!(names_of(&mut ws2, "F"), ["X", "Hidden"]);
    assert_eq!(names_of(&mut ws2, "G"), Vec::<String>::new());
    assert_eq!(names_of(&mut ws, "C"), ["X"]);
    assert_eq!(names_of(&mut ws, "D"), ["X", "Inner", "Y"]);
    assert_eq!(names_of(&mut ws, "E"), Vec::<String>::new());

    // and each import says what it resolved to
    let imports: Vec<_> = ws
        .model()
        .ids()
        .filter(|&id| ws.model().kind(id).is_a(sysml_model::ElementKind::Import))
        .collect();
    let a = named(&ws, "A");
    let x = named(&ws, "X");
    assert_eq!(ws.import_of(imports[0]), Some(a));
    assert_eq!(ws.import_of(imports[1]), Some(x));
}

/// `RequirementConstraintUsage : ConstraintUsage`, `FramedConcernUsage
/// : ConcernUsage` and `RequirementVerificationUsage : RequirementUsage`
/// -- each may be written as a bare reference to what it names, and
/// read as one it arrived as the plain reference a usage with no
/// keyword would. A reference is not composite, and
/// `validateRequirementConstraintMembershipIsComposite` says what a
/// requirement assumes, frames or verifies must be.
#[test]
fn what_a_requirement_assumes_or_verifies_is_not_a_bare_reference() {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file(
        "model.sysml",
        "package P {\n\
         \tconstraint def C;\n\
         \tconcern def N;\n\
         \trequirement def R2;\n\
         \trequirement def R {\n\
         \t\tassume constraint c : C;\n\
         \t\tframe concern n : N;\n\
         \t\tverify requirement v : R2;\n\t}\n}\n",
    );
    ws.resolve_all();
    let model = ws.model();
    let kind_of = |want: &str| {
        let id = model
            .ids()
            .find(|&id| model.name(id) == Some(want))
            .unwrap_or_else(|| panic!("`{want}` is declared"));
        (model.kind(id), model.get(id, "isComposite").cloned())
    };
    let composite = Some(sysml_model::Value::Bool(true));
    assert_eq!(
        kind_of("c"),
        (sysml_model::ElementKind::ConstraintUsage, composite.clone())
    );
    assert_eq!(
        kind_of("n"),
        (sysml_model::ElementKind::ConcernUsage, composite.clone())
    );
    assert_eq!(
        kind_of("v"),
        (sysml_model::ElementKind::RequirementUsage, composite)
    );
}

/// What an end declares is read from the type that owns it, with what
/// that type inherits: `assoc HappensWhile specializes HappensDuring {
/// end feature thisOccurrence redefines shorterOccurrence ... }`
/// redefines the end its supertype declares. Read past that, the name
/// is found again inside something one of the ends reaches, and the
/// redefinition then relates two features no one type features.
#[test]
fn an_end_redefines_the_one_its_own_type_inherits() {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file(
        "model.kerml",
        "package K {\n\
         \tclassifier Occ {\n\t\tfeature shorter : Occ;\n\t}\n\
         \tassoc Outer {\n\
         \t\tend feature shorter : Occ;\n\
         \t\tend feature longer : Occ;\n\t}\n\
         \tassoc Inner specializes Outer {\n\
         \t\tend feature here : Occ redefines shorter;\n\
         \t\tend feature there : Occ redefines longer;\n\t}\n}\n",
    );
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "{stats:?}");
    let model = ws.model();
    let here = model
        .ids()
        .find(|&id| ws.qualified_name_of(id) == "K::Inner::here")
        .expect("`here` is declared");
    let redefines: Vec<String> = model
        .owned(here)
        .iter()
        .copied()
        .filter(|&it| model.kind(it) == sysml_model::ElementKind::Redefinition)
        .filter_map(|it| model.get(it, "redefinedFeature").and_then(|v| v.as_id()))
        .map(|to| ws.qualified_name_of(to))
        .collect();
    assert_eq!(
        redefines,
        vec!["K::Outer::shorter".to_string()],
        "the end the supertype declares, not the member of the end's own type"
    );
}

/// `FlowEnd = ( OwnedReferenceSubsetting '.' )? FlowFeatureMember` --
/// what a flow end relates and what flows through it are two things,
/// and the notation writes them as one name. Built as one, three
/// constraints about a flow end were asked of nothing at all.
#[test]
fn a_flow_relates_flow_ends_that_own_what_flows() {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file(
        "model.sysml",
        "package P {\n\
         \tattribute def Fuel;\n\
         \tpart def Tank { out attribute fuelOut : Fuel; }\n\
         \tpart def Engine { in attribute fuelIn : Fuel; }\n\
         \tpart def Vehicle {\n\
         \t\tpart tank : Tank;\n\
         \t\tpart engine : Engine;\n\
         \t\tflow from tank.fuelOut to engine.fuelIn;\n\t}\n}\n",
    );
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "{stats:?}");
    let model = ws.model();
    let named = |want: &str| {
        model
            .ids()
            .find(|&id| model.name(id) == Some(want))
            .unwrap_or_else(|| panic!("`{want}` is declared"))
    };
    let flow = model
        .ids()
        .find(|&id| model.kind(id).is_a(sysml_model::ElementKind::FlowUsage))
        .expect("the flow is built");
    let ends: Vec<sysml_model::ElementId> = model
        .owned(flow)
        .iter()
        .copied()
        .filter(|&it| model.kind(it) == sysml_model::ElementKind::FlowEnd)
        .collect();
    assert_eq!(ends.len(), 2, "a flow end apiece");

    for (end, holder, flows, direction) in [
        (ends[0], "tank", "fuelOut", "out"),
        (ends[1], "engine", "fuelIn", "in"),
    ] {
        assert_eq!(
            model.get(end, "isEnd"),
            Some(&sysml_model::Value::Bool(true)),
            "`validateFlowEndIsEnd`"
        );
        // exactly one owned feature: the one that flows. The
        // multiplicity beside it is owned through an `OwningMembership`
        // and is not one -- `validateFlowEndNestedFeature`
        let owned: Vec<sysml_model::ElementId> = model
            .owned(end)
            .iter()
            .copied()
            .filter(|&it| model.kind(it) == sysml_model::ElementKind::Feature)
            .collect();
        assert_eq!(owned.len(), 1, "one feature flows: {owned:?}");
        let redefines = model
            .owned(owned[0])
            .iter()
            .copied()
            .find(|&it| model.kind(it) == sysml_model::ElementKind::Redefinition)
            .and_then(|it| model.get(it, "redefinedFeature").and_then(|v| v.as_id()))
            .expect("the feature that flows redefines the one it names");
        assert_eq!(redefines, named(flows));
        // and is passed the way the one it redefines is, which is what
        // `validateRedefinitionDirectionConformance` reads
        assert_eq!(
            model.get(owned[0], "direction"),
            model.get(named(flows), "direction"),
            "`{flows}` flows {direction}"
        );
        // read back, the two are the name the notation wrote
        assert_eq!(
            sysml_model::end_reaches(model, end),
            vec![named(holder), named(flows)]
        );
    }

    // an end that names one thing has nothing in front of what flows,
    // and what flows is passed no particular way
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file(
        "plain.sysml",
        "part def P {\n\tpart a;\n\tpart b;\n\tflow from a to b;\n}\n",
    );
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "{stats:?}");
    let model = ws.model();
    let ends: Vec<sysml_model::ElementId> = model
        .ids()
        .filter(|&it| model.kind(it) == sysml_model::ElementKind::FlowEnd)
        .collect();
    assert_eq!(ends.len(), 2);
    for end in ends {
        let flows = model
            .owned(end)
            .iter()
            .copied()
            .find(|&it| model.kind(it) == sysml_model::ElementKind::Feature)
            .expect("one feature flows");
        assert_eq!(model.get(flows, "direction"), None);
        assert_eq!(sysml_model::end_reaches(model, end).len(), 1);
    }
}

/// `featured by T` says what features a feature, and what it names may
/// be declared inside that very feature -- `member step merge ...
/// featured by TakePicture_snapshots { member feature
/// TakePicture_snapshots ... }`. Read from around it instead, the name
/// resolves to nothing.
#[test]
fn a_featured_by_is_read_from_inside_the_feature_it_is_written_in() {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file(
        "model.kerml",
        "package K {\n\
         \tclassifier C;\n\
         \tfeature outer : C;\n\
         \tfeature host : C featured by inner {\n\
         \t\tfeature inner : C;\n\t}\n\
         \tfeature deep : C featured by held::nested {\n\
         \t\tfeature held : C {\n\t\t\tfeature nested : C;\n\t\t}\n\t}\n\
         \tfeature plain : C featured by outer;\n}\n",
    );
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "{stats:?}");
    let named = |ws: &sysml_semantics::Workspace, want: &str| {
        ws.model()
            .ids()
            .find(|&id| ws.model().name(id) == Some(want))
            .unwrap_or_else(|| panic!("`{want}` is declared"))
    };
    let featured_by = |ws: &sysml_semantics::Workspace, of: &str| {
        ws.model()
            .owned(named(ws, of))
            .iter()
            .copied()
            .find(|&it| ws.model().kind(it) == sysml_model::ElementKind::TypeFeaturing)
            .and_then(|it| match ws.model().get(it, "featuringType") {
                Some(&sysml_model::Value::Ref(to)) => Some(to),
                _ => None,
            })
            .expect("the featuring is built")
    };
    assert_eq!(featured_by(&ws, "host"), named(&ws, "inner"));
    assert_eq!(featured_by(&ws, "deep"), named(&ws, "nested"));
    assert_eq!(featured_by(&ws, "plain"), named(&ws, "outer"));
}

/// `attribute <H> henry : PermeanceUnit, InductanceUnit = Wb/A` types
/// the attribute by both and then says what it is. Read as though the
/// comma opened a declaration of its own, only the first type stuck.
#[test]
fn a_value_may_follow_the_last_of_a_type_list() {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file(
        "model.sysml",
        "package P {\n\
         \tattribute def A;\n\
         \tattribute def B;\n\
         \tattribute h : A, B = 1;\n}\n",
    );
    ws.resolve_all();
    let named = |want: &str| {
        ws.model()
            .ids()
            .find(|&id| ws.model().name(id) == Some(want))
            .unwrap_or_else(|| panic!("`{want}` is declared"))
    };
    let (a, b, h) = (named("A"), named("B"), named("h"));
    let ups = ws.supertypes(h);
    assert!(
        ups.contains(&a) && ups.contains(&b),
        "typed by both: {ups:?}"
    );
}

/// `first merge::snap.deep then ...` names one feature and then a step
/// within it. Counted a segment at a time the end's chain gains a step
/// the notation never wrote.
#[test]
fn an_end_chain_is_cut_where_the_dots_fall() {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file(
        "model.sysml",
        "action def A {\n\
         \taction m {\n\t\taction snap {\n\t\t\taction deep;\n\t\t}\n\t}\n\
         \tfirst m::snap.deep then m;\n}\n",
    );
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "{stats:?}");
    let succession = ws
        .model()
        .ids()
        .find(|&id| {
            ws.model()
                .kind(id)
                .is_a(sysml_model::ElementKind::SuccessionAsUsage)
        })
        .expect("the succession is built");
    let chained: Vec<usize> = ws
        .model()
        .owned(succession)
        .iter()
        .filter(|&&end| ws.model().get(end, "isEnd") == Some(&sysml_model::Value::Bool(true)))
        .filter_map(|&end| match ws.model().get(end, "chainingFeature") {
            Some(sysml_model::Value::RefList(chain)) => Some(chain.len()),
            _ => None,
        })
        .collect();
    assert_eq!(chained, vec![2], "one chain of two steps: {chained:?}");
}

/// `variant action a1;` is owned through a `VariantMembership`, and only
/// a `FeatureMembership` features what it owns -- so a variation is not
/// made of its variants, which is what `validateUsageIsReferential` says
/// from the other side.
#[test]
fn a_variant_is_referred_to_rather_than_made_of() {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file(
        "model.sysml",
        "package P {\n\
         \tvariation action def A {\n\t\tvariant action a1;\n\t}\n\
         \taction def B {\n\t\taction b1;\n\t}\n}\n",
    );
    ws.resolve_all();
    let composite = |want: &str| {
        let id = ws
            .model()
            .ids()
            .find(|&id| ws.model().name(id) == Some(want))
            .unwrap_or_else(|| panic!("`{want}` is declared"));
        ws.model().get(id, "isComposite").cloned()
    };
    assert_eq!(composite("a1"), Some(sysml_model::Value::Bool(false)));
    assert_eq!(composite("b1"), Some(sysml_model::Value::Bool(true)));
}

/// `MetadataUsageDeclaration = ( Identification ( ':' | 'typed' 'by' ) )?
/// OwnedFeatureTyping` -- what follows `metadata` is the definition the
/// usage is typed by, and is a name of its own only where one is spelled
/// out in front of it. Read as a name, `metadata Classified { ... }`
/// declared a metadata usage typed by nothing at all.
#[test]
fn a_metadata_usage_is_typed_by_the_name_after_the_keyword() {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file(
        "model.sysml",
        "package P {
         	metadata def Classified;
         	part x { metadata Classified; }
         	part y { metadata m : Classified; }
         	part z { @Classified; }
}
",
    );
    ws.resolve_all();
    let named = |ws: &sysml_semantics::Workspace, want: &str| {
        ws.model()
            .ids()
            .find(|&id| ws.model().name(id) == Some(want))
            .unwrap_or_else(|| panic!("`{want}` is declared"))
    };
    let metadata_in = |ws: &sysml_semantics::Workspace, holder: &str| {
        *ws.model()
            .owned(named(ws, holder))
            .iter()
            .find(|&&child| {
                ws.model()
                    .kind(child)
                    .is_a(sysml_model::ElementKind::MetadataUsage)
            })
            .expect("the part carries metadata")
    };
    let definition = named(&ws, "Classified");
    for holder in ["x", "y", "z"] {
        let usage = metadata_in(&ws, holder);
        assert!(
            ws.supertypes(usage).contains(&definition),
            "the metadata of `{holder}` is typed by `Classified`"
        );
    }
    // only the one written with a `:` in front of it is named
    assert_eq!(ws.model().name(metadata_in(&ws, "x")), None);
    assert_eq!(ws.model().name(metadata_in(&ws, "y")), Some("m"));
    assert_eq!(ws.model().name(metadata_in(&ws, "z")), None);
}

#[test]
fn implied_specializations_are_materialized_once() {
    let mut ws = sysml_semantics::Workspace::new();
    // a miniature semantic library, enough for the bases to resolve
    ws.add_file(
        "mini-library.kerml",
        "package Base {\n\tabstract classifier Anything;\n\tabstract feature things;\n\
         \tabstract datatype DataValue;\n}\n",
    );
    ws.add_file(
        "mini-parts.sysml",
        "package Parts {\n\tabstract part def Part;\n}\n",
    );
    ws.add_file(
        "model.sysml",
        "part def Vehicle;\n\
         part def Car :> Parts::Part {\n\tattribute mass;\n}\n",
    );
    ws.resolve_all();
    let written = ws.materialize_implied();
    assert!(written > 0);
    let model = ws.model();
    let named = |name: &str| {
        model
            .ids()
            .find(|&id| model.name(id) == Some(name))
            .unwrap()
    };

    // Vehicle gained an implied subclassification of Parts::Part...
    let vehicle = named("Vehicle");
    let implied: Vec<_> = model
        .owned(vehicle)
        .iter()
        .copied()
        .filter(|&child| {
            model.kind(child) == sysml_model::ElementKind::Subclassification
                && model.get(child, "isImplied") == Some(&sysml_model::Value::Bool(true))
        })
        .collect();
    assert_eq!(implied.len(), 1);
    assert_eq!(
        model.get(implied[0], "superclassifier"),
        Some(&sysml_model::Value::Ref(named("Part")))
    );
    assert_eq!(
        model.get(vehicle, "isImpliedIncluded"),
        Some(&sysml_model::Value::Bool(true))
    );

    // ...Car reaches it explicitly, so nothing was implied for it
    let car = named("Car");
    assert!(!model.owned(car).iter().any(|&child| {
        model.get(child, "isImplied") == Some(&sysml_model::Value::Bool(true))
            && model.kind(child) == sysml_model::ElementKind::Subclassification
    }));

    // a feature subsets `Base::things` the implied way
    let mass = named("mass");
    let subsets: Vec<_> = model
        .owned(mass)
        .iter()
        .copied()
        .filter(|&child| model.kind(child) == sysml_model::ElementKind::Subsetting)
        .collect();
    assert!(subsets.iter().any(|&child| {
        model.get(child, "subsettedFeature") == Some(&sysml_model::Value::Ref(named("things")))
            && model.get(child, "isImplied") == Some(&sysml_model::Value::Bool(true))
    }));
    // and is implicitly typed by the base classifier, not subsetting it
    assert!(model.owned(mass).iter().any(|&child| {
        model.kind(child) == sysml_model::ElementKind::FeatureTyping
            && model.get(child, "type") == Some(&sysml_model::Value::Ref(named("DataValue")))
            && model.get(child, "isImplied") == Some(&sysml_model::Value::Bool(true))
    }));

    // running the pass again writes nothing: everything is reachable now
    assert_eq!(ws.materialize_implied(), 0);
}

#[test]
fn implied_bases_reach_through_chains_and_keywords() {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file(
        "mini-parts.sysml",
        "package Parts {\n\tabstract part def Part;\n}\n",
    );
    ws.add_file(
        "model.sysml",
        "package P {\n\
         \tmetadata def SemanticMetadata { attribute baseType; }\n\
         \tpart causes;\n\
         \tmetadata def cause :> SemanticMetadata { :>> baseType = causes meta X; }\n\
         \t#cause part def Storm;\n\
         \tmetadata def plain;\n\
         \t#plain part def Cloudy;\n\
         \t#nowhere part def Foggy;\n\
         \tpart def Middle :> Parts::Part;\n\
         \tpart def Leaf :> Middle;\n}\n",
    );
    ws.resolve_all();
    ws.materialize_implied();
    let model = ws.model();
    let named = |name: &str| {
        model
            .ids()
            .find(|&id| model.name(id) == Some(name))
            .unwrap()
    };
    let implied_of = |elem| {
        model
            .owned(elem)
            .iter()
            .copied()
            .filter(|&child| model.get(child, "isImplied") == Some(&sysml_model::Value::Bool(true)))
            .count()
    };

    // `Leaf` reaches `Parts::Part` through `Middle`, so nothing is implied
    assert_eq!(implied_of(named("Leaf")), 0);
    // `#cause` implies the keyword's base alongside the library base
    let storm = named("Storm");
    assert!(model.owned(storm).iter().any(|&child| {
        model.kind(child) == sysml_model::ElementKind::Subclassification
            && model.get(child, "superclassifier")
                == Some(&sysml_model::Value::Ref(named("causes")))
    }));
    // a keyword that names no SemanticMetadata implies nothing extra, and
    // an unresolvable one implies nothing at all
    assert_eq!(implied_of(named("Cloudy")), 1);
    assert_eq!(implied_of(named("Foggy")), 1);
}

#[test]
fn a_dangling_typing_does_not_stop_the_implied_walk() {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file(
        "mini-actions.sysml",
        "package Actions {\n\tabstract action def AcceptAction;\n}\n",
    );
    ws.add_file(
        "mini-base.kerml",
        "package Base {\n\tabstract feature things;\n}\n",
    );
    // the accept payload's type never resolves, so its reified typing has
    // no target; the pass walks past it and still implies the base
    ws.add_file(
        "machine.sysml",
        "state def S {\n\tstate a;\n\tstate b;\n\
         \ttransition t1 first a accept p : Nowhere then b;\n}\n",
    );
    ws.resolve_all();
    ws.materialize_implied();
    let model = ws.model();
    let accept = model
        .ids()
        .find(|&id| model.kind(id) == sysml_model::ElementKind::AcceptActionUsage)
        .expect("the trigger was reified");
    let base = model
        .ids()
        .find(|&id| model.name(id) == Some("AcceptAction"))
        .unwrap();
    assert!(model.owned(accept).iter().any(|&child| {
        model.kind(child) == sysml_model::ElementKind::FeatureTyping
            && model.get(child, "type") == Some(&sysml_model::Value::Ref(base))
            && model.get(child, "isImplied") == Some(&sysml_model::Value::Bool(true))
    }));
    // the payload feature itself: its declared typing never resolved, so
    // the walk passed the dangling relationship and implied the subset
    let payload = model
        .ids()
        .find(|&id| model.name(id) == Some("p"))
        .expect("the payload was built");
    let things = model
        .ids()
        .find(|&id| model.name(id) == Some("things"))
        .unwrap();
    assert!(model.owned(payload).iter().any(|&child| {
        model.kind(child) == sysml_model::ElementKind::Subsetting
            && model.get(child, "subsettedFeature") == Some(&sysml_model::Value::Ref(things))
    }));
}

#[test]
fn a_bare_annotation_resolves_to_nothing_quietly() {
    // error recovery can leave an annotation with no name at all
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file("bare.sysml", "part def A {\n\t@ ;\n}\n");
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0);
}

#[test]
fn a_kerml_relation_after_a_name_still_leaves_a_declaration() {
    // `featured by` and its siblings follow a declared name the way
    // `chains` does. Reading one of them as the start of a reference
    // costs the declaration its name, and every qualified path through
    // it stops resolving -- which is how ten references in the official
    // `TimeVaryingFeatures.kerml` went unresolved.
    for relation in [
        "featured by f",
        "chains f",
        "unions f",
        "intersects f",
        "differences f",
        "disjoint from f",
        "inverse of f",
        "conjugates f",
    ] {
        let text = format!(
            "package T {{\n\tclass C {{\n\t\tmember feature x {relation};\n\
             \t\tfeature q :>> C::x;\n\t}}\n\tfeature f;\n}}\n"
        );
        let mut ws = Workspace::default();
        let file = ws.add_file("t.kerml", &text);
        let stats = ws.resolve_all();
        assert!(ws.file_parse(file).ok(), "{relation}: parses");
        assert_eq!(
            stats.unresolved,
            0,
            "{relation}: `C::x` found nothing -- {:?}",
            ws.unresolved()
        );
    }
}

/// The shape a language server works in: a project is resolved without
/// the buffers that are open, and the open buffers are added to it
/// afterwards. A name that was not there for the first resolution must
/// not stay missing for the rest of the session.
#[test]
fn a_name_missing_when_it_was_first_looked_up_is_found_once_its_file_arrives() {
    let mut ws = Workspace::default();
    let a = ws.add_file(
        "a.sysml",
        "package A { part def Car :> B::Vehicle; part car : Car { attribute :>> mass; } }",
    );
    ws.resolve_files(&[a]);
    let b = ws.add_file(
        "b.sysml",
        "package B { part def Vehicle { attribute mass; } }",
    );
    let c = ws.add_file(
        "c.sysml",
        "package C { part c : A::Car { attribute :>> mass; } }",
    );
    ws.resolve_files(&[b, c]);
    assert!(
        ws.unresolved().iter().all(|u| u.file != c),
        "{:?}",
        ws.unresolved()
    );

    // the same for an import: its failure was cached too
    let mut ws = Workspace::default();
    let a = ws.add_file("a.sysml", "package A { public import B::*; }");
    ws.resolve_files(&[a]);
    let b = ws.add_file("b.sysml", "package B { part def Vehicle; }");
    let c = ws.add_file("c.sysml", "package C { part v : A::Vehicle; }");
    ws.resolve_files(&[b, c]);
    assert!(ws.unresolved().is_empty(), "{:?}", ws.unresolved());
}

/// A root package of one's own named after one of the standard
/// library's is read on the side of the boundary it was written on,
/// whichever order the files arrived in, and the collision is said out
/// loud.
#[test]
fn a_root_package_named_after_a_library_one_is_reported_and_read_from_its_own_side() {
    let library = "standard library package Requirements {\n    part def RequirementCheck;\n}\nstandard library package Constraints {\n    part def Check :> Requirements::RequirementCheck;\n}\n";
    let mine = "package Requirements {\n    part def Safe;\n}\npackage M {\n    import Requirements::*;\n    part s : Safe;\n}\n";
    for order in [
        [("lib.sysml", library), ("m.sysml", mine)],
        [("m.sysml", mine), ("lib.sysml", library)],
    ] {
        let ws = ws(&order);
        assert!(ws.unresolved().is_empty(), "{:?}", ws.unresolved());
        let collisions = ws.findings(&[]).collisions;
        assert_eq!(collisions.len(), 1, "{collisions:?}");
        assert_eq!(ws.file_name(collisions[0].file), "m.sysml");
        assert_eq!(
            collisions[0].what,
            "`Requirements` is also a root package of the standard library"
        );
        // and the collision is reported in the file it is written in
        assert!(ws.findings(&[0]).collisions.len() + ws.findings(&[1]).collisions.len() == 1);
    }
}

/// A `private package` is not a way through: what is nested in it stays
/// nested, however public each of those members is.
#[test]
fn a_recursive_import_stops_at_a_private_namespace() {
    let text = "package P {\n    private package Hidden { part def X; }\n    part def Open;\n}\npackage M {\n    import P::**;\n    part o : Open;\n    part x : X;\n}\n";
    let mut pruned = ws(&[("m.sysml", text)]);
    let names: Vec<&str> = pruned
        .unresolved()
        .iter()
        .map(|u| u.name.as_str())
        .collect();
    assert_eq!(names, ["X"]);
    let m = pruned
        .model()
        .ids()
        .find(|&id| pruned.model().name(id) == Some("M"))
        .expect("M");
    let members = pruned.imported_members(m);
    let imported: Vec<String> = members
        .iter()
        .filter_map(|&id| pruned.model().name(id).map(String::from))
        .collect();
    assert!(imported.contains(&"Open".to_string()), "{imported:?}");
    assert!(!imported.contains(&"X".to_string()), "{imported:?}");
    let offered: Vec<String> = pruned
        .visible_names(0, offset_of(text, "part x"))
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert!(!offered.contains(&"X".to_string()), "{offered:?}");

    // `import all` is what asks for the private ones anyway
    let all = text.replace("import P::**", "import all P::**");
    let asking_for_all = ws(&[("m.sysml", &all)]);
    assert!(
        asking_for_all.unresolved().is_empty(),
        "{:?}",
        asking_for_all.unresolved()
    );
}

/// `that` is not looked up, so nothing was walked to reach it. What the
/// last qualified name walked belongs to that name: recorded again here
/// it would offer a rename of `Lib` an edit over the word `that`.
#[test]
fn a_name_the_resolver_does_not_look_up_records_no_earlier_segments() {
    let text = "package Lib { part def Q; }\npackage M {\n    part q : Lib::Q { satisfy nowhere.deep by that; }\n}\n";
    let ws = ws(&[("m.sysml", text)]);
    let lib = ws
        .model()
        .ids()
        .find(|&id| ws.model().name(id) == Some("Lib"))
        .expect("Lib");
    let written: Vec<&str> = ws
        .references_to(lib)
        .map(|r| &text[usize::from(r.range.start())..usize::from(r.range.end())])
        .collect();
    assert_eq!(written, ["Lib"]);
}

/// The language server resolves a project once and then answers from
/// copies of it, one per set of open buffers.
#[test]
fn a_copy_of_a_workspace_answers_what_the_original_answers() {
    let original = ws(&[("m.sysml", "package P { part def A; part a : A; }")]);
    let mut copy = original.clone();
    let usage = copy
        .model()
        .ids()
        .find(|&id| copy.model().name(id) == Some("a"))
        .expect("a");
    let found = copy.resolve_from(usage, &["A".to_string()]);
    assert_eq!(copy.model().name(found.expect("A")), Some("A"));
    assert_eq!(copy.references().len(), original.references().len());
    assert!(copy.findings(&[]).names.is_empty());
}

/// Every other query answers `None` for a cursor the file does not
/// have. An editor that trails a stale position behind an edit asks
/// this one as readily as the rest.
#[test]
fn a_cursor_past_the_end_of_the_file_is_not_inside_a_call() {
    let mut ws = ws(&[(
        "m.sysml",
        "package P { calc def Sum { in a; } attribute s = Sum(1); }",
    )]);
    let past = TextSize::from(10_000);
    assert!(ws.callable_at(0, past).is_none());
    assert!(ws.reference_at(0, past).is_none());
    assert!(ws.definition_at(0, past).is_none());
}

/// Looking a name up used to scan every member of the namespace, so a
/// package of n parts cost n scans of n members to resolve. Ten
/// thousand of them is a real model, and it took minutes.
#[test]
fn a_package_of_thousands_of_members_resolves_without_scanning_them_all() {
    let n = 8000;
    let mut text = String::from("package P { part def A0;\n");
    for i in 1..=n {
        text.push_str(&format!("part def A{i} :> A{};\n", i - 1));
    }
    text.push_str("}\n");
    let started = std::time::Instant::now();
    let mut ws = Workspace::default();
    ws.add_file("m.sysml", &text);
    let stats = ws.resolve_all();
    assert_eq!((stats.resolved, stats.unresolved), (n, 0));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "resolving {n} siblings took {:?}",
        started.elapsed()
    );
}

/// What is offered as you type has to be what a lookup will find. A
/// private import does not re-export, and a chain of re-exports is
/// followed to the end however long it is.
#[test]
fn completion_offers_the_names_a_lookup_would_find() {
    let text = "package Lib { part def Widget; }\npackage B { import Lib::*; }\npackage M { import B::*; part w : Widget; }\n";
    let mut behind_a_private_import = ws(&[("m.sysml", text)]);
    // lookup refuses it, so completion must not offer it
    assert_eq!(behind_a_private_import.unresolved().len(), 1);
    let offered: Vec<String> = behind_a_private_import
        .visible_names(0, offset_of(text, "part w"))
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert!(!offered.contains(&"Widget".to_string()), "{offered:?}");

    // twenty packages re-exporting one another, which lookup follows
    let mut text = String::from("package L0 { part def Deep; }\n");
    for i in 1..=20 {
        text.push_str(&format!(
            "package L{i} {{ public import L{}::*; }}\n",
            i - 1
        ));
    }
    text.push_str("package M { import L20::*; part d : Deep; }\n");
    let mut long_chain = ws(&[("m.sysml", &text)]);
    assert!(long_chain.unresolved().is_empty());
    let offered: Vec<String> = long_chain
        .visible_names(0, offset_of(&text, "part d :"))
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert!(offered.contains(&"Deep".to_string()), "{offered:?}");
}

/// A package's members are the ones written in it; a type's are its own
/// and every public one it inherits, so importing a type imports what
/// its supertypes give it.
#[test]
fn importing_a_type_imports_what_it_inherits() {
    let text = "package P {\n    part def Base { attribute a; }\n    part def Sub :> Base { attribute b; }\n}\npackage M {\n    import P::Sub::*;\n    part x { attribute ra = a; attribute rb = b; }\n}\n";
    let ws = ws(&[("m.sysml", text)]);
    assert!(ws.unresolved().is_empty(), "{:?}", ws.unresolved());
}

/// A quoted name means what it spells once its escapes are resolved,
/// and the model stores it that way. A reference that only stripped the
/// quotes was comparing two different strings.
#[test]
fn a_quoted_name_with_an_escape_in_it_resolves() {
    let text = "package P {\n    part def 'a\\'b';\n    part x : 'a\\'b';\n}\n";
    let ws = ws(&[("m.sysml", text)]);
    assert!(ws.unresolved().is_empty(), "{:?}", ws.unresolved());
    assert_eq!(ws.references().len(), 1);
    assert_eq!(
        ws.model().name(ws.references()[0].target),
        Some("a'b"),
        "the model stores the name unescaped"
    );
}

#[test]
fn an_alias_says_on_the_model_what_it_names() {
    let ws = ws(&[(
        "m.sysml",
        "package P {\n    part def Engine;\n    alias Motor for Engine;\n    alias Broken for ;\n}\n",
    )]);
    let model = ws.model();
    let aliases: Vec<_> = model.ids().filter(|&id| ws.is_alias(id)).collect();
    assert_eq!(aliases.len(), 2);
    let named: Vec<Option<&str>> = aliases
        .iter()
        .map(|&id| ws.alias_target(id).and_then(|to| model.name(to)))
        .collect();
    // the one that names something says so; the one that names nothing
    // is left saying nothing rather than pointing at the wrong thing
    assert_eq!(named, vec![Some("Engine"), None]);
}

#[test]
fn what_a_view_exposes_ignores_visibility() {
    let ws = ws(&[(
        "m.sysml",
        "package ViewTest {\n    package P {\n        private part p2;\n    }\n    view def V;\n    view v : V {\n        expose P::*;\n        alias vp2 for p2;\n    }\n}\n",
    )]);
    // `expose` is an import with isImportAll set, so what the package
    // keeps to itself is still what the view is pointed at
    assert_eq!(ws.unresolved().len(), 0, "{:?}", ws.unresolved());
}

/// An import whose own path walks back into a type asks that type what
/// it specializes, and the answer it gets is refused by the guard that
/// keeps the import from resolving itself. Settling for that answer
/// leaves the model saying two different things: `A :> Other` reified,
/// and `Other`'s members unreachable through `A`.
#[test]
fn what_a_blocked_import_hid_is_not_the_supertype_list_kept() {
    let ws = ws(&[(
        "m.sysml",
        "package P {
    public import Q::Sub::*;
    part def A :> Base, Other;
    part a : A {
        attribute :>> fromOther;
    }
}
package Q {
    public import P::A::Nothing::*;
    public import R::*;
}
package R {
    package Sub {
        part def Base;
        part def Other { attribute fromOther; }
    }
}
",
    )]);
    assert_eq!(ws.unresolved().len(), 0, "{:?}", ws.unresolved());
}

/// Everything a pass reifies, in one model: typings, a specialization,
/// a redefinition, the reference behind `perform`, an annotation, the
/// ends of a connector (twice over the same feature), a trigger's
/// payload type and what an alias names.
const REIFIES: &str = "package P {
    part def Base;
    part def A :> Base {
        attribute mass;
    }
    alias Alias for A;
    part a : A {
        attribute :>> mass;
    }
    part b : A;
    comment about a /* the one at the end of the connection */
    connect a to b;
    connect a to a;
    action def Sense;
    action sense : Sense;
    part c {
        perform sense;
        state s {
            entry; then done;
            state done;
            accept e : Base then done;
        }
    }
}
";

#[test]
fn asking_twice_leaves_the_model_saying_it_once() {
    let mut ws = Workspace::default();
    ws.add_file("m.sysml", REIFIES);
    let first = ws.resolve_all();
    assert_eq!(first.unresolved, 0, "{:?}", ws.unresolved());
    let (elements, references) = (ws.model().len(), ws.references().len());
    let again = ws.resolve_all();
    assert_eq!(
        ws.model().len(),
        elements,
        "a second pass reified what the first already had"
    );
    assert_eq!(
        (again.resolved, again.unresolved),
        (first.resolved, first.unresolved)
    );
    // and what was found about the file is replaced, not added to
    assert_eq!(ws.references().len(), references);
}

/// `part p4 :> p4;` is a feature saying it is the one its type already
/// declares, and the language reads it that way even where there is no
/// such feature. Nothing else can mean that: a type is not its own
/// type, and a definition does not specialize itself.
#[test]
fn only_a_subsetting_may_name_the_declaration_that_writes_it() {
    let ws = ws(&[(
        "m.sysml",
        "package P {\n    part v : v;\n    part def C :> C;\n    part p4 :> p4;\n    part r ::> r;\n}\n",
    )]);
    let names: Vec<&str> = ws.unresolved().iter().map(|u| u.name.as_str()).collect();
    assert_eq!(names, ["v", "C"]);
    let model = ws.model();
    let loops = model
        .ids()
        .filter(|&id| model.kind(id).is_a(sysml_model::ElementKind::FeatureTyping))
        .count();
    assert_eq!(loops, 0, "a feature was left standing as its own type");
}

/// A model can specialize deeper than a stack goes. Walking what it
/// inherits took the process down with it; now the walk stops and the
/// name is reported as one that resolves to nothing.
#[test]
fn a_specialization_chain_deeper_than_the_stack_is_a_finding() {
    let deep = 8_000;
    let mut text = String::from("package P {\n");
    for step in 0..deep {
        text.push_str(&format!("    part def A{step} :> A{};\n", step + 1));
    }
    text.push_str("    part a : A0 { attribute :>> nowhere; }\n}\n");
    let ws = ws(&[("m.sysml", &text)]);
    let names: Vec<&str> = ws.unresolved().iter().map(|u| u.name.as_str()).collect();
    // the end of the chain names nothing, and neither does a member
    // looked for past the depth the walk stops at
    assert_eq!(names, [format!("A{deep}").as_str(), "nowhere"]);
}

#[test]
fn one_file_read_twice_is_still_one_file() {
    let mut ws = Workspace::default();
    let text = "package P {\n    part def Thing;\n}\n";
    let first = ws.add_file("m.sysml", text);
    assert_eq!(ws.add_file("m.sysml", text), first);
    assert_eq!(ws.file_count(), 1);
    // the same name over different text is a different file, because
    // ids already handed out for the first one still name it
    let other = ws.add_file("m.sysml", "package Q {\n    part def Thing;\n}\n");
    assert_ne!(other, first);
    assert_eq!(ws.file_count(), 2);
    ws.resolve_all();
    let things: Vec<_> = ws
        .named_elements()
        .filter(|(_, name)| *name == "Thing")
        .collect();
    assert_eq!(things.len(), 2, "one per file, not one per reading");
}

/// The same artifact one step further in: a user-defined keyword says
/// what it stands for through a `SemanticMetadata` base, and that name
/// is looked up in the same refused window. A keyword that came back
/// with nothing leaves everything it marks without the members it
/// brings.
#[test]
fn what_a_blocked_import_hid_is_not_the_keyword_base_kept() {
    let ws = ws(&[(
        "m.sysml",
        "package P {
    public import Q::Sub::*;
    metadata def Marker :> SemanticMetadata {
        :>> baseType = Other meta SysML::Usage;
    }
    #Marker part def A;
    part a : A {
        attribute :>> fromOther;
    }
}
package Q {
    public import P::A::Nothing::*;
    public import R::*;
}
package R {
    package Sub {
        metadata def SemanticMetadata { attribute baseType; }
        part def Other { attribute fromOther; }
    }
}
",
    )]);
    assert_eq!(ws.unresolved().len(), 0, "{:?}", ws.unresolved());
}

/// `then b;` names where the flow goes and not where it comes from, and
/// a succession relates both. The specification says so itself --
/// `validateConnectorRelatedFeatures`, "a concrete Connector must have
/// at least two relatedFeatures" -- and without the source the model
/// says a step follows nothing.
#[test]
fn a_succession_records_what_it_follows() {
    let related = |source: &str| {
        let mut ws = sysml_semantics::Workspace::new();
        ws.add_file("test.sysml", source);
        ws.resolve_all();
        let model = ws.model();
        let succession = model
            .ids()
            .find(|&id| model.kind(id).name() == "SuccessionAsUsage")
            .expect("the succession is built");
        match model.get(succession, "relatedFeature") {
            Some(sysml_model::Value::RefList(ends)) => ends
                .iter()
                .map(|&end| model.name(end).unwrap_or("?").to_string())
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        }
    };

    // the step written before it
    assert_eq!(
        related("action def A {\n\taction a;\n\tthen b;\n\taction b;\n}\n"),
        ["a", "b"]
    );
    // an occurrence too, which is how a sequence model is written
    assert_eq!(
        related(
            "occurrence def O;\npart def P {\n\tevent occurrence e : O;\n\tthen f;\n\
             \tevent occurrence f : O;\n}\n"
        ),
        ["e", "f"]
    );
    // where the one before it is another succession, the answer is
    // where that one went: `then b; then c;` runs b to c
    let all = |source: &str| {
        let mut ws = sysml_semantics::Workspace::new();
        ws.add_file("test.sysml", source);
        ws.resolve_all();
        let model = ws.model();
        model
            .ids()
            .filter(|&id| model.kind(id).name() == "SuccessionAsUsage")
            .map(|id| match model.get(id, "relatedFeature") {
                Some(sysml_model::Value::RefList(ends)) => ends
                    .iter()
                    .map(|&end| model.name(end).unwrap_or("?").to_string())
                    .collect::<Vec<_>>()
                    .join("-"),
                _ => String::new(),
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        all("action def A {\n\taction a;\n\tthen b;\n\tthen c;\n\taction b;\n\taction c;\n}\n"),
        ["a-b", "b-c"]
    );

    // a connector before it that relates nothing is stepped over: it is
    // not where anything comes from
    assert_eq!(
        related("action def A {\n\taction a;\n\tbind p = q;\n\tthen b;\n\taction b;\n}\n"),
        ["a", "b"]
    );
    // and `entry;` on its own declares an action for one to start from
    let ends = related("state def S {\n\tentry;\n\tthen Wait;\n\tstate Wait;\n}\n");
    assert_eq!(ends.len(), 2, "{ends:?}");
    assert_eq!(ends[1], "Wait");

    // `then merge m;` writes the node and the succession into it as one
    // statement, so the succession's other end is the sibling that
    // statement also became -- not an operand anywhere in the text
    assert_eq!(
        related("action def A {\n\taction a;\n\tthen merge m;\n}\n"),
        ["a", "m"]
    );
    // and where such a statement writes a `from` and a `to` of its own,
    // they say where the flow runs rather than where the succession
    // does: reading them as the succession's leaves it relating one
    // thing, which `validateConnectorRelatedFeatures` rejects
    assert_eq!(all(MESSAGE), ["e-m"]);
}

/// `then message m of T from a to b;` writes a flow and the succession
/// into it, and each has ends of its own: the flow runs from `a` to `b`,
/// the step into the flow from what stands above it.
const MESSAGE: &str = "occurrence def T;\noccurrence def O {\n\tevent occurrence a;\n\
                       \tevent occurrence b;\n\tevent occurrence e;\n\
                       \tthen message m of T from a to b;\n}\n";

/// Which keywords name an end is a question about the element asking and
/// not about the syntax alone: one statement is two elements here, and
/// read off the syntax the two take each other's ends.
#[test]
fn a_message_says_where_it_runs_as_well_as_what_it_follows() {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file("test.sysml", MESSAGE);
    ws.resolve_all();
    assert_eq!(ws.unresolved().len(), 0, "{:?}", ws.unresolved());
    let model = ws.model();
    let ends = |metaclass: &str| {
        let id = model
            .ids()
            .find(|&id| model.kind(id).name() == metaclass)
            .unwrap_or_else(|| panic!("the {metaclass} is built"));
        match model.get(id, "relatedFeature") {
            Some(sysml_model::Value::RefList(related)) => related
                .iter()
                .map(|&end| model.name(end).unwrap_or("?").to_string())
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        }
    };
    assert_eq!(ends("FlowUsage"), ["a", "b"]);
    assert_eq!(ends("SuccessionAsUsage"), ["e", "m"]);
}

/// `send new S() via p to b;` says which port a message leaves by and
/// who receives it, `accept s via p;` where one arrives, and `assign v
/// := 1;` which feature it sets. None of those names was being looked
/// up at all, so each stood for nothing -- and a name that stands for
/// nothing was not reported either, which is worse than reporting it.
///
/// The standard keeps the first two as arguments of the action, in the
/// input parameters it declares in order and reads back by position
/// (`senderArgument = argument(2)`, `receiverArgument = argument(3)`),
/// and the last as the one membership an assignment does not own.
#[test]
fn what_a_send_an_accept_and_an_assign_name_is_looked_up() {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file(
        "test.sysml",
        "attribute def S;\npart def P { port p; }\n\
         action def A {\n\tattribute v;\n\tpart hub : P;\n\taction x;\n\
         \tthen assign v := 1;\n\tthen send new S() via hub.p to x;\n\
         \tthen accept s via hub.p;\n}\n",
    );
    ws.resolve_all();
    assert_eq!(ws.unresolved().len(), 0, "{:?}", ws.unresolved());
    let model = ws.model();
    let only = |metaclass: &str| {
        model
            .ids()
            .find(|&id| model.kind(id).name() == metaclass)
            .unwrap_or_else(|| panic!("the {metaclass} is built"))
    };
    let parameters = |action: sysml_model::ElementId| {
        model
            .owned(action)
            .iter()
            .copied()
            .filter(|&child| model.kind(child) == ElementKind::ReferenceUsage)
            .collect::<Vec<_>>()
    };
    // the argument an expression stands for, by the name it resolved to
    let argument = |parameter: sysml_model::ElementId| {
        let value = model
            .owned(parameter)
            .iter()
            .copied()
            .find(|&child| model.kind(child) == ElementKind::FeatureValue)?;
        let expression = model.get(value, "value")?.as_id()?;
        let referent = model.get(expression, "referent")?.as_id()?;
        model.name(referent).map(str::to_string)
    };

    // payload, sender and receiver -- three of them whether or not the
    // source wrote a clause for each, which is what
    // `validateSendActionParameters` says
    let send = parameters(only("SendActionUsage"));
    assert_eq!(send.len(), 3);
    assert_eq!(argument(send[0]), None, "the payload writes no name");
    assert_eq!(argument(send[1]).as_deref(), Some("p"));
    assert_eq!(argument(send[2]).as_deref(), Some("x"));

    // and two for an accept: payload and receiver
    let accept = parameters(only("AcceptActionUsage"));
    assert_eq!(accept.len(), 2);
    assert_eq!(argument(accept[1]).as_deref(), Some("p"));

    // the assignment refers to what it sets without owning it
    let assign = only("AssignmentActionUsage");
    let referred = model
        .owned(assign)
        .iter()
        .copied()
        .find(|&child| model.kind(child) == ElementKind::Membership)
        .and_then(|membership| model.get(membership, "memberElement")?.as_id())
        .and_then(|target| model.name(target));
    assert_eq!(referred, Some("v"));

    // and a name that stands for nothing is reported, which is what
    // looking it up was for
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file(
        "miss.sysml",
        "action def A {\n\tthen assign nowhere := 1;\n}\n",
    );
    ws.resolve_all();
    assert_eq!(
        ws.unresolved()
            .iter()
            .map(|it| it.name.clone())
            .collect::<Vec<_>>(),
        ["nowhere"]
    );
}

/// `abstract function LiteralEvaluation specializes Evaluation { return
/// : ScalarValue[1]; }` -- the library writes no `redefines`, and the
/// standard says it does not have to: a result parameter of a function
/// that specializes another redefines that one's.
///
/// Without the redefinition, the specializing function has two result
/// parameters -- its own and the one it inherits -- and "a function has
/// exactly one" is true of none of the five hundred in the corpus that
/// declare one. The walk carries on past a general type that declares
/// no result of its own, since one may be inherited in turn, and past a
/// general type it has already looked through.
#[test]
fn a_result_parameter_redefines_the_one_the_general_function_declares() {
    let mut ws = Workspace::new();
    ws.add_file(
        "f.kerml",
        "classifier T;\n\
         function General { return : T; }\n\
         function Middle specializes General;\n\
         function Special specializes Middle { return : T; }\n\
         function Base;\n\
         function Left specializes Base;\n\
         function Right specializes Base;\n\
         function Diamond specializes Left, Right { return : T; }\n",
    );
    ws.resolve_all();
    assert!(ws.materialize_implied() > 0);
    let model = ws.model();
    let named = |name: &str| {
        model
            .ids()
            .find(|&id| model.name(id) == Some(name))
            .unwrap_or_else(|| panic!("`{name}` is declared"))
    };
    let result_of = |of: sysml_model::ElementId| {
        model
            .owned(of)
            .iter()
            .copied()
            .find(|&it| model.member_role(it) == Some(sysml_model::Role::Return))
            .expect("a return was declared")
    };
    let special = result_of(named("Special"));
    let general = result_of(named("General"));
    let implied: Vec<_> = model
        .owned(special)
        .iter()
        .copied()
        .filter(|&it| model.kind(it) == ElementKind::Redefinition)
        .collect();
    assert_eq!(implied.len(), 1, "one implied redefinition, past Middle");
    let redefinition = implied[0];
    assert_eq!(
        model.get(redefinition, "redefinedFeature"),
        Some(&sysml_model::Value::Ref(general))
    );
    assert_eq!(
        model.get(redefinition, "isImplied"),
        Some(&sysml_model::Value::Bool(true))
    );
    // `validateElementIsImpliedIncluded` -- what owns an implied
    // relationship says that it does
    assert_eq!(
        model.get(special, "isImpliedIncluded"),
        Some(&sysml_model::Value::Bool(true))
    );
    // General declares the result the others inherit, so it redefines
    // nothing and says nothing about implied relationships
    assert!(model
        .owned(general)
        .iter()
        .all(|&it| model.kind(it) != ElementKind::Redefinition));
    // Where two general types meet again above, the walk passes the
    // one they share the second time rather than looking through it
    // twice -- and nothing up there declares a result, so `Diamond`
    // redefines nothing
    let diamond = result_of(named("Diamond"));
    assert!(model
        .owned(diamond)
        .iter()
        .all(|&it| model.kind(it) != ElementKind::Redefinition));
}

/// `port p : ~P` types the port by the conjugate of `P`, which the port
/// definition owns under that name -- so naming it is a step further
/// down the same path, and the walk that finds `P` finds it.
///
/// And `~P` has what `P` has: conjugating a type reverses the direction
/// of its features, not which features it has. Without that, a member
/// reached through a conjugated port names nothing.
#[test]
fn a_conjugated_port_is_typed_by_the_conjugate_and_has_what_it_conjugates() {
    let mut ws = Workspace::new();
    let file = ws.add_file(
        "p.sysml",
        "part def V {\n\
         \tport def P { attribute size; }\n\
         \tport plain : P;\n\
         \tport other : ~P;\n\
         \tpart w { attribute a = other.size; }\n\
         }\n",
    );
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
    let root = ws.file_roots(file)[0];
    let named = |ws: &Workspace, want: &str| {
        ws.model()
            .descendants(root)
            .into_iter()
            .find(|&id| ws.model().name(id) == Some(want))
            .unwrap_or_else(|| panic!("`{want}` is declared"))
    };
    let (plain, other) = (named(&ws, "plain"), named(&ws, "other"));
    let type_of = |ws: &Workspace, port| {
        ws.model()
            .type_of(port)
            .map(|it| ws.qualified_name_of(it))
            .unwrap_or_default()
    };
    assert_eq!(type_of(&ws, plain), "V::P");
    assert_eq!(type_of(&ws, other), "V::P::~P");
    // and the conjugate carries what the original declares
    let conjugate = ws.model().type_of(other).expect("typed");
    let inherited: Vec<String> = ws
        .supertypes(conjugate)
        .iter()
        .map(|&up| ws.qualified_name_of(up))
        .collect();
    assert_eq!(inherited, ["V::P"]);
}

/// `end cart : ShoppingCart crosses selectedProduct.inCart` says which
/// feature of the other end this one is reached across, and the standard
/// makes a `CrossSubsetting` of it. Written in the same shape as
/// `subsets`, it was arriving as a plain subsetting -- so no model built
/// here had a cross subsetting anywhere, and the constraint about
/// owning at most one could not be asked.
///
/// What an end crosses to is reached through the ends of the
/// association, this one included, so the path may start with the very
/// name being declared.
#[test]
fn an_end_that_crosses_says_so_with_a_cross_subsetting() {
    let mut ws = Workspace::new();
    let file = ws.add_file(
        "a.kerml",
        "package K {\n\
         \tclass Cart;\n\
         \tclass Product;\n\
         \tassoc Selection {\n\
         \t\tend cart : Cart crosses selectedProduct.inCart {\n\
         \t\t\tmember feature inCart : Cart;\n\
         \t\t}\n\
         \t\tend selectedProduct : Product {\n\
         \t\t\tmember feature inCart : Product;\n\
         \t\t}\n\
         \t}\n\
         }\n",
    );
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
    let root = ws.file_roots(file)[0];
    let cart = ws
        .model()
        .descendants(root)
        .into_iter()
        .find(|&id| ws.model().name(id) == Some("cart"))
        .expect("the end is declared");
    let crossings: Vec<sysml_model::ElementId> = ws
        .model()
        .owned(cart)
        .iter()
        .copied()
        .filter(|&it| ws.model().kind(it) == ElementKind::CrossSubsetting)
        .collect();
    assert_eq!(crossings.len(), 1, "one cross subsetting, and it is one");
    let crossing = crossings[0];
    // it is a subsetting, so the end that declares it is its
    // subsettingFeature; what it crosses to is the narrower name
    assert_eq!(
        ws.model().get(crossing, "subsettingFeature"),
        Some(&sysml_model::Value::Ref(cart))
    );
    // and what it crosses to is a chain and not a name: "the target of
    // a cross subsetting relationship must be a feature chain in which
    // the first feature is the other association end and the second
    // feature is the cross feature for that end"
    let crossed = ws
        .model()
        .get(crossing, "crossedFeature")
        .and_then(sysml_model::Value::as_id)
        .expect("what it crosses to");
    let chain: Vec<String> = ws
        .model()
        .get(crossed, "chainingFeature")
        .and_then(sysml_model::Value::as_ids)
        .expect("the chain it names")
        .to_vec()
        .into_iter()
        .map(|step| ws.qualified_name_of(step))
        .collect();
    assert_eq!(
        chain,
        [
            "K::Selection::selectedProduct",
            "K::Selection::selectedProduct::inCart"
        ]
    );
}

/// A union says which things a type is one of, not which type it is.
///
/// `classifier U unions A, B;` narrows an extent; it does not
/// specialize, and nothing is inherited through it. Read as a
/// specialization, a connector typed by a union of two associations had
/// the ends of both -- four where it relates two -- and
/// `validateConnectorBinarySpecialization` says a connector with more
/// than two ends does not specialize the binary link.
#[test]
fn a_union_is_not_a_specialization() {
    let mut ws = Workspace::new();
    let file = ws.add_file(
        "a.kerml",
        "package K {\n\
         \tclassifier T;\n\
         \tassoc One { end feature a : T; end feature b : T; }\n\
         \tassoc Two { end feature c : T; end feature d : T; }\n\
         \tclassifier Either unions One, Two;\n\
         }\n",
    );
    ws.resolve_all();
    let root = ws.file_roots(file)[0];
    let either = ws
        .model()
        .descendants(root)
        .into_iter()
        .find(|&id| ws.model().name(id) == Some("Either"))
        .expect("the union is declared");
    // what it unions is not what it specializes
    assert_eq!(ws.supertypes(either), Vec::new());
}

/// A succession that names one end reifies both.
///
/// `then b;` says where the flow goes and not where it comes from, and
/// the answer is its neighbour in the same body. That neighbour was
/// kept as a related feature and not as an end, so
/// `connectorEnd->at(1)` reached past it to the one that was written --
/// and the four constraints that count what a control node is joined by
/// read the ends in the order the connector relates them.
#[test]
fn a_succession_that_names_one_end_reifies_both() {
    let mut ws = Workspace::new();
    let file = ws.add_file(
        "a.sysml",
        "action def A {\n\taction one;\n\tthen two;\n\taction two;\n}\n",
    );
    ws.resolve_all();
    let root = ws.file_roots(file)[0];
    let succession = ws
        .model()
        .owned(root)
        .iter()
        .copied()
        .find(|&it| ws.model().kind(it).is_a(ElementKind::SuccessionAsUsage))
        .expect("the `then` writes one");
    let reaches: Vec<Option<&str>> = ws
        .model()
        .owned(succession)
        .iter()
        .copied()
        .filter(|&it| ws.model().get(it, "isEnd") == Some(&sysml_model::Value::Bool(true)))
        .map(|end| {
            sysml_model::end_reaches(ws.model(), end)
                .last()
                .and_then(|&it| ws.model().name(it))
        })
        .collect();
    // the one it runs from is the step above it, and it comes first
    assert_eq!(reaches, [Some("one"), Some("two")]);
    assert_eq!(
        ws.model()
            .get(succession, "relatedFeature")
            .and_then(sysml_model::Value::as_ids)
            .unwrap_or_default()
            .iter()
            .map(|&it| ws.model().name(it))
            .collect::<Vec<_>>(),
        [Some("one"), Some("two")]
    );
}

/// A relationship is binary where it relates exactly two things, and
/// nothing specializes what specializes it.
///
/// `validateConnectorBinarySpecialization` -- "if a Connector has more
/// than two connectorEnds, then it must not specialize, directly or
/// indirectly, the Association BinaryLink" -- and
/// `validateAssociationBinarySpecialization` says the same of an
/// association. What a type implicitly specializes is not read off its
/// metaclass alone: the pilot implementation writes `numEnds != 2 ?
/// base : binary` for every one of them, counted over the ends the type
/// owns. A definition that declares none -- `abstract connection def
/// Multicausation` -- is not binary either, and everything built on it
/// was.
///
/// And `Connections::Connection` is a connection definition like any
/// other, so the base its metaclass names is `BinaryConnection` --
/// which specializes it: implied that way round, the library's own base
/// for every connection was binary.
#[test]
fn what_relates_exactly_two_things_is_binary() {
    let mut ws = Workspace::new();
    ws.load_dir(std::path::Path::new(
        "../../vendor/sysml-v2-release/sysml.library",
    ))
    .expect("the library is a submodule");
    let file = ws.add_file(
        "a.sysml",
        "package K {\n\
         \tpart def A;\n\
         \tconnection def Two { end a : A; end b : A; }\n\
         \tconnection def Three { end a : A; end b : A; end c : A; }\n\
         \tconnection def None;\n\
         \tpart def Holder {\n\
         \t\tconnection two : Two;\n\
         \t\tconnection three : Three;\n\
         \t}\n\
         }\n",
    );
    ws.resolve_all();
    let root = ws.file_roots(file)[0];
    let of = |ws: &Workspace, name: &str| {
        ws.model()
            .descendants(root)
            .into_iter()
            .find(|&id| ws.model().name(id) == Some(name))
            .unwrap_or_else(|| panic!("`{name}` is declared"))
    };
    let (two, three) = (of(&ws, "Two"), of(&ws, "Three"));
    let connection = ws
        .model()
        .ids()
        .find(|&id| ws.qualified_name_of(id) == "Connections::Connection")
        .expect("the library declares it");
    fn reaches(ws: &mut Workspace, from: sysml_model::ElementId, wanted: &str) -> bool {
        let mut queue = vec![from];
        let mut seen = Vec::new();
        while let Some(at) = queue.pop() {
            if ws.qualified_name_of(at) == wanted {
                return true;
            }
            if seen.contains(&at) {
                continue;
            }
            seen.push(at);
            queue.extend(ws.supertypes(at));
        }
        false
    }
    assert!(reaches(&mut ws, two, "Connections::BinaryConnection"));
    assert!(!reaches(&mut ws, three, "Connections::BinaryConnection"));
    // and one that declares no ends at all relates no two things
    let bare = of(&ws, "None");
    assert!(!reaches(&mut ws, bare, "Connections::BinaryConnection"));
    // and a usage declares no ends of its own, so what it relates is
    // what it is typed by
    let (of_two, of_three) = (of(&ws, "two"), of(&ws, "three"));
    assert!(reaches(&mut ws, of_two, "Connections::BinaryConnection"));
    assert!(!reaches(&mut ws, of_three, "Connections::BinaryConnection"));
    // and the library's own base for every connection is not a kind of
    // the binary one that specializes it
    let ups: Vec<String> = ws
        .supertypes(connection)
        .into_iter()
        .map(|it| ws.qualified_name_of(it))
        .collect();
    assert!(
        !ups.contains(&"Connections::BinaryConnection".to_string()),
        "{ups:?}"
    );
}

/// An `else` writes where a guard that did not hold goes, and a
/// connector's parentheses hold its ends wherever they stand.
///
/// `DefaultTargetSuccession : TransitionUsage = 'else'
/// TransitionSuccessionMember`, and `NaryConnectorDeclaration :
/// Connector = FeatureDeclaration? '(' ConnectorEndMember ','
/// ConnectorEndMember ( ',' ConnectorEndMember )* ')'` -- KerML writes
/// the list after the declaration with no keyword at all.
#[test]
fn an_else_and_a_list_of_ends_say_what_they_relate() {
    let mut ws = Workspace::new();
    let file = ws.add_file(
        "a.kerml",
        "class C {\n\
         \tfeature a;\n\
         \tfeature b;\n\
         \tfeature c;\n\
         \tconnector ps : P (a, b, c);\n\
         \tconnector counted : P ([1] a, [0..1] b, [1] c);\n\
         \tclassifier P;\n\
         }\n",
    );
    ws.resolve_all();
    let root = ws.file_roots(file)[0];
    let connector = ws
        .model()
        .descendants(root)
        .into_iter()
        .find(|&id| ws.model().name(id) == Some("ps"))
        .expect("the connector is declared");
    assert_eq!(
        ws.model()
            .get(connector, "relatedFeature")
            .and_then(sysml_model::Value::as_ids)
            .unwrap_or_default()
            .iter()
            .map(|&it| ws.model().name(it))
            .collect::<Vec<_>>(),
        [Some("a"), Some("b"), Some("c")]
    );
    // and each end may count what it relates in front of its name,
    // which the parser reads as a declaration of its own
    let counted = ws
        .model()
        .descendants(root)
        .into_iter()
        .find(|&id| ws.model().name(id) == Some("counted"))
        .expect("the connector is declared");
    assert_eq!(
        ws.model()
            .get(counted, "relatedFeature")
            .and_then(sysml_model::Value::as_ids)
            .unwrap_or_default()
            .iter()
            .map(|&it| ws.model().name(it))
            .collect::<Vec<_>>(),
        [Some("a"), Some("b"), Some("c")]
    );

    // `if x > 1 then A2; else A3;` writes two branches of one decision,
    // and the second names where it goes after the `else`
    let mut ws = Workspace::new();
    let file = ws.add_file(
        "b.sysml",
        "action def A {\n\
         \tattribute x;\n\
         \tdecide;\n\
         \tif x > 1 then A2;\n\
         \telse A3;\n\
         \taction A2;\n\
         \taction A3;\n\
         }\n",
    );
    ws.resolve_all();
    let root = ws.file_roots(file)[0];
    let branches: Vec<Vec<Option<&str>>> = ws
        .model()
        .owned(root)
        .iter()
        .copied()
        .filter(|&it| ws.model().kind(it) == ElementKind::TransitionUsage)
        .map(|it| {
            ws.model()
                .owned(it)
                .iter()
                .copied()
                .find(|&c| ws.model().kind(c).is_a(ElementKind::SuccessionAsUsage))
                .and_then(|s| {
                    ws.model()
                        .get(s, "relatedFeature")
                        .and_then(sysml_model::Value::as_ids)
                })
                .unwrap_or_default()
                .iter()
                .map(|&r| ws.model().name(r))
                .collect()
        })
        .collect();
    assert_eq!(branches.len(), 2);
    assert_eq!(branches[1], [None, Some("A3")]);
}

/// A transition relates through the succession it owns, and an n-ary
/// connect writes each of its ends in the list.
///
/// "A TransitionUsage is not a Connector: what it relates it relates
/// through a Succession of its own", so that is where its two ends
/// belong -- read as the transition's, the succession it owns related
/// nothing and the constraints that count what a control node is joined
/// by had no ends to read. And `connect ( cause1 ::> causer1, cause2
/// ::> causer2 )` writes each end in the list, named and referring, the
/// same way a binary one writes two.
#[test]
fn a_transition_relates_through_its_succession_and_a_list_writes_ends() {
    let mut ws = Workspace::new();
    let file = ws.add_file(
        "a.sysml",
        "part def P {\n\
         \tpart a;\n\
         \tpart b;\n\
         \tconnect ( c1 ::> a, c2 ::> b );\n\
         \tstate def S {\n\
         \t\tstate off;\n\
         \t\tstate on;\n\
         \t\ttransition t first off then on;\n\
         \t}\n\
         }\n",
    );
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
    let root = ws.file_roots(file)[0];
    let named = |ws: &Workspace, name: &str| {
        ws.model()
            .descendants(root)
            .into_iter()
            .find(|&id| ws.model().name(id) == Some(name))
            .unwrap_or_else(|| panic!("`{name}` is declared"))
    };
    // the ends of a transition are the succession's, and so is what it
    // relates
    let transition = named(&ws, "t");
    let succession = ws
        .model()
        .owned(transition)
        .iter()
        .copied()
        .find(|&it| ws.model().kind(it).is_a(ElementKind::SuccessionAsUsage))
        .expect("a transition owns one");
    assert!(ws.model().get(transition, "relatedFeature").is_none());
    assert_eq!(
        ws.model()
            .get(succession, "relatedFeature")
            .and_then(sysml_model::Value::as_ids)
            .unwrap_or_default()
            .len(),
        2
    );
    assert_eq!(
        ws.model()
            .owned(succession)
            .iter()
            .filter(|&&it| ws.model().get(it, "isEnd") == Some(&sysml_model::Value::Bool(true)))
            .count(),
        2
    );
    // and each of a list's ends is one
    let connector = ws
        .model()
        .owned(root)
        .iter()
        .copied()
        .find(|&it| ws.model().kind(it).is_a(ElementKind::Connector))
        .expect("the connect statement builds one");
    let ends: Vec<Option<&str>> = ws
        .model()
        .owned(connector)
        .iter()
        .copied()
        .filter(|&it| ws.model().get(it, "isEnd") == Some(&sysml_model::Value::Bool(true)))
        .map(|it| ws.model().name(it))
        .collect();
    assert_eq!(ends, [Some("c1"), Some("c2")]);
}

/// A named declaration with a reference is the end itself.
///
/// `interface i : WHI connect [1] lugNutPort ::> wheel.lugNutPort to
/// [1] shankPort ::> hub.shankPort;` -- `ConnectorEnd : Feature = (
/// OwnedCrossMultiplicityMember )? ( declaredName = NAME REFERENCES )?
/// OwnedReferenceSubsetting`, so what stands after `connect` is the end
/// and what it refers to is what the connector relates. Read as a
/// member of the interface, twenty-one of the corpus related nothing at
/// all.
#[test]
fn a_named_declaration_with_a_reference_is_the_end() {
    let mut ws = Workspace::new();
    let file = ws.add_file(
        "a.sysml",
        "part def P {\n\
         \tport def LC;\n\
         \tport def SC;\n\
         \tinterface def WHI { end lc : LC; end sc : SC; }\n\
         \tpart w { port lcp : LC; }\n\
         \tpart h { port scp : SC; }\n\
         \tinterface i : WHI connect [1] lcp ::> w.lcp to [1] scp ::> h.scp;\n\
         }\n",
    );
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
    let root = ws.file_roots(file)[0];
    let interface = ws
        .model()
        .descendants(root)
        .into_iter()
        .find(|&id| ws.model().name(id) == Some("i"))
        .expect("the interface is declared");
    let ends: Vec<Option<&str>> = ws
        .model()
        .owned(interface)
        .iter()
        .copied()
        .filter(|&it| ws.model().get(it, "isEnd") == Some(&sysml_model::Value::Bool(true)))
        .map(|it| ws.model().name(it))
        .collect();
    assert_eq!(ends, [Some("lcp"), Some("scp")]);
    // and what they refer to is what the interface relates
    assert_eq!(
        ws.model()
            .get(interface, "relatedFeature")
            .and_then(sysml_model::Value::as_ids)
            .unwrap_or_default()
            .iter()
            .map(|&it| ws.qualified_name_of(it))
            .collect::<Vec<_>>(),
        ["P::w::lcp", "P::h::scp"]
    );
    // and it relates them through a chain: `w.lcp` is `lcp` of that `w`,
    // which is what the abstract syntax the OMG publishes stands under
    // the reference. What the connector relates is the feature the
    // chain ends at, the same as for an end written without a name.
    let steps: Vec<usize> =
        ws.model()
            .owned(interface)
            .iter()
            .copied()
            .filter(|&it| ws.model().get(it, "isEnd") == Some(&sysml_model::Value::Bool(true)))
            .filter_map(|end| {
                ws.model().owned(end).iter().copied().find(|&it| {
                    ws.model().kind(it) == sysml_model::ElementKind::ReferenceSubsetting
                })
            })
            .filter_map(|it| {
                ws.model()
                    .get(it, "referencedFeature")
                    .and_then(sysml_model::Value::as_id)
            })
            .map(|reached| ws.model().chaining_feature(reached).len())
            .collect();
    assert_eq!(steps, [2, 2], "each reference names a chain of two");
}

/// A connector end is one thing.
///
/// `validateFeatureEndMultiplicity` -- "if a Feature has isEnd = true,
/// then it must have multiplicity 1..1" -- and the notation writes it
/// nowhere: what stands before an end in `first [0..1] decide then
/// [0..1] merge` is the cross multiplicity, how many things at the far
/// end go with one at this one. Without the range, the four constraints
/// that count what a control node is joined by have nothing to read.
/// An end may carry a multiplicity, and what is inside the brackets is
/// not part of the name beside it.
///
/// `connector ps : P ([0..*] myCart, ...)` writes a bound and then a
/// name. The `*` of the bound was counted as a step of that name, so
/// the steps said two where the segments said one, and reading a prefix
/// of the name ran off the end of it -- a panic, on a model that parses
/// and resolves. `ProductSelection_N_ary.kerml` writes exactly this
/// with `[1]`, where a bound of one token happened to agree.
#[test]
fn a_bound_written_before_an_end_is_not_part_of_its_name() {
    let mut ws = Workspace::new();
    let source = "package P {\n\
         \tclassifier Cart;\n\
         \tassoc A {\n\t\tend feature c : Cart[1];\n\t\tend feature d : Cart[1];\n\t}\n\
         \tclassifier Holder {\n\
         \t\tfeature myCart : Cart[1];\n\
         \t\tfeature other : Cart[1];\n\
         \t\tconnector a : A ([0..*] myCart, [0..1] other);\n\
         \t}\n}\n";
    let file = ws.add_file("m.kerml", source);
    let stats = ws.resolve_files(&[file]);
    assert_eq!(stats.unresolved, 0, "{:?}", ws.unresolved());

    // and the ends reach what they name, one step each
    let connector = ws
        .model()
        .ids()
        .find(|&id| ws.model().name(id) == Some("a"))
        .expect("the connector is declared");
    let related: Vec<String> = ws
        .model()
        .get(connector, "relatedFeature")
        .and_then(sysml_model::Value::as_ids)
        .unwrap_or_default()
        .iter()
        .map(|&it| ws.qualified_name_of(it))
        .collect();
    assert_eq!(related, ["P::Holder::myCart", "P::Holder::other"]);
}

#[test]
fn a_connector_end_is_one_thing() {
    let mut ws = Workspace::new();
    let file = ws.add_file(
        "a.sysml",
        "part def P {\n\tpart a;\n\tpart b;\n\tconnect a to b;\n}\n",
    );
    ws.resolve_all();
    let root = ws.file_roots(file)[0];
    let connector = ws
        .model()
        .owned(root)
        .iter()
        .copied()
        .find(|&it| ws.model().kind(it).is_a(ElementKind::Connector))
        .expect("the connection is declared");
    for end in ws.model().owned(connector).to_vec() {
        if ws.model().get(end, "isEnd") != Some(&sysml_model::Value::Bool(true)) {
            continue;
        }
        let range = ws
            .model()
            .get(end, "multiplicity")
            .and_then(sysml_model::Value::as_id)
            .expect("an end counts one");
        assert_eq!(ws.model().kind(range), ElementKind::MultiplicityRange);
        // the bounds are its first owned members, in order, which is
        // what `validateMultiplicityRangeBounds` asks for
        let bounds: Vec<Option<&sysml_model::Value>> = ws
            .model()
            .owned(range)
            .iter()
            .map(|&it| ws.model().get(it, "value"))
            .collect();
        assert_eq!(
            bounds,
            [
                Some(&sysml_model::Value::Int(1)),
                Some(&sysml_model::Value::Int(1))
            ]
        );
        assert_eq!(
            ws.model()
                .get(range, "lowerBound")
                .and_then(sysml_model::Value::as_id),
            ws.model().owned(range).first().copied()
        );
    }
}

/// A binding binds the two ends written around its `=`.
///
/// SysML writes `binding [1] bind [0..*] base.edges = [0..*] be;` and
/// KerML `binding ab of a = b;` or `binding a = b;`. A declaration
/// stands only in front of the keyword that introduces the first end,
/// so without a `bind` or an `of` the name after `binding` is that end
/// -- and a binding that writes no `=` at all declares a name and
/// nothing else. Eighty-eight of the library's bindings were binding
/// nothing, and the `=` was being read a second time as a value, which
/// wrote two edits over the one name on a rename.
#[test]
fn a_binding_binds_what_was_written_around_its_equals() {
    let mut ws = Workspace::new();
    let file = ws.add_file(
        "a.kerml",
        "class C {\n\
         \tfeature a;\n\
         \tfeature b;\n\
         \tbinding a = b;\n\
         \tbinding named of a = b;\n\
         \tbinding [1] a = [1] b;\n\
         \tbinding empty { doc /* nothing bound */ }\n\
         }\n",
    );
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
    let root = ws.file_roots(file)[0];
    let bound: Vec<(Option<String>, Vec<Option<&str>>)> = ws
        .model()
        .owned(root)
        .iter()
        .copied()
        .filter(|&it| ws.model().kind(it).is_a(ElementKind::BindingConnector))
        .map(|it| {
            (
                ws.model().name(it).map(str::to_string),
                ws.model()
                    .get(it, "relatedFeature")
                    .and_then(sysml_model::Value::as_ids)
                    .unwrap_or_default()
                    .iter()
                    .map(|&r| ws.model().name(r))
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        bound,
        [
            (None, vec![Some("a"), Some("b")]),
            (Some("named".to_string()), vec![Some("a"), Some("b")]),
            // the count before an end is not the end
            (None, vec![Some("a"), Some("b")]),
            // and one that binds nothing keeps the name it declared
            (Some("empty".to_string()), vec![]),
        ]
    );
    // the `=` is an end and not a value, so what follows it is recorded
    // once -- recorded twice, a rename writes two edits over one name
    let mut ranges: Vec<_> = ws.references().iter().map(|it| it.name_range).collect();
    ranges.sort_by_key(|it| (usize::from(it.start()), usize::from(it.end())));
    let written = ranges.len();
    ranges.dedup();
    assert_eq!(ranges.len(), written, "a name recorded twice");
}

/// A KerML connector relates what it was written between, and names
/// itself only where it wrote a `from`.
///
/// `BinaryConnectorDeclaration : Connector = ( FeatureDeclaration?
/// 'from' | isSufficient ?= 'all' 'from'? )? ConnectorEndMember 'to'
/// ConnectorEndMember` -- a declaration is written only in front of a
/// `from`, so `connector eng to tank;` names no connector at all and
/// relates `eng` to `tank`. Read as a name, the enclosing class came to
/// have two members called `eng` and the connector related one thing;
/// and a `connector` was never asked for its ends in the first place,
/// because only a usage was.
#[test]
fn a_kerml_connector_relates_what_it_was_written_between() {
    let mut ws = Workspace::new();
    let file = ws.add_file(
        "a.kerml",
        "class C {\n\
         \tfeature eng;\n\
         \tfeature tank;\n\
         \tfeature b;\n\
         \tconnector eng to tank;\n\
         \tconnector named from eng to tank;\n\
         \tconnector eng ::> tank to b;\n\
         }\n",
    );
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
    let root = ws.file_roots(file)[0];
    let related: Vec<(Option<String>, Vec<Option<&str>>)> = ws
        .model()
        .owned(root)
        .iter()
        .copied()
        .filter(|&it| ws.model().kind(it).is_a(ElementKind::Connector))
        .map(|it| {
            (
                ws.model().name(it).map(str::to_string),
                ws.model()
                    .get(it, "relatedFeature")
                    .and_then(sysml_model::Value::as_ids)
                    .unwrap_or_default()
                    .iter()
                    .map(|&r| ws.model().name(r))
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        related,
        [
            // no `from`, so the name is the end it runs from
            (None, vec![Some("eng"), Some("tank")]),
            // with one, the name is the connector's own
            (Some("named".to_string()), vec![Some("eng"), Some("tank")]),
            // and what an end refers to wins over the name it was given
            (None, vec![Some("tank"), Some("b")]),
        ]
    );
}

/// An owned cross feature belongs to the end written after it, and
/// carries the relationships the standard implies for it.
///
/// `end owningEntities[1..*] feature owner : LegalEntity;` declares the
/// end `owner`, not the end `owningEntities`: "owned cross features are
/// in the namespace of the owning association ends, so their names are
/// qualified by the name of the association ends, e.g.
/// `LegalAssetOwnership::owner::owningEntities`". And where the end
/// redefines another, its cross feature subsets that one's -- nothing
/// writes that down, and `validateFeatureCrossFeatureSpecialization`
/// asks for it back.
#[test]
fn a_cross_feature_is_the_ends_and_subsets_what_the_end_it_redefines_crosses_to() {
    let mut ws = Workspace::new();
    let file = ws.add_file(
        "a.kerml",
        "package K {\n\
         \tclass Cart;\n\
         \tclass Product;\n\
         \tassoc Selection {\n\
         \t\tend inCart[0..1] feature cart : Cart;\n\
         \t\tend selectedProducts[0..*] feature selectedProduct : Product;\n\
         \t}\n\
         \tassoc One specializes Selection {\n\
         \t\tend inCart1[0..1] feature cart redefines cart;\n\
         \t\tend selectedProduct1[0..1] feature selectedProduct redefines selectedProduct;\n\
         \t}\n\
         }\n",
    );
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
    let root = ws.file_roots(file)[0];
    let named = |name: &str| {
        ws.model()
            .descendants(root)
            .into_iter()
            .find(|&id| ws.qualified_name_of(id) == name)
            .unwrap_or_else(|| panic!("`{name}` is declared"))
    };
    // the ends of the association are what the declarations named, and
    // the cross features are under them
    let selection = named("K::Selection");
    let ends: Vec<Option<&str>> = ws
        .model()
        .owned(selection)
        .iter()
        .filter(|&&it| ws.model().get(it, "isEnd") == Some(&sysml_model::Value::Bool(true)))
        .map(|&it| ws.model().name(it))
        .collect();
    assert_eq!(ends, [Some("cart"), Some("selectedProduct")]);
    let cross = named("K::Selection::cart::inCart");
    assert_eq!(
        ws.model().get(cross, "isEnd"),
        None,
        "a cross feature is not an end"
    );
    // and the redefining end's cross feature subsets the redefined
    // end's, implied and marked as such
    let mine = named("K::One::cart::inCart1");
    let implied: Vec<(String, Option<&sysml_model::Value>)> = ws
        .model()
        .owned(mine)
        .iter()
        .filter(|&&it| ws.model().kind(it) == ElementKind::Subsetting)
        .map(|&it| {
            (
                ws.qualified_name_of(
                    ws.model()
                        .get(it, "subsettedFeature")
                        .and_then(sysml_model::Value::as_id)
                        .expect("what it subsets"),
                ),
                ws.model().get(it, "isImplied"),
            )
        })
        .collect();
    assert_eq!(
        implied,
        [(
            "K::Selection::cart::inCart".to_string(),
            Some(&sysml_model::Value::Bool(true))
        )]
    );
}

/// An end declared beside a supertype's redefines the one at the same
/// position.
///
/// "If a Feature has isEnd = true and an owningType that is not empty,
/// then, for each direct supertype of its owningType, it must redefine
/// the endFeature at the same position, if any." Nothing writes it, and
/// without it a connector inherits the ends of what types it beside its
/// own.
#[test]
fn an_end_redefines_the_one_at_its_position_in_what_its_type_specializes() {
    let mut ws = Workspace::new();
    let file = ws.add_file(
        "a.kerml",
        "package K {\n\
         \tclass Thing;\n\
         \tassoc Pair {\n\
         \t\tend feature one : Thing;\n\
         \t\tend feature two : Thing;\n\
         \t}\n\
         \tassoc Narrower specializes Pair {\n\
         \t\tend feature left : Thing;\n\
         \t\tend feature right : Thing;\n\
         \t}\n\
         }\n",
    );
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
    let root = ws.file_roots(file)[0];
    let named = |name: &str| {
        ws.model()
            .descendants(root)
            .into_iter()
            .find(|&id| ws.qualified_name_of(id) == name)
            .unwrap_or_else(|| panic!("`{name}` is declared"))
    };
    let redefined = |of: &str| -> Vec<String> {
        ws.model()
            .owned(named(of))
            .iter()
            .filter(|&&it| ws.model().kind(it) == ElementKind::Redefinition)
            .filter(|&&it| ws.model().get(it, "isImplied") == Some(&sysml_model::Value::Bool(true)))
            .filter_map(|&it| {
                ws.model()
                    .get(it, "redefinedFeature")
                    .and_then(sysml_model::Value::as_id)
            })
            .map(|it| ws.qualified_name_of(it))
            .collect()
    };
    assert_eq!(redefined("K::Narrower::left"), ["K::Pair::one"]);
    assert_eq!(redefined("K::Narrower::right"), ["K::Pair::two"]);
    // what declares the ends redefines nothing of its own
    assert_eq!(redefined("K::Pair::one"), Vec::<String>::new());
}

/// `end` is written once. The corpus writes it on the outer feature and
/// nests redefinitions of it without repeating the keyword, and the
/// standard says as much: a feature redefining an end is one, and an
/// end is not composite.
#[test]
fn a_feature_that_redefines_an_end_is_an_end_and_is_not_composite() {
    let mut ws = Workspace::new();
    let file = ws.add_file(
        "e.kerml",
        "package K {\n\
         \tclass C;\n\
         \tassoc A {\n\
         \t\tend feature outer : C;\n\
         \t}\n\
         \tassoc B specializes A {\n\
         \t\tfeature middle redefines outer;\n\
         \t}\n\
         \tassoc D specializes B {\n\
         \t\tfeature inner redefines middle;\n\
         \t}\n\
         }\n",
    );
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
    let root = ws.file_roots(file)[0];
    let named = |want: &str| {
        ws.model()
            .descendants(root)
            .into_iter()
            .find(|&id| ws.model().name(id) == Some(want))
            .unwrap_or_else(|| panic!("`{want}` is declared"))
    };
    // written, redefining, and redefining a redefinition
    for want in ["outer", "middle", "inner"] {
        let id = named(want);
        assert_eq!(
            ws.model().get(id, "isEnd"),
            Some(&sysml_model::Value::Bool(true)),
            "`{want}` is an end"
        );
        assert_eq!(
            ws.model().get(id, "isComposite"),
            Some(&sysml_model::Value::Bool(false)),
            "`{want}` is not composite"
        );
    }
}

/// `first x;` on its own is `InitialNodeMember : FeatureMembership =
/// MemberPrefix 'first' memberFeature = [QualifiedName]`: it names
/// which step comes first and writes no flow at all. The flow is what a
/// `then` writes -- `TargetSuccession : SuccessionAsUsage =
/// SourceEndMember 'then' ConnectorEndMember` -- so `first a; then b;`
/// is one succession and not two.
///
/// Read as a succession as well, the `first` took the flow its `then`
/// writes and left that one relating its own declaration to itself: the
/// corpus had merge and fork nodes with a succession from themselves to
/// themselves.
#[test]
fn a_first_names_the_step_that_comes_first_and_writes_no_flow() {
    let mut ws = Workspace::new();
    let file = ws.add_file(
        "f.sysml",
        "action def A {\n\
         \taction p;\n\
         \tfirst p;\n\
         \tthen merge m;\n\
         \tthen action q;\n\
         }\n",
    );
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "unresolved: {:?}", ws.unresolved());
    let root = ws.file_roots(file)[0];
    let model = ws.model();
    let flows: Vec<(String, String)> = model
        .owned(root)
        .iter()
        .filter(|&&it| model.kind(it) == ElementKind::SuccessionAsUsage)
        .map(|&it| {
            let ends = model.related_feature(it);
            (
                ends.first()
                    .map(|&f| ws.qualified_name_of(f))
                    .unwrap_or_default(),
                ends.get(1)
                    .map(|&f| ws.qualified_name_of(f))
                    .unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        flows,
        [
            ("A::p".to_string(), "A::m".to_string()),
            ("A::m".to_string(), "A::q".to_string()),
        ]
    );
    // and the `first` names `p` without owning it
    let named = model
        .owned(root)
        .iter()
        .copied()
        .find(|&it| model.kind(it) == ElementKind::Membership)
        .expect("`first p;` is a membership");
    assert_eq!(
        model.get(named, "memberElement").and_then(|it| match it {
            sysml_model::Value::Ref(to) => Some(ws.qualified_name_of(*to)),
            _ => None,
        }),
        Some("A::p".to_string())
    );
}

/// A dotted operand of a subsetting names a chain, not the feature the
/// last step names anywhere. `Occurrences.kermlx` -- the abstract syntax
/// the OMG publishes for its own library -- stands a `Feature` of its own
/// under `subset laterOccurrence.successors subsets
/// earlierOccurrence.successors;`, carrying each step as a
/// `FeatureChaining`. Read as the last step alone, both ends of that
/// statement are the one `successors`, and what features them is read off
/// a feature the path never reached.
///
/// A specialization relates types rather than features, and the published
/// abstract syntax carries no chain beneath one.
#[test]
fn a_dotted_operand_of_a_subsetting_is_the_chain_and_not_its_last_step() {
    use sysml_model::{ElementKind, Value};

    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file(
        "model.kerml",
        "package K {\n\
         \tclassifier C {\n\t\tfeature deeper;\n\t}\n\
         \tfeature outer : C;\n\
         \tfeature other : C;\n\
         \tsubset outer.deeper subsets other.deeper;\n\
         \tfeature plain subsets outer.deeper;\n}\n",
    );
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "{stats:?}");

    let steps = |id| match ws.model().get(id, "chainingFeature") {
        Some(Value::RefList(chain)) => chain.len(),
        _ => 0,
    };
    // the statement: both ends are chains of two, and neither is the
    // `deeper` that `C` declares
    let statement = ws
        .model()
        .ids()
        .find(|&id| {
            ws.model().kind(id) == ElementKind::Subsetting
                && ws
                    .model()
                    .owner(id)
                    .is_none_or(|o| ws.model().name(o) == Some("K"))
        })
        .expect("the subsetting statement is built");
    for end in ["subsettingFeature", "subsettedFeature"] {
        let reached = ws
            .model()
            .get(statement, end)
            .and_then(Value::as_id)
            .unwrap_or_else(|| panic!("`{end}` is resolved"));
        assert_eq!(steps(reached), 2, "{end} is a chain of two");
        assert_eq!(ws.model().name(reached), None, "{end} is a chain, unnamed");
    }

    // and the same operand written as part of a declaration
    let declared = ws
        .model()
        .ids()
        .find(|&id| ws.model().name(id) == Some("plain"))
        .expect("`plain` is declared");
    let subsetting = ws
        .model()
        .owned(declared)
        .iter()
        .copied()
        .find(|&it| ws.model().kind(it) == ElementKind::Subsetting)
        .expect("its subsetting is reified");
    let reached = ws
        .model()
        .get(subsetting, "subsettedFeature")
        .and_then(Value::as_id)
        .expect("what it subsets is resolved");
    assert_eq!(steps(reached), 2, "a declared chain is a chain too");
}

/// What a `disjoint` says is looked at.
///
/// KerML writes disjointness two ways -- beside a declaration, as
/// `feature h2 ... disjoint from h1;`, and as a statement of its own,
/// `disjoint b.f.a from b.a;` -- and neither end of either was resolved.
/// A name that answers to nothing there was a model this toolchain
/// called sound, which is the one thing the corpus cannot check for
/// itself.
///
/// A dotted end is a chain, as it is for every other subsetting-shaped
/// relationship: the abstract syntax the OMG publishes stands one under
/// a `Disjoining` too.
#[test]
fn both_ends_of_a_disjointness_are_looked_at() {
    use sysml_model::{ElementKind, Value};

    let disjoinings = |ws: &Workspace| -> Vec<(Option<String>, Option<String>)> {
        ws.model()
            .ids()
            .filter(|&id| ws.model().kind(id) == ElementKind::Disjoining)
            .map(|id| {
                let end = |prop| {
                    ws.model()
                        .get(id, prop)
                        .and_then(Value::as_id)
                        .map(|at| ws.qualified_name_of(at))
                };
                (end("typeDisjoined"), end("disjoiningType"))
            })
            .collect()
    };

    // beside a declaration
    let mut ws = Workspace::new();
    ws.add_file(
        "k.kerml",
        "package K {\n\tclassifier A;\n\tfeature a : A;\n\tfeature b : A disjoint from a;\n}\n",
    );
    assert_eq!(ws.resolve_all().unresolved, 0);
    assert_eq!(
        disjoinings(&ws),
        [(Some("K::b".into()), Some("K::a".into()))]
    );

    // as a statement, named or not
    let mut ws = Workspace::new();
    ws.add_file(
        "k.kerml",
        "package K {\n\tclassifier A;\n\tclassifier B;\n\
         \tfeature a : A;\n\tfeature b : A;\n\
         \tdisjoint b from a;\n\tdisjoining d disjoint A from B;\n}\n",
    );
    assert_eq!(ws.resolve_all().unresolved, 0);
    assert_eq!(
        disjoinings(&ws),
        [
            (Some("K::b".into()), Some("K::a".into())),
            (Some("K::A".into()), Some("K::B".into())),
        ]
    );

    // a name that answers to nothing is reported, on either side
    for source in [
        "package K {\n\tclassifier A;\n\tfeature b : A disjoint from nope;\n}\n",
        "package K {\n\tclassifier A;\n\tfeature a : A;\n\tdisjoint nope from a;\n}\n",
        "package K {\n\tclassifier A;\n\tfeature b : A;\n\tdisjoint b from nope;\n}\n",
    ] {
        let mut ws = Workspace::new();
        ws.add_file("k.kerml", source);
        let stats = ws.resolve_all();
        assert_eq!(stats.unresolved, 1, "{:?}", ws.unresolved());
        assert_eq!(ws.unresolved()[0].name, "nope");
    }

    // and a dotted end is the chain, not the feature it ends at
    let mut ws = Workspace::new();
    ws.add_file(
        "k.kerml",
        "package K {\n\tclassifier C {\n\t\tfeature deep;\n\t}\n\
         \tfeature x : C;\n\tfeature y : C;\n\tdisjoint x.deep from y.deep;\n}\n",
    );
    assert_eq!(ws.resolve_all().unresolved, 0);
    let steps: Vec<usize> = ws
        .model()
        .ids()
        .filter(|&id| ws.model().kind(id) == ElementKind::Disjoining)
        .flat_map(|id| {
            ["typeDisjoined", "disjoiningType"]
                .iter()
                .filter_map(|prop| ws.model().get(id, prop).and_then(Value::as_id))
                .map(|at| ws.model().chaining_feature(at).len())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(steps, [2, 2], "both ends are chains of two");
}

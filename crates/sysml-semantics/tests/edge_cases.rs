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
    // and enumeration through the cyclic imports terminates too
    assert_eq!(ws.unresolved().len(), 2);
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
    assert!(labels.contains(&"nested"), "{labels:?}");
    assert!(labels.contains(&"marker"), "{labels:?}");
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
        "package P {\n    metadata def SemanticMetadata { attribute baseType; }\n    struct Left { feature onlyLeft; }\n    struct Right { feature onlyRight; }\n    metadata def Either :> SemanticMetadata {\n        :>> baseType = if true ? Left meta X else Right meta X;\n    }\n    #Either part def W { attribute :>> onlyLeft; attribute :>> onlyRight; }\n}\n",
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

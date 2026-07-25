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
    // the zz redefinitions fall back to self-resolution; what matters here
    // is that the succession/inv/binding implicit-supertype rows executed
    assert_eq!(ws.unresolved().len(), 0);
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
    // the self-referential base is skipped and the degenerate value yields
    // no base; `nothere` self-resolves via the effective-name fallback.
    // What matters is that both semantic_base edge branches executed.
    assert_eq!(ws.unresolved().len(), 0, "{:?}", ws.unresolved());
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

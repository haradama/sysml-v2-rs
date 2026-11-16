//! Two corners of resolution: a clash between inherited members of a type
//! whose supertype is a root declaration known only by its short name,
//! and alias chains deep enough to be set aside more than once.

use sysml_semantics::Workspace;

fn workspace(files: &[(&str, &str)]) -> Workspace {
    let mut ws = Workspace::default();
    for (name, text) in files {
        ws.add_file(*name, text);
    }
    ws.resolve_all();
    ws
}

/// A member inherited from another file's root declaration that has only
/// a short name is not reached through a root package of a name another
/// file shares, so the clash with the member inherited beside it stands.
#[test]
fn a_clash_through_a_root_declaration_known_by_its_short_name_stands() {
    let mut ws = workspace(&[
        ("a.sysml", "part def E { part x; }\npart def F :> S, E;\n"),
        ("b.sysml", "part def <S> { part x; }\n"),
    ]);
    let checked = ws.check_rules(&[0, 1]);
    let rules: Vec<&str> = checked
        .violations
        .iter()
        .map(|violation| violation.rule)
        .collect();
    assert!(
        rules.contains(&"validateNamespaceDistinguishibility"),
        "{rules:?}"
    );
}

/// Several aliases of one name are all met past the depth the walk goes,
/// and the chain still arrives at what it names.
#[test]
fn a_long_chain_of_repeated_aliases_still_resolves() {
    let n = 200;
    let mut text = String::from("package D {\n\tpart def X;\n");
    for i in 0..n {
        text.push_str(&format!("\talias A{i} for A{};\n", i + 1));
        text.push_str(&format!("\talias A{i} for A{};\n", i + 1));
    }
    text.push_str(&format!("\talias A{n} for X;\n\tpart first : A0;\n}}\n"));
    let ws = workspace(&[("m.sysml", &text)]);
    let names: Vec<&str> = ws.unresolved().iter().map(|u| u.name.as_str()).collect();
    assert!(!names.contains(&"A0"), "{names:?}");
}

/// A second answer to a name met past the depth the walk goes does not
/// stop the chain from arriving at what it names.
#[test]
fn a_second_answer_met_while_one_is_set_aside_changes_nothing() {
    let n = 100;
    let mut text = String::from("package D {\n\tpart def X;\n");
    for i in 0..n {
        text.push_str(&format!("\talias A{i} for A{};\n", i + 1));
    }
    text.push_str("\talias A2 for A64;\n");
    text.push_str(&format!("\talias A{n} for X;\n\tpart first : A0;\n}}\n"));
    let ws = workspace(&[("m.sysml", &text)]);
    let names: Vec<&str> = ws.unresolved().iter().map(|u| u.name.as_str()).collect();
    assert!(!names.contains(&"A0"), "{names:?}");
}

//! What a reach resolves, and what it leaves alone.

use sysml_semantics::Workspace;

const LIBRARY: &str = "package Lib {\n\tpart def Wheel;\n\tpart def Hub { part w : Wheel; }\n\tpart def Unrelated;\n}\n";

fn settled() -> Workspace {
    let mut ws = Workspace::new();
    ws.add_file("lib.sysml", LIBRARY);
    ws.resolve_all();
    ws
}

/// A document put on a library that is already resolved reaches into
/// nothing: every element of the library has been resolved once, and
/// resolving it again used to record every reference in it a second
/// time -- fourteen thousand of them, on the standard library, per
/// keystroke that asked.
#[test]
fn a_reach_does_not_re_enter_a_file_resolved_whole() {
    let mut ws = settled();
    let before = ws.references().len();
    let doc = ws.add_file(
        "doc.sysml",
        "package P { part def Car { part h : Lib::Hub; } }",
    );
    let stats = ws.resolve_reached(&[doc]);
    // the document's own reference, and no more
    assert_eq!(stats.unresolved, 0);
    assert_eq!(ws.references().len(), before + 2, "{:?}", ws.references());
    // a second time is the same
    let again = ws.clone();
    let mut again = again;
    again.resolve_files(&[doc]);
    assert_eq!(again.references().len(), before + 2);
}

/// On a library that has not been resolved, a reach follows what the
/// document names and what that names in turn, and stops there.
#[test]
fn a_reach_follows_what_is_named_and_stops() {
    let mut ws = Workspace::new();
    ws.add_file("lib.sysml", LIBRARY);
    let doc = ws.add_file(
        "doc.sysml",
        "package P { part def Car { part h : Lib::Hub; } }",
    );
    let stats = ws.resolve_reached(&[doc]);
    assert_eq!(stats.unresolved, 0);
    let named = |name: &str| {
        ws.named_elements()
            .find(|(_, declared)| *declared == name)
            .map(|(id, _)| id)
            .unwrap()
    };
    // `Hub` was reached, so the typing inside it resolved and `Wheel`
    // with it; `Unrelated` was named by nothing and is left as built
    let hub = named("Hub");
    let types = |of| ws.model().owned(of).to_vec();
    assert!(!types(hub).is_empty());
    let resolved_targets: Vec<&str> = ws
        .references()
        .iter()
        .filter_map(|r| ws.model().name(r.target))
        .collect();
    assert!(resolved_targets.contains(&"Hub"), "{resolved_targets:?}");
    assert!(resolved_targets.contains(&"Wheel"), "{resolved_targets:?}");
    assert!(
        !resolved_targets.contains(&"Unrelated"),
        "{resolved_targets:?}"
    );
}

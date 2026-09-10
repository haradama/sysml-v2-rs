//! What answers to a name other than its own.
//!
//! A feature that declares no name of its own answers to the name of
//! what it redefines, so one declaration can have several homes:
//! `part l : Logical { part :>> component; }` gives `component` a second
//! one, and `l.component` names that. A rename that stops at the
//! declaration leaves every mention of the others standing, which is
//! what [`Workspace::named_after`] is asked to prevent.

use sysml_semantics::Workspace;

fn resolved(text: &str) -> Workspace {
    let mut ws = Workspace::default();
    ws.add_file("a.sysml", text);
    ws.resolve_all();
    ws
}

fn named(ws: &Workspace, qualified: &str) -> sysml_model::ElementId {
    ws.model()
        .ids()
        .find(|&id| ws.qualified_name_of(id) == qualified)
        .unwrap_or_else(|| panic!("no element called `{qualified}`"))
}

/// One redefinition borrows the name, and so does a redefinition of
/// that: the answer is the whole chain and not the first step of it.
///
/// This is the part that has to be walked rather than scanned for. It
/// used to be walked by scanning the whole model once per step, which on
/// a model with the standard library in it cost a millisecond and a
/// quarter per step -- in the rename an editor is waiting on.
#[test]
fn a_chain_of_redefinitions_all_answer_to_the_one_name() {
    let ws = resolved(
        "package P {\n\
         \tpart def Base { part component; }\n\
         \tpart def Middle :> Base { part :>> component; }\n\
         \tpart def Leaf :> Middle { part :>> component; }\n\
         }\n",
    );
    let base = named(&ws, "P::Base::component");
    let heirs = ws.named_after(base);
    // They declare no name, so what tells them apart is what owns them.
    // `?` is how a nameless element is spelled, and both of these are
    // one -- which is the whole reason they answer to somebody else's
    // name.
    let mut spelled: Vec<String> = heirs.iter().map(|&it| ws.qualified_name_of(it)).collect();
    spelled.sort();
    assert_eq!(spelled, ["P::Leaf::?", "P::Middle::?"]);
}

/// A redefinition that declares a name of its own is its own name from
/// there on, and so is anything that redefines *it*.
#[test]
fn a_redefinition_that_names_itself_ends_the_chain() {
    let ws = resolved(
        "package P {\n\
         \tpart def Base { part component; }\n\
         \tpart def Middle :> Base { part named :>> component; }\n\
         \tpart def Leaf :> Middle { part :>> named; }\n\
         }\n",
    );
    let base = named(&ws, "P::Base::component");
    assert_eq!(ws.named_after(base), []);
}

/// A feature that redefines more than one thing answers to the first of
/// them alone, so renaming the second leaves it standing.
#[test]
fn only_the_first_of_several_redefinitions_lends_its_name() {
    let ws = resolved(
        "package P {\n\
         \tpart def Base { part driver; part driver_b; }\n\
         \tpart def Sub :> Base { part :>> driver :>> driver_b; }\n\
         }\n",
    );
    let first = named(&ws, "P::Base::driver");
    let second = named(&ws, "P::Base::driver_b");
    assert_eq!(ws.named_after(first).len(), 1, "it goes by `driver`");
    assert_eq!(ws.named_after(second), [], "and not by `driver_b`");
}

/// A feature nothing borrows from lends its name to nobody, and asking
/// is not an error.
#[test]
fn a_name_nothing_borrows_is_answered_by_nothing() {
    let ws = resolved("package P { part def Base { part alone; } }\n");
    assert_eq!(ws.named_after(named(&ws, "P::Base::alone")), []);
}

/// `::>` borrows a name the same way `:>>` does: a feature that says
/// what it refers to and declares no name of its own goes by the name of
/// what it refers to.
#[test]
fn a_reference_subsetting_borrows_the_name_too() {
    let ws = resolved(
        "package P {\n\
         \tpart def Wheel;\n\
         \tpart def Car { part hub : Wheel; ref part ::> hub; }\n\
         }\n",
    );
    let hub = named(&ws, "P::Car::hub");
    assert_eq!(
        ws.named_after(hub).len(),
        1,
        "the reference that refers to `hub` answers to its name"
    );
}

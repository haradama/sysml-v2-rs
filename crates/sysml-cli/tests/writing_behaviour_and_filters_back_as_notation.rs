//! `textual::write` on the action statements, accepts, sends, assigns,
//! satisfactions, metadata, dependencies, imports and expressions that a
//! document carries, and on what it must say it did not write.
use serde_json::{json, Value};
use std::collections::HashMap;
use sysml_cli::textual::{write_with, Written};
use sysml_model::ElementId;

fn written(text: &str, kept_text: bool) -> Written {
    written_after(text, kept_text, |_| {})
}

/// The document `export` writes for `text`, changed by `change` the way a
/// document from another tool differs from it, and written back.
fn written_after(text: &str, kept_text: bool, change: impl FnOnce(&mut Vec<Value>)) -> Written {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file("model.sysml", text);
    ws.resolve_all();
    let mut extras = sysml_interchange::Extras::default();
    let ids: Vec<ElementId> = ws.model().ids().collect();
    for id in ids {
        if let Some(target) = ws.imported_alias(id).or_else(|| ws.import_of(id)) {
            extras.import_targets.insert(id, target);
        }
    }
    let document = sysml_interchange::to_json_with(ws.model(), &extras);
    let mut objects: Vec<Value> = serde_json::from_value(document).unwrap();
    change(&mut objects);
    let bytes = serde_json::to_vec(&objects).unwrap();
    let (model, roots, _) = sysml_interchange::read_json(&bytes).unwrap();
    write_with(&model, &roots, &HashMap::new(), kept_text)
}

fn text<'a>(object: &'a Value, key: &str) -> &'a str {
    object[key].as_str().unwrap_or_default()
}

/// The objects of metaclass `ty` named `name`, as indices into `doc`.
fn named(doc: &[Value], ty: &str, name: &str) -> Vec<usize> {
    (0..doc.len())
        .filter(|&at| text(&doc[at], "@type") == ty && text(&doc[at], "name") == name)
        .collect()
}

/// What `owner` owns, of metaclass `ty`, in the order the document lists it.
fn owned(doc: &[Value], owner: usize, ty: &str) -> Vec<usize> {
    let mut ids: Vec<&str> = Vec::new();
    for key in ["ownedRelationship", "ownedElement"] {
        for it in doc[owner][key].as_array().into_iter().flatten() {
            ids.push(it["@id"].as_str().unwrap());
        }
    }
    (0..doc.len())
        .filter(|&at| text(&doc[at], "@type") == ty && ids.contains(&text(&doc[at], "@id")))
        .collect()
}

/// Every object of metaclass `ty`, in document order.
fn all(doc: &[Value], ty: &str) -> Vec<usize> {
    (0..doc.len())
        .filter(|&at| text(&doc[at], "@type") == ty)
        .collect()
}

fn said(w: &Written, line: &str) {
    assert!(w.sysml.contains(line), "no `{line}` in\n{}", w.sysml);
}

const DECLARATIONS: &str = "package P {
    part def Engine { attribute power; }
    part def Car { part engine : Engine[1..5]; part wheels[2..*]; attribute n; }
    attribute def Go;
    item def Fuel;
    metadata def Safety;
    part car : Car {
        metadata tag : Safety about car, car.engine;
        @Safety;
    }
    requirement def R { subject s : Car; }
    requirement req : R;
    part c2 : Car {
        satisfy requirement r1 : R by car;
        not satisfy req by car;
    }
    dependency dep from car to req, c2;
    import Q::*;
    protected import Q::x::**;
    public import all Q::y;
    package Q { part x; part y; }
    import Q::*::**;
    part z[*];
    part m { metadata Safety about car; }
    part sel { attribute items; attribute k = items->select {in i; i > 1}; attribute l = items->collect {in i; i * 2}; }
    part def Qty { attribute n; }
    part items : Qty;
    attribute f = (items).n;
    part w[3];
    part x[2..4];
    part v { attribute m = (1, 2); attribute s = new Car(); attribute t = Car.metadata; }
}
";

const ACTIONS: &str = "package P {
    part def Car { attribute n; }
    attribute def Go;
    item def Fuel;
    part car : Car;
    action def A {
        attribute n : Car;
        in item payload : Fuel;
        then terminate;
        action tx { terminate n; }
        action ae { accept Go via car; }
        action af { accept g : Go via car; }
        action ag { accept g : Go; }
        action ah { accept after 5; }
        action ai { accept g2 at 10; }
        action aj { send new Car() via car to car; }
        action ak { assign car.n := 3; }
        action al { assign n := 3; }
        action am { if n == 1 action thn { action z; } else action els { action y; } }
        action an { while n == 1 action body { action z; } }
        action ao { loop action body { action z; } }
        action aq { if n == 1 { action z; } }
        action ap { for i in (1, 2) action body { action z; } }
    }
    state def S {
        attribute n;
        entry assign n := 1;
        do send Go() to car;
        exit terminate;
        entry action nm accept Go;
        entry action en { action q; }
        do action dd;
        exit action ex;
        state a; state b;
        transition t1 first a accept Go then b;
    }
}
";

#[test]
fn declarations_are_written_back_as_they_were_declared_in_both_modes() {
    for kept in [true, false] {
        let w = written(DECLARATIONS, kept);
        assert!(w.unwritten.is_empty(), "{:?}", w.unwritten);
        said(&w, "part engine : Engine [1..5];");
        said(&w, "part wheels [2..*];");
        said(&w, "@Safety;");
        said(&w, "satisfy requirement r1 : R by car;");
        said(&w, "not satisfy req by car;");
        said(&w, "dependency dep from car to req, c2;");
        said(&w, "protected import Q::x::**;");
        said(&w, "public import all Q::y;");
        said(&w, "private import Q::*::**;");
        said(&w, "part z [*];");
        said(&w, "part w [3];");
        said(&w, "part x [2..4];");
        said(&w, "attribute f = (items).n;");
        said(&w, "metadata Safety about car;");
        said(&w, "attribute s = new Car();");
        said(&w, "attribute t = Car.metadata;");
    }
}

#[test]
fn action_statements_are_written_back_as_they_were_declared_in_both_modes() {
    for kept in [true, false] {
        let w = written(ACTIONS, kept);
        assert!(w.unwritten.is_empty(), "{:?}", w.unwritten);
        said(&w, "terminate n;");
        said(&w, "accept Go via car;");
        said(&w, "accept g : Go via car;");
        said(&w, "accept after 5;");
        said(&w, "accept g2 at 10;");
        said(&w, "send new Car() via car to car;");
        said(&w, "assign n := 3;");
        said(&w, "action thn {");
        said(&w, "action body {");
        said(&w, "for i in (1, 2) action body {");
        said(&w, "entry assign n := 1;");
        said(&w, "do send Go() to car;");
        said(&w, "exit terminate;");
        said(&w, "entry action nm accept Go;");
        said(&w, "entry action en {");
        said(&w, "do action dd;");
        said(&w, "exit action ex;");
        said(&w, "transition t1 first a accept Go then b;");
    }
}

const STEPS: &str = "package P {
    attribute def Go;
    part car { attribute g; part e { part f; } }
    action def A {
        attribute n;
        action a1; action b1; action c1;
        succession s1 first a1 then b1;
        first a1 then c1;
        first car.e then b1;
        first a1 then car.e.f;
        succession s2 first a1 if n > 1 then b1;
        succession first b1 if n > 1 then c1 { doc /* hi */ }
        action p1;
        succession sp first p1 then q1;
        action q1;
        action p2;
        succession sn first p2 if n > 1 then q1;
        action p3;
        succession first p3 if n > 1 then q1 { doc /* said */ }
        action d1 { action q; }
        first c1 then d1;
        first d1 if n > 2 then a1;
        first d1 if n > 2 then car.e;
    }
    state def S {
        state a; state b; state c;
        transition first a then b;
        transition t1 first a accept Go do send Go() then b;
        transition t2 first a accept Go do action ea { action q; } then b;
        transition first a then b;
        transition t4 first a if car.g then b;
        transition first b accept Go via car then c;
        accept Go if car.g then c;
        transition first a accept Go if car.g do send Go() then b;
    }
}
";

#[test]
fn successions_and_transitions_are_written_back_as_they_were_declared_in_both_modes() {
    for kept in [true, false] {
        let w = written(STEPS, kept);
        assert!(w.unwritten.is_empty(), "{:?}", w.unwritten);
        said(&w, "succession s1 first a1 then b1;");
        said(&w, "first car.e then b1;");
        said(&w, "succession sp first p1 then q1;");
        said(&w, "succession sn first p2 if ");
        said(&w, "doc /* said */");
        said(&w, "transition t1 first a accept Go do send Go() then b;");
        said(&w, "transition first a then b;");
        said(&w, "accept Go if car");
        said(&w, "transition first b accept Go via car then c;");
    }
}

/// Every statement here names something no declaration answers to, so a
/// document from a tool that did not resolve it has nothing to write it by.
const UNRESOLVED: &str = "package P {
    attribute def Go;
    part car { attribute g; }
    action def A {
        attribute n;
        action a1; action b1; action c1; action d1;
        action w1 { terminate nowhere; }
        action w2 { accept Go via nowhere; }
        action w3 { send nowhere via car to car; }
        action w4 { assign n := nowhere; }
        action w5 { assign nowhere := 3; }
        action w6 { if nowhere action x { action z; } }
        action w7 { if n == 1 { action z; } else if nowhere { action y; } }
        action w8 { while nowhere { action z; } }
        action w9 { while n == 1 { action z; } until nowhere; }
        action w10 { for i in nowhere { action z; } }
        action w11 { loop { action z; } until nowhere; }
        succession first a1 if nowhere then b1;
        first b1 then nowhere;
        succession first c1 if nowhere then d1;
    }
    state def S {
        state a; state b; state c;
        transition first a accept nowhere then b;
        transition t4 first a if nowhere then b;
        transition t5 first a accept Go do assign n := nowhere then b;
        transition t6 first a accept Go do action ea { assign nowhere := 1; } then c;
        transition t7 first a accept Go if nowhere then b;
        transition first b accept after nowhere then c;
        accept Go if nowhere then c;
        accept Go do action eb { assign nowhere := 1; } then c;
        accept Go via nowhere then c;
    }
    requirement def R;
    part c2 { satisfy nowhere by car; satisfy R by nowhere; satisfy requirement r1 : R by nowhere; }
    dependency dep from car to nowhere;
    dependency dep2 from nowhere to car;
    metadata def Safety;
    part m { metadata Safety about nowhere; @nowhere; }
    part v { attribute x[nowhere]; attribute y[1..nowhere]; part q[nowhere..2]; }
    import nowhere::*;
    import Q::*[nowhere];
    import Q::*[@Safety or nowhere];
    package Q { part x; }
}
";

#[test]
fn what_names_nothing_is_said_not_written_rather_than_guessed_at() {
    let w = written(UNRESOLVED, false);
    for kind in [
        "TerminateActionUsage",
        "AcceptActionUsage",
        "SendActionUsage",
        "AssignmentActionUsage",
        "IfActionUsage",
        "WhileLoopActionUsage",
        "ForLoopActionUsage",
        "TransitionUsage",
        "SatisfyRequirementUsage",
        "MetadataUsage",
        "NamespaceImport",
        "multiplicity",
    ] {
        assert!(
            w.unwritten.iter().any(|(it, _)| it == kind),
            "no {kind} among {:?}",
            w.unwritten
        );
    }
    said(&w, "// not written: TerminateActionUsage");
    said(&w, "// not written: ForLoopActionUsage");
    said(&w, "// not written: NamespaceImport");
    said(&w, "attribute n;");
}

const MACHINE: &str = "package P {
    attribute def Go;
    part car;
    metadata def Safety;
    requirement def R;
    requirement req : R;
    part holder { satisfy req by car; metadata Safety about car; }
    state def S {
        state a; state b; state c;
        transition t1 first a accept Go then b;
        transition t2 first a then c;
        state d;
        transition first d then e;
        state e;
        accept after 5 then c;
    }
    action def A {
        attribute n;
        action b1; action a1;
        first a1 then b1;
        if n > 1 then b1;
        attribute c;
        action l { loop { action z; } until true; }
        action ac { accept Go via car; }
        action sn { send Go() via car to car; }
        action w { while n > 1 { action z; } }
        action f { for i in (1, 2) { action z; } }
        action i { if n == 1 { action z; } }
        action asg { assign n := c; }
    }
}
";

/// The ends of a succession.
fn succession_ends(doc: &[Value], succession: usize) -> Vec<usize> {
    let ids: Vec<&str> = doc[succession]["ownedElement"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["@id"].as_str().unwrap())
        .collect();
    (0..doc.len())
        .filter(|&at| ids.contains(&text(&doc[at], "@id")) && doc[at]["isEnd"] == true)
        .collect()
}

/// The ends of the succession a transition holds.
fn ends(doc: &[Value], transition: usize) -> Vec<usize> {
    succession_ends(doc, owned(doc, transition, "SuccessionAsUsage")[0])
}

#[test]
fn a_transition_whose_succession_is_no_succession_is_not_written() {
    let w = written_after(MACHINE, true, |doc| {
        for transition in all(doc, "TransitionUsage") {
            for succession in owned(doc, transition, "SuccessionAsUsage") {
                doc[succession]["@type"] = json!("BindingConnectorAsUsage");
            }
        }
    });
    assert!(
        w.unwritten
            .iter()
            .filter(|(it, _)| it == "TransitionUsage")
            .count()
            >= 4,
        "{:?}",
        w.unwritten
    );
    said(&w, "// not written: TransitionUsage");
    assert!(!w.sysml.contains("transition t1 first"), "{}", w.sysml);
}

#[test]
fn a_transition_that_does_not_join_two_ends_is_not_written() {
    let w = written_after(MACHINE, true, |doc| {
        for transition in named(doc, "TransitionUsage", "t2") {
            let first = ends(doc, transition)[0];
            doc[first]["isEnd"] = json!(false);
        }
    });
    assert_eq!(
        w.unwritten
            .iter()
            .filter(|(it, at)| it == "TransitionUsage" && at == "t2")
            .count(),
        1,
        "{:?}",
        w.unwritten
    );
    assert!(w.sysml.contains("transition t1 first a accept Go then b;"));
}

#[test]
fn a_transition_with_an_end_it_cannot_name_is_not_written() {
    let w = written_after(MACHINE, true, |doc| {
        for transition in named(doc, "TransitionUsage", "t2") {
            for end in ends(doc, transition) {
                for sub in owned(doc, end, "ReferenceSubsetting") {
                    doc[sub]["@type"] = json!("Dependency");
                }
            }
        }
    });
    assert_eq!(
        w.unwritten,
        vec![("TransitionUsage".to_string(), "t2".to_string())]
    );
}

#[test]
fn what_a_succession_runs_to_must_be_something_that_can_be_named() {
    for kept in [true, false] {
        let w = written_after(MACHINE, kept, |doc| {
            // the end of every succession that says what it runs to now
            // refers to the end itself, which has no name
            for succession in all(doc, "SuccessionAsUsage") {
                let Some(&to) = succession_ends(doc, succession).last() else {
                    continue;
                };
                let id = doc[to]["@id"].clone();
                for sub in owned(doc, to, "ReferenceSubsetting") {
                    doc[sub]["referencedFeature"] = json!({ "@id": id });
                }
            }
        });
        for at in [("SuccessionAsUsage", "A"), ("TransitionUsage", "A")] {
            assert!(
                w.unwritten.contains(&(at.0.to_string(), at.1.to_string())),
                "{at:?} in {:?}",
                w.unwritten
            );
        }
        said(&w, "// not written: SuccessionAsUsage");
    }
}

#[test]
fn a_trigger_that_is_no_trigger_is_not_written() {
    let w = written_after(MACHINE, true, |doc| {
        for it in all(doc, "TriggerInvocationExpression") {
            doc[it]["@type"] = json!("LiteralInteger");
        }
    });
    assert_eq!(
        w.unwritten,
        vec![("TransitionUsage".to_string(), "S".to_string())]
    );
    said(&w, "transition t1 first a accept Go then b;");
    assert!(!w.sysml.contains("accept after 5"), "{}", w.sysml);
}

#[test]
fn a_dependency_is_written_by_its_clients_and_suppliers_or_not_at_all() {
    let src = "package P { part a; part b; dependency d from a to b; }";
    let w = written_after(src, true, |doc| {
        for it in all(doc, "Dependency") {
            doc[it].as_object_mut().unwrap().remove("client");
        }
    });
    assert_eq!(
        w.unwritten,
        vec![("Dependency".to_string(), "d".to_string())]
    );
    said(&w, "// not written: Dependency");
    // a supplier said as one rather than as a list is the one supplier
    let w = written_after(src, true, |doc| {
        for it in all(doc, "Dependency") {
            let one = doc[it]["supplier"][0].clone();
            doc[it]["supplier"] = one;
        }
    });
    assert!(w.unwritten.is_empty(), "{:?}", w.unwritten);
    said(&w, "dependency d from a to b;");
}

const FILTERED: &str = "package P { package Q { part x; } import Q::*; part v { attribute m = 1 + 2; } view def Vw; view vw : Vw { expose Q::*[@Safety]; } metadata def Safety; }";

#[test]
fn an_import_that_says_nothing_of_its_visibility_is_written_without_one() {
    let w = written_after(FILTERED, true, |doc| {
        for it in all(doc, "NamespaceImport") {
            doc[it]["visibility"] = Value::Null;
        }
        // a filter that the membership holds rather than names
        for it in all(doc, "ElementFilterMembership") {
            doc[it].as_object_mut().unwrap().remove("condition");
        }
    });
    assert!(w.unwritten.is_empty(), "{:?}", w.unwritten);
    said(&w, "\n    import Q::*;\n");
    said(&w, "expose Q::*[@Safety];");
}

#[test]
fn text_kept_in_a_language_that_is_not_the_notation_is_not_used() {
    let w = written_after(FILTERED, true, |doc| {
        for it in all(doc, "TextualRepresentation") {
            doc[it]["language"] = json!("ocl");
        }
    });
    // the expression is written from what it is, and the exposure that
    // has no other way of being said is left out
    said(&w, "attribute m = (1 + 2);");
    assert!(
        w.unwritten
            .contains(&("NamespaceExpose".to_string(), "vw".to_string())),
        "{:?}",
        w.unwritten
    );
}

#[test]
fn statements_with_the_parts_they_need_taken_away_are_not_written() {
    let w = written_after(MACHINE, true, |doc| {
        // no parameters: neither a condition nor a body to write
        for ty in [
            "IfActionUsage",
            "WhileLoopActionUsage",
            "ForLoopActionUsage",
        ] {
            for it in all(doc, ty) {
                for at in owned_any(doc, it) {
                    doc[at]["direction"] = Value::Null;
                }
            }
        }
        // and an assignment that assigns to nothing
        for it in all(doc, "AssignmentActionUsage") {
            for at in owned(doc, it, "Membership") {
                doc[at]["@type"] = json!("OwningMembership");
            }
        }
    });
    for at in [
        ("WhileLoopActionUsage", "l"),
        ("WhileLoopActionUsage", "w"),
        ("ForLoopActionUsage", "f"),
        ("IfActionUsage", "i"),
        ("AssignmentActionUsage", "asg"),
    ] {
        assert!(
            w.unwritten.contains(&(at.0.to_string(), at.1.to_string())),
            "{at:?} in {:?}",
            w.unwritten
        );
    }
}

/// What `owner` owns by `ownedElement`, whatever its metaclass.
fn owned_any(doc: &[Value], owner: usize) -> Vec<usize> {
    let ids: Vec<&str> = doc[owner]["ownedElement"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|it| it["@id"].as_str().unwrap())
        .collect();
    (0..doc.len())
        .filter(|&at| ids.contains(&text(&doc[at], "@id")))
        .collect()
}

/// The parameters of an action statement, which are what it owns that has
/// a direction.
fn parameters(doc: &[Value], action: usize) -> Vec<usize> {
    owned_any(doc, action)
        .into_iter()
        .filter(|&at| doc[at]["direction"].is_string())
        .collect()
}

/// Hand `param` the value `value`, as a `FeatureValue` of its own the way
/// the document writes every other one.
fn give_value(doc: &mut Vec<Value>, param: usize, value: &Value) {
    let mut given = doc[all(doc, "FeatureValue")[0]].clone();
    let id = format!("{}-given", text(&doc[param], "@id"));
    let param_id = doc[param]["@id"].clone();
    given["@id"] = json!(id);
    given["elementId"] = json!(id);
    given["featureWithValue"] = json!({ "@id": param_id });
    given["membershipOwningNamespace"] = json!({ "@id": param_id });
    given["owningRelatedElement"] = json!({ "@id": param_id });
    given["source"] = json!([{ "@id": param_id }]);
    given["relatedElement"] = json!([{ "@id": param_id }, value]);
    for key in ["value", "memberElement", "ownedMemberElement"] {
        given[key] = value.clone();
    }
    given["target"] = json!([value]);
    given["ownedRelatedElement"] = json!([]);
    doc[param]["ownedRelationship"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "@id": id }));
    doc.push(given);
}

#[test]
fn a_statement_missing_what_it_is_said_with_is_not_written() {
    let w = written_after(MACHINE, true, |doc| {
        // the receiver `accept` waits through has no value
        for it in all(doc, "AcceptActionUsage") {
            for p in parameters(doc, it).into_iter().skip(1) {
                for fv in owned(doc, p, "FeatureValue") {
                    doc[fv]["@type"] = json!("OwningMembership");
                }
            }
        }
        // nothing is sent, and nothing says what is
        for it in all(doc, "SendActionUsage") {
            for p in parameters(doc, it) {
                for fv in owned(doc, p, "FeatureValue") {
                    doc[fv]["@type"] = json!("OwningMembership");
                }
            }
        }
        // what is assigned to is not said
        for it in all(doc, "AssignmentActionUsage") {
            for m in owned(doc, it, "Membership") {
                doc[m].as_object_mut().unwrap().remove("memberElement");
            }
        }
        // what is satisfied is not said
        for it in all(doc, "SatisfyRequirementUsage") {
            doc[it]
                .as_object_mut()
                .unwrap()
                .remove("satisfiedRequirement");
            doc[it].as_object_mut().unwrap().remove("satisfyingFeature");
        }
        // what the metadata is about is nothing that has a name
        for it in all(doc, "MetadataUsage") {
            for a in owned(doc, it, "Annotation") {
                let id = doc[a]["@id"].clone();
                doc[a]["annotatedElement"] = json!({ "@id": id });
            }
        }
    });
    for (kind, at) in [
        ("SendActionUsage", "sn"),
        ("AssignmentActionUsage", "asg"),
        ("SatisfyRequirementUsage", "holder"),
        ("MetadataUsage", "holder"),
    ] {
        assert!(
            w.unwritten.contains(&(kind.to_string(), at.to_string())),
            "{kind} in {at}: {:?}",
            w.unwritten
        );
    }
    // an `accept` that waits through nothing is the `accept` of what it
    // waits for
    said(&w, "action ac {\n            accept Go;\n        }");
}

#[test]
fn a_transition_triggered_by_a_list_of_one_or_by_nothing_is_still_written() {
    for trigger in [json!(null), json!(true)] {
        let w = written_after(MACHINE, true, |doc| {
            for it in all(doc, "TransitionUsage") {
                let was = doc[it]["triggerAction"].clone();
                doc[it]["triggerAction"] = match &trigger {
                    // the one trigger there is, said as one rather than as a list
                    Value::Bool(_) => was.get(0).cloned().unwrap_or(Value::Null),
                    nothing => nothing.clone(),
                };
                let guards = doc[it]["guardExpression"].clone();
                doc[it]["guardExpression"] = match &trigger {
                    Value::Bool(_) => guards.get(0).cloned().unwrap_or(Value::Null),
                    nothing => nothing.clone(),
                };
                let effects = doc[it]["effectAction"].clone();
                doc[it]["effectAction"] = match &trigger {
                    Value::Bool(_) => effects.get(0).cloned().unwrap_or(Value::Null),
                    nothing => nothing.clone(),
                };
            }
        });
        assert!(
            w.sysml.contains("transition first d then e;"),
            "{}",
            w.sysml
        );
        if trigger == json!(true) {
            said(&w, "transition t1 first a accept Go then b;");
            said(&w, "accept after 5 then c;");
        }
    }
}

#[test]
fn a_multiplicity_is_read_from_whichever_bounds_the_document_keeps() {
    let src = "package P { part w[3]; part x[2..4]; }";
    let bounds = |doc: &[Value], at: usize| -> Vec<Value> {
        owned_any(doc, at)
            .into_iter()
            .map(|b| doc[b]["@id"].clone())
            .collect()
    };
    // one bound, as the upper one
    let w = written_after(src, true, |doc| {
        for it in all(doc, "MultiplicityRange") {
            if let [one] = bounds(doc, it).as_slice() {
                doc[it]["upperBound"] = json!({ "@id": one });
            }
        }
    });
    said(&w, "part w [3];");
    // the bounds kept as a list
    let w = written_after(src, true, |doc| {
        for it in all(doc, "MultiplicityRange") {
            if let [one] = bounds(doc, it).as_slice() {
                doc[it]["bound"] = json!([{ "@id": one }]);
            }
        }
    });
    said(&w, "part w [3];");
    // no bounds kept at all, which leaves what the range owns
    let w = written_after(src, true, |doc| {
        for it in all(doc, "MultiplicityRange") {
            for key in ["bound", "lowerBound", "upperBound"] {
                doc[it][key] = Value::Null;
            }
        }
    });
    said(&w, "part w [3];");
    said(&w, "part x [2..4];");
    // and a range that owns none says nothing of its bounds
    let w = written_after(src, true, |doc| {
        for it in all(doc, "MultiplicityRange") {
            for key in ["bound", "lowerBound", "upperBound"] {
                doc[it][key] = Value::Null;
            }
            doc[it]["ownedRelationship"] = json!([]);
            doc[it]["ownedElement"] = json!([]);
        }
    });
    assert!(
        w.unwritten.iter().any(|(it, _)| it == "multiplicity"),
        "{:?}",
        w.unwritten
    );
}

#[test]
fn an_assignment_is_written_through_what_its_first_parameter_comes_to() {
    let src = "package P { part def Q { attribute n; } action def A { attribute c : Q; attribute d : Q; action asg { assign c.n := d; } } }";
    let through = |value_of: fn(&[Value], usize) -> Value| {
        written_after(src, true, |doc| {
            let assign = all(doc, "AssignmentActionUsage")[0];
            let parameters = parameters(doc, assign);
            let value = value_of(doc, owned(doc, parameters[1], "FeatureValue")[0]);
            give_value(doc, parameters[0], &value);
        })
    };
    // the value it comes to is one the notation can write
    let w = through(|doc, fv| doc[fv]["value"].clone());
    said(&w, "assign d.n := d;");
    // and one it cannot is not guessed at
    let w =
        through(|doc, _| json!({ "@id": text(&doc[all(doc, "AssignmentActionUsage")[0]], "@id") }));
    assert!(
        w.unwritten
            .contains(&("AssignmentActionUsage".to_string(), "asg".to_string())),
        "{:?}",
        w.unwritten
    );
    // what is assigned to is no feature that has a name
    let w = written_after(src, true, |doc| {
        let assign = all(doc, "AssignmentActionUsage")[0];
        let id = doc[assign]["@id"].clone();
        for m in owned(doc, assign, "Membership") {
            doc[m]["memberElement"] = json!({ "@id": id });
        }
    });
    assert!(
        w.unwritten
            .contains(&("AssignmentActionUsage".to_string(), "asg".to_string())),
        "{:?}",
        w.unwritten
    );
}

#[test]
fn a_loop_whose_condition_is_a_value_is_written_with_that_condition() {
    let src = "package P { action def A { attribute n; action l { loop { action z; } until true; } action w { while n > 1 { action z; } until n < 3; } } }";
    let w = written_after(src, true, |doc| {
        let looping = all(doc, "WhileLoopActionUsage");
        let first = parameters(doc, looping[0])[0];
        let value = doc[all(doc, "LiteralBoolean")[0]]["@id"].clone();
        give_value(doc, first, &json!({ "@id": value }));
    });
    said(&w, "while true {");
    // a parameter after the body that is neither the condition nor an
    // action is more than a loop says
    let w = written_after(src, true, |doc| {
        for looping in all(doc, "WhileLoopActionUsage") {
            let params = parameters(doc, looping);
            let body = params
                .iter()
                .position(|&p| text(&doc[p], "@type") == "ActionUsage");
            if let Some(body) = body {
                for &p in &params[body + 1..] {
                    doc[p]["@type"] = json!("ReferenceUsage");
                }
            }
        }
    });
    assert!(
        w.unwritten
            .iter()
            .any(|(it, _)| it == "WhileLoopActionUsage"),
        "{:?}",
        w.unwritten
    );
}

#[test]
fn a_filter_that_filters_nothing_is_not_written() {
    let src = "package P { package Q { part x; } metadata def Safety; import Q::*[@Safety]; }";
    let w = written_after(src, true, |doc| {
        for it in all(doc, "ElementFilterMembership") {
            doc[it].as_object_mut().unwrap().remove("condition");
            doc[it]["ownedRelatedElement"] = json!([]);
            doc[it]["ownedRelationship"] = json!([]);
            doc[it]["ownedElement"] = json!([]);
        }
    });
    assert!(
        w.unwritten.iter().any(|(it, _)| it == "NamespaceImport"),
        "{:?}",
        w.unwritten
    );
}

#[test]
fn metadata_that_is_typed_by_nothing_is_not_written() {
    let w = written_after(MACHINE, true, |doc| {
        for it in all(doc, "MetadataUsage") {
            for typing in owned(doc, it, "FeatureTyping") {
                doc[typing]["type"] = Value::Null;
            }
        }
    });
    assert!(
        w.unwritten
            .contains(&("MetadataUsage".to_string(), "holder".to_string())),
        "{:?}",
        w.unwritten
    );
    said(&w, "// not written: MetadataUsage");
}

//! Interchange documents written back as the notation, in their less
//! usual corners: roles, modifiers, relationships declared on their own,
//! KerML's connectors -- and the documents that say less than they should,
//! which are left out rather than guessed at.
use std::collections::HashMap;

use serde_json::{json, Value};
use sysml_cli::textual::{write_with, Written};
use sysml_model::ElementId;

/// The document `sysml export` writes for `text`, as JSON elements.
fn document(name: &str, text: &str) -> Vec<Value> {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file(name, text);
    ws.resolve_all();
    let mut extras = sysml_interchange::Extras::default();
    let ids: Vec<ElementId> = ws.model().ids().collect();
    for id in ids {
        if let Some(target) = ws.imported_alias(id).or_else(|| ws.import_of(id)) {
            extras.import_targets.insert(id, target);
        }
    }
    match sysml_interchange::to_json_with(ws.model(), &extras) {
        Value::Array(all) => all,
        other => panic!("a document is an array, not {other}"),
    }
}

/// A document, read the way `import_json` reads one, and written.
fn written_from(doc: &[Value], kept_text: bool) -> Written {
    let bytes = serde_json::to_vec(doc).unwrap();
    let (model, roots, _) = sysml_interchange::read_json(&bytes).unwrap();
    write_with(&model, &roots, &HashMap::new(), kept_text)
}

fn written(name: &str, text: &str) -> String {
    written_from(&document(name, text), true).sysml
}

/// `text`, exported, changed by `edit` the way a document from another
/// tool might differ, and written.
fn edited(name: &str, text: &str, edit: impl FnOnce(&mut Vec<Value>)) -> Written {
    let mut doc = document(name, text);
    edit(&mut doc);
    written_from(&doc, true)
}

/// Each of `said` is a line of `sysml`, as written.
fn says(sysml: &str, said: &[&str]) {
    for line in said {
        assert!(
            sysml.lines().any(|it| it.trim() == *line),
            "no line `{line}` in\n{sysml}"
        );
    }
}

/// The one element of `kind` named `name`.
fn named(doc: &[Value], kind: &str, name: &str) -> usize {
    let at: Vec<usize> = (0..doc.len())
        .filter(|&i| doc[i]["@type"] == kind && doc[i]["declaredName"] == name)
        .collect();
    assert_eq!(at.len(), 1, "{kind} {name} among {}", doc.len());
    at[0]
}

/// The element that has `id`.
fn by_id(doc: &[Value], id: &Value) -> usize {
    let id = id.get("@id").unwrap_or(id);
    doc.iter().position(|it| &it["@id"] == id).unwrap()
}

fn rm(d: &mut [Value], i: usize, prop: &str) {
    d[i].as_object_mut().unwrap().remove(prop);
}

/// The first element of `kind`.
fn first(d: &[Value], kind: &str) -> usize {
    d.iter()
        .position(|it| it["@type"] == kind)
        .unwrap_or_else(|| {
            let kinds: std::collections::BTreeSet<_> =
                d.iter().map(|it| it["@type"].to_string()).collect();
            panic!("no {kind} among {kinds:?}")
        })
}

/// What a requirement, a use case, a verification and a view say of the
/// things that play a part in them: each role is a word of its own.
#[test]
fn what_plays_a_part_in_a_requirement_is_written_with_the_word_for_its_part() {
    let sysml = written(
        "roles.sysml",
        "package P {\n\
         \tpart def A;\n\
         \trequirement def R {\n\
         \t\tsubject s : A;\n\
         \t\tactor a1 : A;\n\
         \t\tstakeholder sh : A;\n\
         \t\tobjective;\n\
         \t\tassume constraint ac { true }\n\
         \t\trequire constraint rc { true }\n\
         \t}\n\
         \tuse case def U { objective { doc /* o */ } actor u : A; }\n\
         \tverification def Ver { verify R1; subject sub : A; }\n\
         \trequirement R1 : R;\n\
         \tanalysis def An { frame R1; return r : A; }\n\
         \tview def Vw { render rr; }\n\
         \tvariation part def Vv { variant part a; variant part b; }\n\
         }\n",
    );
    says(
        &sysml,
        &[
            "subject s : A;",
            "actor part a1 : A;",
            "stakeholder part sh : A;",
            "objective requirement;",
            "assume constraint ac {",
            "require constraint rc {",
            "actor part u : A;",
            "subject sub : A;",
            "requirement R1 : R;",
            "frame P::R1;",
            "return r : A;",
            "variation part def Vv {",
            "variant part a;",
            "variant part b;",
        ],
    );
}

/// A comment says what it is about, and what it is called; a
/// representation says the language it is in.
#[test]
fn a_comment_is_written_with_its_name_what_it_is_about_and_its_locale() {
    let sysml = written(
        "notes.sysml",
        "package P {\n\
         \tpart def A;\n\
         \tpart def B;\n\
         \tcomment <c1> Note about A, B locale \"en\" /* hello */\n\
         \trep <r1> ocl language \"ocl\" /* x > 1 */\n\
         \tdoc <d1> /* doc */\n\
         \talias Al for A;\n\
         \talias <'al2'> Al2 for B;\n\
         }\n",
    );
    says(
        &sysml,
        &[
            "comment <c1> Note about A, B locale \"en\" /* hello */",
            "rep <r1> ocl language \"ocl\" /* x > 1 */",
            "doc <d1> /* doc */",
            "alias Al for A;",
            "alias Al2 for B;",
        ],
    );
}

/// The prefixes of a usage: the words in front of the keyword, in the
/// order the notation takes them.
#[test]
fn the_words_in_front_of_a_usage_are_written_in_the_order_the_notation_takes_them() {
    let sysml = written(
        "prefixes.sysml",
        "package P {\n\
         \tpart def A;\n\
         \tpart def C { attribute x; attribute y; }\n\
         \tabstract part def Abs;\n\
         \tpart def Eng { part ee; }\n\
         \tconnection def Cn { end [1] part x : A; end [0..1] part y : A; end z : A; }\n\
         \tpart c1 : C {\n\
         \t\tref :>> x;\n\
         \t\t:>> y = 3;\n\
         \t\tderived attribute d = 1;\n\
         \t\tend part e : A;\n\
         \t\tend [0..1] part e2 : A;\n\
         \t\tindividual part ind;\n\
         \t\tsnapshot part snap : A;\n\
         \t\ttimeslice part ts : A;\n\
         \t\ttimeslice tt;\n\
         \t\tabstract attribute abstractly;\n\
         \t}\n\
         \tpart def Mem { ref item it; abstract part ap; }\n\
         \tenum def En { enum a; enum b; }\n\
         }\n",
    );
    says(
        &sysml,
        &[
            "abstract part def Abs;",
            ":>> C::x;",
            ":>> C::y = 3;",
            "end part e : A;",
            "end [0..1] part e2 : A;",
            "individual part ind;",
            "snapshot part snap : A;",
            "timeslice part ts : A;",
            "timeslice tt;",
            "abstract attribute abstractly;",
            "end [1] part x : A;",
            "end [0..1] part y : A;",
            "ref item it;",
            "abstract part ap;",
            "enum a;",
            "enum b;",
        ],
    );
}

/// What an action body says: its steps and what follows them, and the
/// ways to stop.
#[test]
fn the_steps_of_an_action_and_what_ends_it_are_written_as_the_notation_has_them() {
    let sysml = written(
        "steps.sysml",
        "package P {\n\
         \taction def Ac {\n\
         \t\taction a1;\n\
         \t\taction a2;\n\
         \t\tfirst a1;\n\
         \t\tthen a2;\n\
         \t\tterminate;\n\
         \t\tterminate a1;\n\
         \t\tentry action en;\n\
         \t\tdo action dd;\n\
         \t\texit action ex;\n\
         \t\tstate st parallel { state s1; state s2; }\n\
         \t\tconstraint k { true }\n\
         \t\tassert not constraint nc { false }\n\
         \t\tassert not k;\n\
         \t}\n\
         \tstate def St { entry; do; exit; entry action e1; }\n\
         }\n",
    );
    says(
        &sysml,
        &[
            "first a1;",
            "then a2;",
            "terminate;",
            "terminate a1;",
            "entry action en;",
            "do action dd;",
            "exit action ex;",
            "state st parallel {",
            "assert not constraint nc {",
            "assert not k;",
            "entry;",
            "do;",
            "exit;",
            "entry action e1;",
        ],
    );
}

/// A metadata definition is applied as the prefix of what it is applied
/// to, and written as such where the declaration has no kind of its own.
#[test]
fn metadata_applied_to_a_definition_of_no_kind_is_written_in_front_of_it() {
    let sysml = written(
        "prefixed.sysml",
        "package P {\n\tmetadata def M;\n\t#M def Q;\n\t#M def Q2 { part x; }\n\t#M part z;\n}\n",
    );
    says(&sysml, &["#M def Q;", "#M def Q2 {", "part x;"]);
}

/// The relationships between features, each written as it is in KerML.
#[test]
fn a_kerml_feature_says_how_it_relates_to_the_others() {
    let sysml = written(
        "relations.kerml",
        "package K {\n\
         \tclass A;\n\
         \tclass B;\n\
         \tclass all Al;\n\
         \tabstract class Abs;\n\
         \tclass U unions A, B;\n\
         \tclass I intersects A, B;\n\
         \tclass Df differences A, B;\n\
         \tclassifier Z conjugates A;\n\
         \tfeature f : A;\n\
         \tfeature g : A;\n\
         \tfeature h : A ordered nonunique [1..*];\n\
         \tfeature i crosses f;\n\
         \tfeature j subsets f, g;\n\
         \tfeature k redefines f;\n\
         \tfeature l references f;\n\
         \tfeature m featured by A, Abs;\n\
         \tfeature n inverse of f;\n\
         \tfeature o disjoint from f;\n\
         \tfeature fv = 3;\n\
         \tfeature fd default = 3;\n\
         \tfeature fi := 3;\n\
         \tfeature fdi default := 3;\n\
         \tderived feature d : A;\n\
         \tportion feature pf : A;\n\
         \tvar feature vf : A;\n\
         \tconst feature kf : A;\n\
         }\n\
         class C { member feature m; composite feature c; feature cc : C; }\n",
    );
    says(
        &sysml,
        &[
            "class all Al;",
            "abstract class Abs;",
            "class U unions A, B;",
            "class I intersects A, B;",
            "class Df differences A, B;",
            "classifier Z ~ A;",
            "feature h : A [1..*] ordered nonunique;",
            "feature i crosses f;",
            "feature j :> f, g;",
            "feature k :>> f;",
            "feature l ::> f;",
            "feature m featured by A, Abs;",
            "feature n inverse of f;",
            "feature o disjoint from f;",
            "feature fv = 3;",
            "feature fd default = 3;",
            "var feature fi := 3;",
            "var feature fdi default := 3;",
            "derived feature d : A;",
            "portion feature pf : A;",
            "var feature vf : A;",
            "const feature kf : A;",
            "member feature m;",
            "composite feature c;",
        ],
    );
}

/// A relationship that is a member of a namespace on its own, rather
/// than said by one of the two things it relates.
#[test]
fn a_relationship_between_two_other_things_is_written_as_a_statement_of_its_own() {
    let sysml = written(
        "statements.kerml",
        "package K {\n\
         \tclass A;\n\
         \tclass B;\n\
         \tfeature f : A;\n\
         \tfeature g : A;\n\
         \tspecialization sp subtype A specializes B;\n\
         \tdisjoining dj disjoint A from B;\n\
         \tdisjoint A from B;\n\
         \tfeaturing ft of g by A;\n\
         \tfeaturing of f by B;\n\
         }\n",
    );
    says(
        &sysml,
        &[
            "specialization sp subtype A specializes B;",
            "disjoining dj disjoint A from B;",
            "disjoint A from B;",
            "featuring ft of g by A;",
        ],
    );
}

/// A connector of KerML names its ends after its keyword, and the word in
/// front of them is there only when something is declared before them.
#[test]
fn a_kerml_connector_is_written_with_its_ends_after_the_word_the_declaration_wants() {
    let sysml = written(
        "connectors.kerml",
        "class C {\n\
         \tfeature f;\n\
         \tfeature g;\n\
         \tfeature h;\n\
         \tconnector (f, g, h);\n\
         \tconnector c1 : C from f to g;\n\
         \tconnector from f to g;\n\
         \tbinding f = g;\n\
         \tbinding bd of f = g;\n\
         \tsuccession first f then g;\n\
         \tabstract connector ac from f to g;\n\
         \tconnector <'s'> nm from f to g;\n\
         \tstep a { feature x; }\n\
         \tstep b { feature y; }\n\
         \tconnector from a.x to b.y;\n\
         \tbinding a.x = b.y;\n\
         \tflow from a.x to b.y;\n\
         \tflow fl of C from a.x to b.y;\n\
         }\n",
    );
    says(
        &sysml,
        &[
            "connector (f, g, h);",
            "connector c1 : C from f to g;",
            "connector f to g;",
            "binding f = g;",
            "binding bd of f = g;",
            "succession first f then g;",
            "abstract connector ac from f to g;",
            "connector <s> nm from f to g;",
        ],
    );
}

/// A connection whose ends are all named has them in its body, since the
/// clause cannot say them, and one of no ends is a declaration like any.
#[test]
fn a_connection_that_names_three_ends_has_them_in_its_body() {
    let sysml = written(
        "ternary.sysml",
        "package P {\n\
         \tconnection def Cn;\n\
         \tpart def A;\n\
         \tpart a : A;\n\
         \tpart b : A;\n\
         \tpart c : A;\n\
         \tconnection k : Cn;\n\
         \tconnection n connect (u ::> a, v ::> b, w ::> c);\n\
         }\n",
    );
    says(
        &sysml,
        &[
            "connection k : Cn;",
            "connection n {",
            "end ref u ::> a [1..1];",
            "end ref w ::> c [1..1];",
        ],
    );
}

/// What a flow carries is said after `of`, with the name it is given.
#[test]
fn what_a_flow_carries_is_written_after_of_with_the_name_it_is_given() {
    let sysml = written(
        "carries.sysml",
        "package P {\n\
         \tpart def A { port p; port q; }\n\
         \tpart def B;\n\
         \tpart def S {\n\
         \t\tpart a : A;\n\
         \t\tpart b : A;\n\
         \t\tflow f of x : B from a.p to b.q;\n\
         \t\tflow of y : B from a.p to b.q;\n\
         \t\tflow from a.p to b.q;\n\
         \t\tmessage m of B from a.p to b.q;\n\
         \t}\n\
         }\n",
    );
    says(
        &sysml,
        &[
            "flow f of x : B from a.p to b.q;",
            "flow of y : B from a.p to b.q;",
            "flow from a.p to b.q;",
            "message m of B from a.p to b.q;",
        ],
    );
}

/// What a document from a server leaves out is left out of the notation,
/// and said to be.
#[test]
fn a_comment_about_what_has_no_name_is_not_written() {
    let w = edited(
        "m.sysml",
        "package P { part def A; comment about A /* c */ }",
        |d| {
            let a = named(d, "PartDefinition", "A");
            rm(d, a, "declaredName");
            rm(d, a, "name");
        },
    );
    says(&w.sysml, &["// not written: Comment"]);
    assert_eq!(w.unwritten, [("Comment".to_string(), "P".to_string())]);
}

#[test]
fn a_filter_that_does_not_name_its_condition_has_the_expression_it_owns() {
    let w = edited("m.sysml", "package P { part def A; filter true; }", |d| {
        let i = first(d, "ElementFilterMembership");
        rm(d, i, "condition");
    });
    says(&w.sysml, &["filter true;"]);
    assert!(w.unwritten.is_empty());
}

#[test]
fn a_filter_whose_condition_is_no_expression_is_not_written() {
    let w = edited("m.sysml", "package P { part def A; filter true; }", |d| {
        let i = first(d, "LiteralBoolean");
        d[i]["@type"] = json!("PartDefinition");
    });
    says(&w.sysml, &["// not written: ElementFilterMembership"]);
    assert_eq!(
        w.unwritten,
        [("ElementFilterMembership".to_string(), "P".to_string())]
    );
}

#[test]
fn a_specialization_with_one_side_missing_is_not_written() {
    let w = edited(
        "m.kerml",
        "package P { class A; class B; specialization s subtype A specializes B; }",
        |d| {
            let i = first(d, "Specialization");
            rm(d, i, "general");
        },
    );
    says(&w.sysml, &["// not written: Specialization"]);
    assert_eq!(
        w.unwritten,
        [("Specialization".to_string(), "s".to_string())]
    );
}

#[test]
fn a_first_step_that_names_nothing_and_an_alias_of_nothing_are_not_written() {
    let w = edited(
        "m.sysml",
        "package P { action def X { action a; first a; then b; action b; } part def A; alias Al for A; }",
        |d| {
            let i = d
                .iter()
                .position(|it| it["@type"] == "Membership" && it["memberName"].is_null())
                .unwrap();
            rm(d, i, "memberElement");
            let i = d
                .iter()
                .position(|it| it["@type"] == "Membership" && it["memberName"] == "Al")
                .unwrap();
            rm(d, i, "memberElement");
        },
    );
    assert_eq!(
        w.unwritten,
        [
            ("Membership".to_string(), "X".to_string()),
            ("Membership".to_string(), "Al".to_string())
        ]
    );
    assert_eq!(w.sysml.matches("// not written: Membership").count(), 2);
}

#[test]
fn a_subclassification_that_names_no_subclassifier_still_says_what_is_specialized() {
    let w = edited(
        "m.sysml",
        "package P { part def A; part def B :> A; }",
        |d| {
            let i = first(d, "Subclassification");
            rm(d, i, "subclassifier");
        },
    );
    says(&w.sysml, &["part def B :> A;"]);
}

#[test]
fn a_typing_that_names_no_type_says_nothing_of_one() {
    let w = edited("m.sysml", "package P { part def A; part x : A; }", |d| {
        let i = first(d, "FeatureTyping");
        rm(d, i, "type");
    });
    says(&w.sysml, &["part x;"]);
}

#[test]
fn a_value_that_cannot_be_written_is_said_to_be_left_out() {
    let w = edited("m.sysml", "package P { attribute x = 3; }", |d| {
        let i = first(d, "LiteralInteger");
        d[i]["@type"] = json!("PartDefinition");
    });
    says(&w.sysml, &["attribute x;"]);
    assert_eq!(w.unwritten, [("expression".to_string(), "x".to_string())]);
}

#[test]
fn a_message_that_is_an_end_says_so() {
    let w = edited(
        "m.sysml",
        "package P { part def A; part def S { message m of A from a to b; } }",
        |d| {
            let i = first(d, "FlowUsage");
            d[i]["isEnd"] = json!(true);
        },
    );
    says(&w.sysml, &["end message m of A;"]);
}

#[test]
fn what_a_performance_refers_to_is_written_once_or_not_as_the_shorthand() {
    let text = "package P { action def T; part def A { action take : T; perform take; } }";
    says(&written("m.sysml", text), &["perform take;"]);
    // with its reference gone, there is nothing for the shorthand to say
    let w = edited("m.sysml", text, |d| {
        let i = d
            .iter()
            .position(|it| {
                it["@type"] == "ReferenceSubsetting" && !it["isImplied"].as_bool().unwrap_or(false)
            })
            .unwrap();
        rm(d, i, "referencedFeature");
    });
    says(&w.sysml, &["perform action;"]);
    // and with two, it cannot say which
    let w = edited("m.sysml", text, |d| {
        let i = d
            .iter()
            .position(|it| {
                it["@type"] == "ReferenceSubsetting" && !it["isImplied"].as_bool().unwrap_or(false)
            })
            .unwrap();
        let mut copy = d[i].clone();
        let id = "00000000-0000-0000-0000-00000000dead";
        copy["@id"] = json!(id);
        copy["elementId"] = json!(id);
        let owner = by_id(d, &copy["owningRelatedElement"]);
        d[owner]["ownedRelationship"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "@id": id }));
        d.push(copy);
    });
    says(&w.sysml, &["perform action ::> take, take;"]);
}

#[test]
fn a_comment_without_the_annotation_that_says_what_it_is_about_is_written_without_it() {
    let w = edited(
        "m.sysml",
        "package P { part def A; comment about A /* c */ }",
        |d| {
            let i = first(d, "Annotation");
            rm(d, i, "annotatedElement");
        },
    );
    says(&w.sysml, &["comment /* c */"]);
}

#[test]
fn a_usage_that_is_the_whole_of_a_document_is_written_as_it_is() {
    let doc = vec![
        json!({"@type": "PartUsage", "@id": "00000000-0000-0000-0000-000000000001", "declaredName": "x"}),
    ];
    let w = written_from(&doc, true);
    says(&w.sysml, &["part x;"]);
}

#[test]
fn what_the_document_only_refers_to_is_named_where_it_is_used_and_declared_nowhere() {
    let doc = document("m.sysml", "package P { part def A; }");
    let bytes = serde_json::to_vec(&doc).unwrap();
    let (model, roots, _) = sysml_interchange::read_json(&bytes).unwrap();
    let outside: HashMap<ElementId, String> = roots
        .iter()
        .map(|&it| (it, "Elsewhere".to_string()))
        .collect();
    let w = write_with(&model, &roots, &outside, true);
    assert_eq!(w.sysml, "");
}

/// The element of `kind` that `owner` owns through `link`.
fn owned_by(doc: &[Value], kind: &str, link: &str, owner: usize) -> usize {
    let id = &doc[owner]["@id"];
    doc.iter()
        .position(|it| {
            it["@type"] == kind
                && &it[link]["@id"] == id
                && !it["isImplied"].as_bool().unwrap_or(false)
        })
        .unwrap_or_else(|| panic!("no {kind} owned through {link}"))
}

const FLOW: &str = "package P {\n\
    \tpart def A { port p; port q; }\n\
    \tpart def S { part a : A; part b : A; flow from a.p to b.q; }\n\
    }\n";

#[test]
fn a_flow_end_that_refers_to_nothing_is_written_as_the_feature_it_redefines() {
    let w = edited("m.sysml", FLOW, |d| {
        let end = first(d, "FlowEnd");
        let reference = owned_by(d, "ReferenceSubsetting", "owningFeature", end);
        rm(d, reference, "referencedFeature");
    });
    says(&w.sysml, &["flow from A::p to b.q;"]);
}

#[test]
fn a_flow_end_whose_nested_feature_redefines_nothing_leaves_the_flow_in_its_body() {
    let w = edited("m.sysml", FLOW, |d| {
        let end = first(d, "FlowEnd");
        let nested = owned_by(d, "ReferenceUsage", "owningType", end);
        let redefinition = owned_by(d, "Redefinition", "owningFeature", nested);
        rm(d, redefinition, "redefinedFeature");
    });
    assert!(!w.sysml.contains("flow from"), "{}", w.sysml);
}

#[test]
fn a_flow_end_that_owns_something_no_end_owns_leaves_the_flow_in_its_body() {
    let w = edited("m.sysml", FLOW, |d| {
        let end = first(d, "FlowEnd");
        let reference = owned_by(d, "ReferenceSubsetting", "owningFeature", end);
        d[reference]["@type"] = json!("Dependency");
    });
    assert!(!w.sysml.contains("flow from"), "{}", w.sysml);
}

/// A comment, or what it is about, held by the model as a property of its
/// own rather than as the annotations it owns: what a model built in a
/// program has, and a document does not.
#[test]
fn what_a_comment_names_as_its_subject_is_written_as_what_it_is_about() {
    use sysml_model::{ElementKind as K, Model, Value as V};
    let mut model = Model::new();
    let package = model.create(K::Package);
    model.set(package, "declaredName", V::String("P".to_string()));
    let part = model.create(K::PartDefinition);
    model.set(part, "declaredName", V::String("A".to_string()));
    model.add_owned(package, part);
    let other = model.create(K::PartDefinition);
    model.set(other, "declaredName", V::String("B".to_string()));
    model.add_owned(package, other);
    for (about, body) in [
        (V::RefList(vec![part, other]), "both"),
        (V::Ref(part), "one"),
    ] {
        let comment = model.create(K::Comment);
        model.set(comment, "body", V::String(body.to_string()));
        model.set(comment, "annotatedElement", about);
        model.add_owned(package, comment);
    }
    let w = write_with(&model, &[package], &HashMap::new(), true);
    says(
        &w.sysml,
        &["comment about A, B /* both */", "comment about A /* one */"],
    );
}

#[test]
fn a_step_that_follows_nothing_is_a_succession_from_the_start() {
    let sysml = written(
        "m.sysml",
        "package P { action def X { then a; action a; } }",
    );
    says(&sysml, &["action a;"]);
}

#[test]
fn a_step_after_a_connector_that_relates_nothing_follows_the_step_before_that() {
    let text =
        "package P { action def Y { action a; action b; connect a to b; then c; action c; } }";
    says(&written("m.sysml", text), &["then action c;"]);
    let w = edited("m.sysml", text, |d| {
        let i = first(d, "ConnectionUsage");
        rm(d, i, "relatedFeature");
    });
    assert!(w.unwritten.is_empty(), "{:?}", w.unwritten);
}

#[test]
fn a_succession_whose_end_has_a_name_is_written_with_the_end_rather_than_as_then() {
    let text = "package P { action def X { action a1; action a2; first a1; then a2; } }";
    says(&written("m.sysml", text), &["then a2;"]);
    let w = edited("m.sysml", text, |d| {
        let i = first(d, "SuccessionAsUsage");
        let end = owned_by(d, "ReferenceUsage", "owningType", i);
        d[end]["declaredName"] = json!("named");
        d[end]["name"] = json!("named");
    });
    says(&w.sysml, &["first named ::> a1 then a2;"]);
}

#[test]
fn a_succession_that_follows_only_successions_is_written_from_the_names_it_gives() {
    let sysml = written(
        "m.sysml",
        "package P { action def X { first a then b; first b then c; action a; action b; action c; } }",
    );
    says(&sysml, &["first a then b;", "first b then c;"]);
}

#[test]
fn a_connector_end_that_is_a_specialization_of_what_it_reaches_leaves_the_flow_in_its_body() {
    let w = edited("m.sysml", FLOW, |d| {
        let end = first(d, "FlowEnd");
        let reference = owned_by(d, "ReferenceSubsetting", "owningFeature", end);
        d[reference]["@type"] = json!("Specialization");
    });
    assert!(!w.sysml.contains("flow from"), "{}", w.sysml);
}

#[test]
fn an_entry_action_that_says_more_than_the_word_is_written_with_what_it_says() {
    let text = "package P { part def A; state def S { entry; } }";
    says(&written("m.sysml", text), &["entry;"]);
    let w = edited("m.sysml", text, |d| {
        let action = first(d, "ActionUsage");
        let ty = named(d, "PartDefinition", "A");
        let id = "00000000-0000-0000-0000-00000000beef";
        d[action]["ownedRelationship"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "@id": id }));
        let typing = json!({
            "@type": "FeatureTyping", "@id": id, "elementId": id,
            "typedFeature": { "@id": d[action]["@id"].clone() },
            "type": { "@id": d[ty]["@id"].clone() },
            "owningFeature": { "@id": d[action]["@id"].clone() },
            "owningRelatedElement": { "@id": d[action]["@id"].clone() },
        });
        d.push(typing);
    });
    says(&w.sysml, &["entry action : A;"]);
}

#[test]
fn metadata_typed_after_another_relationship_is_still_the_prefix_of_what_it_is_on() {
    let w = edited(
        "m.sysml",
        "package P { metadata def M; #M def Q2 { part x; } }",
        |d| {
            let m = first(d, "MetadataUsage");
            // what it owns, in the order the document gives it: a
            // subsetting before the typing
            let typing = owned_by(d, "FeatureTyping", "owningFeature", m);
            let id = "00000000-0000-0000-0000-00000000f00d";
            let subsetting = json!({
                "@type": "Subsetting", "@id": id, "elementId": id,
                "subsettingFeature": { "@id": d[m]["@id"].clone() },
                "subsettedFeature": d[typing]["type"].clone(),
                "owningFeature": { "@id": d[m]["@id"].clone() },
                "owningRelatedElement": { "@id": d[m]["@id"].clone() },
            });
            let typing_id = d[typing]["@id"].clone();
            d[m]["ownedRelationship"] = json!([{ "@id": id }, { "@id": typing_id }]);
            d.insert(typing, subsetting);
        },
    );
    says(&w.sysml, &["#M def Q2 {"]);
}

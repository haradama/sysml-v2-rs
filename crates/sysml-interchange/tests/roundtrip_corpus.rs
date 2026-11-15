//! JSON round-trip over the whole standard library, resolved first so the
//! reified typings and specializations -- and the derived properties
//! computed from them -- are exercised at full scale. Skipped when the
//! corpus submodule is not checked out.

use std::path::Path;

#[test]
fn library_round_trips_through_json() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../sysml-stdlib/library");
    if !root.exists() {
        eprintln!("skipping: {} not checked out", root.display());
        return;
    }
    let files = sysml_semantics::model_files(&root);

    let mut ws = sysml_semantics::Workspace::new();
    for path in &files {
        let text = std::fs::read_to_string(path).unwrap();
        ws.add_file(path.to_string_lossy(), &text);
    }
    ws.resolve_all();
    // the strict pipeline: the implied specializations become part of the
    // model before it is serialized
    let implied = ws.materialize_implied();
    assert!(implied > 1_000, "hardly anything was implied: {implied}");
    let model = ws.model();
    assert!(model.len() > 10_000);

    let json = sysml_interchange::to_json(model);
    // no UUID collisions -- among the elements and among the memberships
    // synthesized between them alike
    let objects = json.as_array().unwrap();
    let ids: std::collections::HashSet<&str> =
        objects.iter().map(|e| e["@id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), objects.len(), "duplicate element UUIDs");
    assert!(objects.len() > model.len(), "no memberships were written");

    // strict shape: every object carries exactly the property set its
    // metaclass declares, across the whole library
    for object in objects {
        let kind = sysml_model::ElementKind::from_name(object["@type"].as_str().unwrap()).unwrap();
        let mut declared: std::collections::BTreeSet<&str> = std::iter::once(kind)
            .chain(kind.ancestors().iter().copied())
            .flat_map(|k| k.own_features())
            .map(|meta| meta.name)
            .collect();
        declared.insert("@type");
        declared.insert("@id");
        let written: std::collections::BTreeSet<&str> = object
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(written, declared, "for a {}", kind.name());
    }

    let (rebuilt, _roots) = sysml_interchange::from_json(&json).unwrap();
    assert_eq!(rebuilt.len(), model.len());
    assert_eq!(sysml_interchange::to_json(&rebuilt), json);
}

/// What a declaration writes as its own relationship's -- a chain under
/// the subsetting that names it, a conjugated port's typing -- is written
/// and read back as the relationship's, and as nothing the declaring
/// feature owns.
#[test]
fn a_chain_and_a_conjugated_port_typing_round_trip_as_written() {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file(
        "m.sysml",
        "package P {\n\
         \tport def PD { out attribute x; }\n\
         \tpart def A { port pt : ~PD; }\n\
         \tpart f : A;\n\
         \tref sub :> f.pt;\n\
         }\n",
    );
    ws.resolve_all();
    assert!(ws.unresolved().is_empty(), "{:?}", ws.unresolved());
    let json = sysml_interchange::to_json(ws.model());
    let objects = json.as_array().unwrap();
    let by_id = |id: &serde_json::Value| {
        objects
            .iter()
            .find(|it| it["@id"] == id["@id"])
            .expect("a reference to an element the document holds")
    };
    let named = |name: &str| {
        objects
            .iter()
            .find(|it| it["declaredName"] == name)
            .unwrap_or_else(|| panic!("`{name}` is written"))
    };

    // `sub` owns its subsetting and nothing else: no feature named `pt`
    let sub = named("sub");
    assert_eq!(sub["ownedFeature"], serde_json::json!([]));
    let subsetting = objects
        .iter()
        .find(|it| it["@type"] == "Subsetting" && it["owningRelatedElement"]["@id"] == sub["@id"])
        .expect("`sub` subsets the chain");
    let related = subsetting["ownedRelatedElement"].as_array().unwrap();
    assert_eq!(related.len(), 1, "the chain is the subsetting's own");
    assert_eq!(related[0], subsetting["subsettedFeature"]);
    let chain = by_id(&related[0]);
    assert_eq!(chain["owningRelationship"]["@id"], subsetting["@id"]);
    assert_eq!(
        chain["ownedFeatureChaining"].as_array().map(Vec::len),
        Some(2),
        "a chaining for each step"
    );
    assert_eq!(chain["chainingFeature"][0]["@id"], named("f")["@id"]);
    assert_eq!(chain["chainingFeature"][1]["@id"], named("pt")["@id"]);

    // and `pt` is typed by the conjugate through a typing of its own kind
    let pt = named("pt");
    let typing = objects
        .iter()
        .find(|it| it["@type"] == "ConjugatedPortTyping")
        .expect("`~PD` is a conjugated port typing");
    assert_eq!(typing["typedFeature"]["@id"], pt["@id"]);
    assert_eq!(typing["conjugatedPortDefinition"], typing["type"]);
    assert_eq!(pt["ownedTyping"][0]["@id"], typing["@id"]);
    assert!(
        pt["type"]
            .as_array()
            .unwrap()
            .iter()
            .any(|it| it["@id"] == typing["type"]["@id"]),
        "`pt` is of the type it is typed by"
    );

    let (rebuilt, _roots) = sysml_interchange::from_json(&json).unwrap();
    assert_eq!(sysml_interchange::to_json(&rebuilt), json);
}

//! The committed `generated.rs` must stay in sync with the vendored
//! metamodel, and malformed metamodels must be rejected.

use std::path::Path;

#[test]
fn committed_generated_file_is_in_sync() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let kerml = std::fs::read_to_string(root.join("vendor/metamodel/KerML.xmi")).unwrap();
    let sysml = std::fs::read_to_string(root.join("vendor/metamodel/SysML.xmi")).unwrap();
    let generated = sysml_codegen::generate_source(&kerml, &sysml);
    let committed =
        std::fs::read_to_string(root.join("crates/sysml-model/src/generated.rs")).unwrap();
    assert_eq!(
        generated, committed,
        "generated.rs is stale — run `cargo run -p sysml-codegen`"
    );
    // run() rewrites the default path with identical bytes (kept in this
    // test so nothing else reads the file mid-write)
    let path = sysml_codegen::run();
    assert!(path.ends_with("crates/sysml-model/src/generated.rs"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), committed);
}

fn wrap(packaged: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<xmi:XMI xmlns:xmi="http://www.omg.org/spec/XMI/20161101"
    xmlns:uml="http://www.omg.org/spec/UML/20161101">
  <uml:Package xmi:id="T" name="T">{packaged}</uml:Package>
</xmi:XMI>"#
    )
}

const EMPTY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xmi:XMI xmlns:xmi="http://www.omg.org/spec/XMI/20161101"
    xmlns:uml="http://www.omg.org/spec/UML/20161101">
  <uml:Package xmi:id="E" name="E"/>
</xmi:XMI>"#;

#[test]
fn generates_a_minimal_metamodel() {
    let xmi = wrap(
        r#"<packagedElement xmi:id="Thing" xmi:type="uml:Class" name="Thing" isAbstract="true">
             <ownedAttribute xmi:id="Thing-flag" xmi:type="uml:Property" name="flag">
               <type href="https://www.omg.org/spec/UML/20161101/PrimitiveTypes.xmi#Boolean"/>
               <upperValue xmi:id="Thing-flag-upper" xmi:type="uml:LiteralUnlimitedNatural" value="1"/>
             </ownedAttribute>
           </packagedElement>
           <packagedElement xmi:id="Sub" xmi:type="uml:Class" name="Sub">
             <generalization xmi:id="Sub-gen" xmi:type="uml:Generalization">
               <general xmi:idref="Thing"/>
             </generalization>
             <ownedAttribute xmi:id="Sub-items" xmi:type="uml:Property" isDerived="true" name="items">
               <type xmi:idref="Thing"/>
               <upperValue xmi:id="Sub-items-upper" xmi:type="uml:LiteralUnlimitedNatural" value="-1"/>
             </ownedAttribute>
             <ownedAttribute xmi:id="Sub-kind" xmi:type="uml:Property" name="kind">
               <type xmi:idref="Color"/>
             </ownedAttribute>
           </packagedElement>
           <packagedElement xmi:id="Color" xmi:type="uml:Enumeration" name="Color">
             <ownedLiteral xmi:id="Color-red" name="red"/>
             <ownedLiteral xmi:id="Color-green" name="green"/>
           </packagedElement>"#,
    );
    let code = sysml_codegen::generate_source(EMPTY, &xmi);
    assert!(code.contains("pub enum ElementKind"));
    assert!(code.contains("Sub,"));
    assert!(code.contains("ElementKind::Sub => &[ElementKind::Thing]"));
    assert!(code.contains("FeatureType::Enumeration(EnumType::Color)"));
    assert!(code.contains(
        r#"FeatureMeta { name: "items", ty: FeatureType::Class(ElementKind::Thing), many: true, derived: true }"#
    ));
    assert!(code.contains(r#"Color::Red => "red""#));
}

#[test]
#[should_panic(expected = "unknown primitive type")]
fn rejects_unknown_primitive_types() {
    let xmi = wrap(
        r#"<packagedElement xmi:id="T1" xmi:type="uml:Class" name="T1">
             <ownedAttribute xmi:id="T1-x" xmi:type="uml:Property" name="x">
               <type href="https://www.omg.org/spec/UML/20161101/PrimitiveTypes.xmi#Weird"/>
             </ownedAttribute>
           </packagedElement>"#,
    );
    sysml_codegen::generate_source(EMPTY, &xmi);
}

#[test]
#[should_panic(expected = "unresolved reference")]
fn rejects_unresolved_references() {
    let xmi = wrap(
        r#"<packagedElement xmi:id="T1" xmi:type="uml:Class" name="T1">
             <generalization xmi:id="T1-gen" xmi:type="uml:Generalization">
               <general xmi:idref="Missing"/>
             </generalization>
           </packagedElement>"#,
    );
    sysml_codegen::generate_source(EMPTY, &xmi);
}

#[test]
#[should_panic(expected = "duplicate metaclass")]
fn rejects_duplicate_metaclasses() {
    let xmi = wrap(
        r#"<packagedElement xmi:id="A1" xmi:type="uml:Class" name="Twin"/>
           <packagedElement xmi:id="A2" xmi:type="uml:Class" name="Twin"/>"#,
    );
    sysml_codegen::generate_source(EMPTY, &xmi);
}

#[test]
#[should_panic(expected = "without type")]
fn rejects_untyped_properties() {
    let xmi = wrap(
        r#"<packagedElement xmi:id="T1" xmi:type="uml:Class" name="T1">
             <ownedAttribute xmi:id="T1-x" xmi:type="uml:Property" name="x"/>
           </packagedElement>"#,
    );
    sysml_codegen::generate_source(EMPTY, &xmi);
}

#[test]
#[should_panic(expected = "unresolved supertype")]
fn rejects_generalization_to_non_classes() {
    let xmi = wrap(
        r#"<packagedElement xmi:id="T1" xmi:type="uml:Class" name="T1">
             <generalization xmi:id="T1-gen" xmi:type="uml:Generalization">
               <general xmi:idref="Hue"/>
             </generalization>
           </packagedElement>
           <packagedElement xmi:id="Hue" xmi:type="uml:Enumeration" name="Hue"/>"#,
    );
    sysml_codegen::generate_source(EMPTY, &xmi);
}

#[test]
fn capitalizes_empty_enum_literals() {
    let xmi = wrap(
        r#"<packagedElement xmi:id="W" xmi:type="uml:Enumeration" name="Weird">
             <ownedLiteral xmi:id="W-empty" name=""/>
           </packagedElement>"#,
    );
    let code = sysml_codegen::generate_source(EMPTY, &xmi);
    assert!(code.contains("pub enum Weird"));
}

#[test]
#[should_panic(expected = "invalid XMI")]
fn rejects_invalid_xml() {
    sysml_codegen::generate_source("not xml", "not xml");
}

#[test]
fn binary_regenerates() {
    let dir = std::env::temp_dir().join("sysml-codegen-test");
    std::fs::create_dir_all(&dir).unwrap();
    let out_path = dir.join("generated.rs");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_sysml-codegen"))
        .env("SYSML_CODEGEN_OUT", &out_path)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("generated"));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let committed =
        std::fs::read_to_string(root.join("crates/sysml-model/src/generated.rs")).unwrap();
    assert_eq!(std::fs::read_to_string(&out_path).unwrap(), committed);
}

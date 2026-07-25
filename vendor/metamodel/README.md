# Vendored OMG normative metamodel (XMI)

These are the normative machine-readable metamodel files published by the
Object Management Group (OMG) with the specifications:

- `KerML.xmi` — from the KerML 1.0 specification,
  <https://www.omg.org/spec/KerML/20250201/KerML.xmi>
- `SysML.xmi` — from the SysML 2.0 specification,
  <https://www.omg.org/spec/SysML/20250201/SysML.xmi>

Copyright © Object Management Group, Inc. and the specification submitters.
The files are reproduced here unmodified, as permitted by the OMG
specification terms of use, which grant a license to use the specifications
to create and distribute software based upon them (see
<https://www.omg.org/legal/tm_list.htm> and the "Use of Specification"
terms in the specification documents).

`crates/sysml-model/src/generated.rs` is generated from these files.
Regenerate it with:

```sh
cargo run -p sysml-codegen
```

OMG®, SysML®, and Systems Modeling Language® are registered trademarks of
the Object Management Group. This project is not affiliated with or
endorsed by the OMG.

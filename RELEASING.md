# Releasing to crates.io

## Pre-flight

```sh
git submodule update --init --depth 1   # corpus for the regression tests
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --release
```

Bump `version` in `[workspace.package]` **and** in the
`[workspace.dependencies]` entries for the `sysml-*` path dependencies
(they must stay in sync so the published crates depend on the released
versions).

## Publish order

Crates must be published leaf-first (each `cargo publish` verifies the
build against the versions already on crates.io):

```sh
cargo publish -p sysml-syntax
cargo publish -p sysml-model
cargo publish -p sysml-semantics
cargo publish -p sysml-interchange
cargo publish -p sysml-api-client
cargo publish -p sysml-lsp
cargo publish -p sysml-cli
```

`sysml-codegen` is `publish = false` (it only regenerates
`sysml-model/src/generated.rs` from the vendored metamodel and is not
useful as a dependency).

Packaged crates do not include `vendor/` — the corpus- and
metamodel-dependent tests detect the missing directory and skip
themselves, so `cargo test` still passes for downstream users and on
docs.rs.

## Licensing note

The crates are `MIT OR Apache-2.0`. `crates/sysml-model/src/generated.rs`
is generated from the normative machine-readable metamodel published by
the OMG with the KerML 1.0 and SysML 2.0 specifications
(`vendor/metamodel/KerML.xmi`, `vendor/metamodel/SysML.xmi`); the OMG
specification terms of use permit creating and distributing software
based on the specifications (see `vendor/metamodel/README.md`). "SysML"
is a registered trademark of the Object Management Group; the published
crates describe the language without claiming affiliation.

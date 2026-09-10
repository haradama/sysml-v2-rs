# sysml-model

Element model: 175 metaclasses generated from the official Ecore
metamodel, arena storage, AST→model builder; the generator that writes
it from [`vendor/metamodel`](https://github.com/haradama/sysml-v2-rs/tree/main/vendor/metamodel) is in the same crate
behind the `codegen` feature.

```rust
let parse = sysml_syntax::parse(text);
let mut model = sysml_model::Model::new();
let built = sysml_model::build_into(&mut model, &parse, None);
```

The metaclasses are generated from the official Ecore metamodel rather
than written by hand, so what this crate believes about SysML is what
the specification says. Elements live in an arena and refer to one
another by `ElementId`, which is what lets a model hold the cycles a
real one has.

## Part of

[sysml-v2-rs](https://github.com/haradama/sysml-v2-rs) -- Rust libraries
for SysML v2. See the workspace README for how the crates fit together
and what has been validated against the official corpus.

## License

MIT OR Apache-2.0.

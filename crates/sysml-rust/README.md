# sysmlv2-rust

The Rust side of a model, both ways. `import` reads an existing crate's
rustdoc JSON as a SysML package whose definitions carry `@code` binding
metadata; `generate` writes Rust from a resolved model: definitions
become structs/enums (multiplicities as containers, declared values as
`Default`, inheritance flattened, cycles boxed), calculations become
functions and methods with simple result expressions translated,
`abstract` calculations and action definitions become traits, an action
whose dataflow the model wired completely becomes the body that performs
it, state definitions become state machines (guards translated where
they read the event payload), API-bound ports become generics and
`perform`ed actions delegating methods.

```rust
let generated = sysml_rust::generate(&model, &roots)?;
println!("{}", generated.rust);
for open in &generated.open {
    println!("still to write: {} -- {}", open.sysml, open.why);
}
```

What the model did not say comes back as a list rather than as a
silence: every `todo!` the output carries has an entry beside it saying
which part of the model would have to say more.

## Part of

[sysml-v2-rs](https://github.com/haradama/sysml-v2-rs) -- Rust libraries
for SysML v2. See the workspace README for how the crates fit together
and what has been validated against the official corpus.

## License

MIT OR Apache-2.0.

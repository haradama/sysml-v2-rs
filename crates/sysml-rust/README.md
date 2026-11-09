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

## Any other language

Rust is the language that ships. Every other is yours to add, through
the seam both directions go through, each half stated as data in no
language in particular:

- `surface` is what some code declares -- its records, enumerations,
  interfaces and functions, each with the name the code gives it and
  the path to it. The import reads a crate into one and
  `surface::to_sysml` writes the package, so only the reading of rustdoc
  is Rust's own; a program that writes the same JSON for another
  language is an importer for it. Everything SysML decides -- which
  names are quoted, how two members that want one name are told apart,
  what is refused for naming a type nothing declares -- is decided in
  the writer, once.
- `plan` is what a model implies for code: every definition's shape,
  its features with what they bottom out in and how many of them there
  are, a state machine's transitions and the state it starts in. A
  program that reads the JSON and writes the code is a generator for
  its language; `generate` is the one that ships.

```rust
let surface = sysml_rust::surface::read(&json)?;
let imported = sysml_rust::surface::to_sysml(&surface, None);

let plan = sysml_rust::plan::of(&mut workspace, &roots);
let json = serde_json::to_string(&plan)?;
```

`sysml import-code` and `sysml codegen` find such programs on `PATH`,
the way Doxygen finds a filter, as `sysml-import-<lang>` and
`sysml-gen-<lang>`; the `schema` feature derives the JSON schema of
each, which `sysml import-code --schema` and `sysml plan --schema`
print. The `@code` binding both directions meet at is spelled once, in
`binding`.

## Part of

[sysml-v2-rs](https://github.com/haradama/sysml-v2-rs) -- Rust libraries
for SysML v2. See the workspace README for how the crates fit together
and what has been validated against the official corpus.

## License

MIT OR Apache-2.0.

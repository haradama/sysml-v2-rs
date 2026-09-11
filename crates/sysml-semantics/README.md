# sysmlv2-semantics

Name resolution (imports, aliases, inheritance, implicit library
specializations, connector ends, the names inside expressions),
relationship reification and implied-relationship materialization — the
whole standard library resolves. Also the 180 well-formedness
constraints the specification states in OCL, read from the metamodel and
evaluated -- with the 235 derivations beside them answering for the
properties the metamodel declares are never stored. Three answers rather
than two, so a constraint reaching for part of the abstract syntax this
model does not build is reported as *not evaluated* instead of as a
violation -- and a flag the builder reads off the source for every
metaclass that has it answers with the default the metamodel declares,
which is what tells a model that said nothing from one this does not
build.

```rust
let mut ws = sysml_semantics::Workspace::new();
ws.load_dir(std::path::Path::new("crates/sysml-stdlib/library"))?;
ws.add_file("vehicle.sysml", text);
let stats = ws.resolve_all();
println!("{}/{} resolved", stats.resolved, stats.resolved + stats.unresolved);
```

[`Workspace::diagnose`] is the one entry point that answers what is
wrong with a model, in the order it is worth saying: what did not parse,
then what did not resolve, then the constraints the specification states
-- which are put only to a model that parses, resolves and has the
standard library to be asked against.

[`Workspace::diagnose`]: https://docs.rs/sysml-semantics/latest/sysml_semantics/struct.Workspace.html#method.diagnose

## Part of

[sysml-v2-rs](https://github.com/haradama/sysml-v2-rs) -- Rust libraries
for SysML v2. See the workspace README for how the crates fit together
and what has been validated against the official corpus.

## License

MIT OR Apache-2.0.

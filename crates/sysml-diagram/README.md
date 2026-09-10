# sysml-diagram

Definition/interconnection diagrams in the standard's own notation —
labelled compartment stacks, ports on the border, composite and
reference memberships told apart by the diamond the specification
draws.

```rust
let diagram = sysml_diagram::definition_diagram(&model, &roots);
let svg = sysml_diagram::render(&diagram, &sysml_diagram::Style::default());
```

Layout is done here by default. The `elk` feature adds
`render_with_elk`, which runs the Eclipse Layout Kernel (`elkrs`) for
the arrangement and the routes; it is behind a feature because spawning
a process and reading its JSON is no business of a crate that draws
SVG.

## Part of

[sysml-v2-rs](https://github.com/haradama/sysml-v2-rs) -- Rust libraries
for SysML v2. See the workspace README for how the crates fit together
and what has been validated against the official corpus.

## License

MIT OR Apache-2.0.

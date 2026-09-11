# sysmlv2-interchange

Standard JSON interchange: the complete property set of every metaclass,
derived ownership/naming/inheritance-closure/import properties, reified
memberships down to
`ParameterMembership`/`SubjectMembership`/`StateSubactionMembership`/...
with visibility and kind, deterministic UUIDs; resolved whole-library
round-trip tested.

```rust
let json = sysml_interchange::to_json(&model);
let (rebuilt, roots) = sysml_interchange::from_json(&json)?;
```

The identities are deterministic UUIDs derived from qualified names, so
the same model exported twice is the same document, and a diff between
two exports is a diff between two models.

## Part of

[sysml-v2-rs](https://github.com/haradama/sysml-v2-rs) -- Rust libraries
for SysML v2. See the workspace README for how the crates fit together
and what has been validated against the official corpus.

## License

MIT OR Apache-2.0.

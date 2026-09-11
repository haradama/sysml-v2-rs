# sysmlv2-cli

`sysml` command-line tool (`parse`, `fmt`, `check`, `stats`, `export`,
`diagram`, `import-rust`, `rustgen`, `api`, `mcp`, `corpus`); `mcp`
speaks the Model Context Protocol over stdio, so an AI agent can ask
whether a model parses and resolves, what names are legal at a point,
what shape a definition has once it is resolved, and what the standard
library actually declares -- and can have the two directions between
code and model derived rather than guessed: an existing Rust crate's API
stated as SysML, and the Rust a model implies together with the list of
what the model left for a person to write. `--project` holds the model
being worked on, so a call names no files and cannot leave one out;
`api` is the client for the SysML v2 API & Services REST standard, and
`api push` sends what `export` writes -- the model, and not the library
it was resolved against: what the model refers to across that line is an
`@id`, and those are derived from the ownership path, so anybody holding
the same library computes the same ones. `--include-library` writes a
document that stands on its own instead.

```sh
sysml parse vehicle.sysml
sysml check model/
sysml diagram vehicle.sysml -o vehicle.svg
sysml mcp
```

A finding is said twice over. To a person it is drawn the way rustc
draws one -- the line quoted, the span underlined, and what is known
about it underneath, which for a name that resolved to nothing is either
the import it wants or the name it was nearly spelt as:

```console
$ sysml check model/
error: `Wheeel` resolves to nothing
 --> model/car.sysml:5:18
  |
5 |         part w : Wheeel;
  |                  ^^^^^^
  |
help: did you mean `Cars::Wheel`?
```

To a program, under `--format json`, which every subcommand takes: each
finding placed by `offset`, `line` and `column` and by where it *ends*,
so the span can be underlined again without lexing the line, and each
unresolved name carrying `declaredAs` and `didYouMean`. `check` also
says which standard library answered it, since without one every
reference into the library reads as unresolved.

`--color auto|always|never` decides the colour; `auto` is what a
terminal gets, and `NO_COLOR` is honoured.

`sysml plan` says what a model implies for code, in no language in
particular: every definition's shape, its features with their
multiplicities and what they bottom out in, a state machine's transitions
and the state it starts in, a requirement's own constraint. It is what
`rustgen` writes Rust from, handed over so that whoever has another
language can write it without guessing.

## Part of

[sysml-v2-rs](https://github.com/haradama/sysml-v2-rs) -- Rust libraries
for SysML v2. See the workspace README for how the crates fit together
and what has been validated against the official corpus.

## License

MIT OR Apache-2.0.

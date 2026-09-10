# sysml-syntax

Lexer (logos) + recursive-descent parser + lossless CST (rowan) + typed
AST.

```rust
let parse = sysml_syntax::parse("part def Vehicle { part eng : Engine; }");
assert!(parse.errors().is_empty());
```

The tree is lossless: every byte of the source is in it, including
whitespace and comments, so a formatter and an editor can both work from
it. Parsing is error-tolerant -- a file that does not parse still yields
a tree, with the parser's complaints beside it, because an editor has to
say something about a file the moment it stops being valid.

## Part of

[sysml-v2-rs](https://github.com/haradama/sysml-v2-rs) -- Rust libraries
for SysML v2. See the workspace README for how the crates fit together
and what has been validated against the official corpus.

## License

MIT OR Apache-2.0.

# sysml-lsp

Language server: diagnostics, go-to-definition, find-references, rename,
completion, hover, symbols, formatting — with a [VSCode
extension](https://github.com/haradama/sysml-v2-rs/tree/main/editors/vscode) as its client.

```sh
sysml-lsp            # speaks LSP over stdio
```

The standard library is parsed and resolved once at startup; the project
is that workspace cloned with the client's files added, and the analysis
is that cloned again with the open buffers on top. A keystroke rebuilds
only the top layer.

## Part of

[sysml-v2-rs](https://github.com/haradama/sysml-v2-rs) -- Rust libraries
for SysML v2. See the workspace README for how the crates fit together
and what has been validated against the official corpus.

## License

MIT OR Apache-2.0.

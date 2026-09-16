# sysmlv2-lsp

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

## Two hosts

Nothing in here reads a message from anywhere. `Session::handle` takes
one message and hands back the messages that answer it; `run` is that
session with `lsp_server`'s two threads and a channel around it. A host
with neither -- a browser's worker, which may block for nothing -- drives
the same session without a loop, which is what
[`sysmlv2-wasm`](../sysml-wasm) does.

`Files` says where the project's files come from, for the same reason: a
filesystem, or the client, which in a browser is the only side of the
connection that can read a workspace. A client that reads them hands
them over in `initializationOptions.files` and keeps them up to date
with a `sysml/files` notification.

## Part of

[sysml-v2-rs](https://github.com/haradama/sysml-v2-rs) -- Rust libraries
for SysML v2. See the workspace README for how the crates fit together
and what has been validated against the official corpus.

## License

MIT OR Apache-2.0.

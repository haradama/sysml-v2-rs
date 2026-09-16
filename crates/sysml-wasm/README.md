# sysmlv2-wasm

[`sysmlv2-lsp`](../sysml-lsp), compiled to WebAssembly: the SysML v2
language server with no machine under it.

```sh
cargo build -p sysmlv2-wasm --target wasm32-unknown-unknown --profile wasm
```

That is the whole toolchain — `rustup target add wasm32-unknown-unknown`
and `cargo build`. There is no binding generator, because what crosses
the boundary is one string each way:

```js
const bytes = new TextEncoder().encode(JSON.stringify(message));
new Uint8Array(wasm.memory.buffer, wasm.take(bytes.length), bytes.length).set(bytes);
const length = wasm.handle(bytes.length);
const answers = JSON.parse(
  new TextDecoder().decode(new Uint8Array(wasm.memory.buffer, wasm.answers(), length))
);
```

Everything else the module needs, it carries: the parser, the metamodel,
name resolution, the specification's constraints, the diagram renderer,
and the KerML and SysML v2 standard libraries, which are 94 files it
reads and resolves in about half a second the first time a client says
`initialize`. A keystroke after that is answered in single-figure
milliseconds.

## What a browser has not got

**A filesystem.** The server reads the project's files from what the
client hands it — in `initializationOptions.files` for the set it starts
with, and in a `sysml/files` notification for anything that changes
after. Each file is `{ "uri": …, "text": … }`, and a `text` of `null` is
one that is gone. The client is the only side of the connection that can
read a workspace, so it is the side that does.

**Threads.** `lsp-server`'s stdio connection reads on one thread and
writes on another, and a worker has neither to spare. So the server is
driven by `sysmlv2_lsp::Session`, which takes one message and hands back
the messages that answer it — the same code the binary runs, without the
loop around it.

## Not published

This is the inside of the [VSCode
extension](../../editors/vscode), which ships the `.wasm` itself. A
crate on crates.io that is only useful once it has been compiled to
WebAssembly and wrapped in fifty lines of JavaScript is a crate nobody
can use. The server it wraps is published, as `sysmlv2-lsp`.

## Part of

[sysml-v2-rs](https://github.com/haradama/sysml-v2-rs) -- Rust libraries
for SysML v2. See the workspace README for how the crates fit together
and what has been validated against the official corpus.

## License

MIT OR Apache-2.0.

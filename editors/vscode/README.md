# SysML v2 for Visual Studio Code

Language support for SysML v2 (`.sysml`) and KerML (`.kerml`), backed by
[`sysml-lsp`](../../crates/sysml-lsp): as-you-type parse and name-resolution
diagnostics, completion, go-to-definition, find references, rename, hover,
document symbols and formatting, plus TextMate syntax highlighting and a
live diagram preview (`SysML: Open Diagram Preview`, or the editor-title
button): definitions, one element's internal structure, or the membership
tree, following unsaved edits.

## Setup

From the repository root:

```sh
make vscode
```

That builds `sysml-lsp`, bundles it and the standard library into the
extension, packages a `.vsix` and installs it -- no configuration needed.
Reload VSCode windows afterwards.

Manual override, when wanted: `sysml.server.path` points at another server
binary, `sysml.library.path` at another standard library (else
`SYSML_LIBRARY_PATH` is honoured).

## Development

```sh
npm install
npm run compile   # or: press F5 in VSCode to launch an Extension Host
```

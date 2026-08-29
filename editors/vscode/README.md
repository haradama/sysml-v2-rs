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

The preview opens beside the editor when a `.sysml` or `.kerml` file is
opened, and follows whichever model you are editing. Closing it keeps it
closed until you ask for it again (`SysML: Open Diagram Preview`, or the
button in the editor title bar); `sysml.preview.openAutomatically` turns
the opening off for good.

Its toolbar zooms: `+` and `-` step by a fifth, `1:1` goes back to the
size it was drawn at, and `Fit` scales it to the window. Ctrl (or
command) and the wheel zoom towards the pointer, the wheel by itself
scrolls, dragging pans, and `+`, `-` and `0` do what the buttons do. An
edit redraws without disturbing where you had scrolled to or how far in
you had zoomed.

`Save...` writes the diagram out. The format follows the name you give
it: `.svg` saves what the server drew, and `.png` saves the preview's own
rendering of it at twice the size -- in the colours and on the background
you are looking at it in, so a dark theme saves the dark drawing.

## What resolves against what

Every `.sysml`/`.kerml` file in the workspace folders is part of the
model, whether or not it is open in a tab -- a file that imports a
sibling resolves against it as it sits on disk, and against the buffer
once you open it. `sysml.workspace.exclude` lists directories that are
not yours to edit (a vendored corpus, someone else's model): their names
do not resolve and are not offered in completion, and the server does not
pay to load them.

## Development

```sh
npm install
npm run compile   # or: press F5 in VSCode to launch an Extension Host
```

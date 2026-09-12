# SysML v2 for Visual Studio Code

Language support for SysML v2 (`.sysml`) and KerML (`.kerml`), backed by
[`sysml-lsp`](https://github.com/haradama/sysml-v2-rs/tree/main/crates/sysml-lsp): as-you-type parse and name-resolution
diagnostics, completion, go-to-definition, find references, rename, hover,
document symbols and formatting, plus TextMate syntax highlighting and a
live diagram preview (`SysML: Open Diagram Preview`, or the editor-title
button): definitions, one element's internal structure, or the membership
tree, following unsaved edits.

Highlighting tells the model from what is written beside it. The prose in
a `doc` is an element of the model -- `sysml export` writes it out, and
hover shows it -- so it is coloured as the content it is, while a `/* */`
or `//` beside it, which nothing in the model keeps, is coloured as a
comment. What a `comment` element holds is a comment, and reads as one.

## Setup

None. The language server and the standard library travel inside the
extension, so the names in a model resolve the moment a file is opened.

Manual override, when wanted: `sysml.server.path` points at another server
binary, `sysml.library.path` at another standard library (else
`SYSML_LIBRARY_PATH` is honoured), `sysml.format.width` says how wide a
line may be before formatting breaks it -- 0 leaves every line as long as
it comes -- `sysml.diagram.skin` says how the preview is painted (the
name of a skin that ships, or an object of colours; see
[`sysml-diagram`](https://github.com/haradama/sysml-v2-rs/tree/main/crates/sysml-diagram#skins)),
and `sysml.trace.server` logs the traffic between VSCode and the server
in the output channel. All of them are read when the server
starts, so a window reload is what applies a change.

The preview is opened with `SysML: Open Diagram Preview`, or the button
in the editor title bar, and then follows whichever model you are
editing. Closing it keeps it closed until you ask again. Set
`sysml.preview.openAutomatically` to have it open by itself whenever a
`.sysml` or `.kerml` file is opened.

Beside the view is how much of the model the drawing is of: `This file`
draws the document on its own, `This folder` draws every model file in
its directory and below alongside it. A model is commonly written across
several files -- one declaring the definitions, another the usages of
them -- and the usages half read on its own declares no definition at
all, which is when the `Definitions` view has nothing to draw and falls
back to the tree. An internal view is of one element, so the choice is
put away while that view is showing.

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

Opening a file from inside one is the exception. It is read together with
the model files in its own directory -- that directory and no deeper --
so that half a model does not report every name its other half declares
as unresolved. Those files are part of the model for as long as the
document is open, so the names in that directory resolve everywhere while
it is. The standard library is never read this way: it is loaded once at
startup, and a second copy of it would collide with the first at every
name.

## Development

From the repository root, `make vscode` builds `sysml-lsp`, bundles it
and the standard library, packages a `.vsix` and installs it. Inside
this directory:

```sh
npm install
npm run compile   # or: press F5 in VSCode to launch an Extension Host
npm test
```

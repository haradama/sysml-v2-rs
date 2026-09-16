# Changelog

## 0.2.0

- The language server is a WebAssembly module now, run in a worker.
  One package installs on Linux, macOS and Windows -- there is no
  platform matrix any more -- and the same one runs in a browser:
  vscode.dev, github.dev, a repository nobody has checked out.
- Nothing is spawned on your machine, so the extension works in a
  workspace you have not trusted.
- The package is 1.8 MB, down from 3.9 MB per platform. The standard
  library ships once, inside the server, rather than again as 94 files
  beside it.
- `sysml.server.path` is gone. There is no process to point at
  another binary: the server is the module inside the extension. A
  `sysml-lsp` you built yourself still speaks stdio and still works with
  any other editor.
- `sysml.library.path` takes a URI as well as a path, so a standard
  library can be somewhere the editor can read rather than only
  somewhere the machine can. `SYSML_LIBRARY_PATH` is no longer read: a
  browser tab has no environment.
- What the server used to say on standard error -- a library path that
  would not open, a skin that would not read -- is said in the output
  channel, where the rest of its log already was.
- Quick fixes for a name that resolves to nothing: the spellings it might
  have wanted, as edits. A name nothing declares was mistyped and the fix
  is the nearest declared name; a name something declares was never in
  scope here and the fix spells it from the root.

## 0.1.1

- `sysml.format.width`: where a line too wide to read is opened, in
  columns. `0` leaves every line as it was written.
- `sysml.diagram.skin`: what the preview is painted in -- the name of
  a skin that ships, or a palette of your own, down to the colour of
  one kind of box and one kind of line.
- Formatting a doc comment of several lines keeps its body together
  rather than leaving the margin of wherever it used to be.
- Editing a large model costs what the edit is rather than what the
  model is.

## 0.1.0

First release.

- Diagnostics as you type: parse errors, unresolved names with what they
  might have meant, and the constraints the specification states.
- Completion, go-to-definition, find references, rename, hover, document
  symbols and formatting.
- A live diagram preview in the standard's own notation -- definitions,
  one element's internal structure, or the membership tree -- that
  follows unsaved edits, zooms, and saves as SVG or PNG.
- Syntax highlighting generated from the lexer's own keyword table.
- The language server and the standard library travel inside the
  package, so nothing needs installing or configuring.

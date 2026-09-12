# Changelog

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

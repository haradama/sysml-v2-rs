# The playground

SysML v2 in a browser tab: type a model on the left, watch it drawn on
the right. Like the PlantUML web server, except that there is no server
— the parser, the metamodel, name resolution, the specification's
constraints, the layout and the renderer are a WebAssembly module
running in the page, and the model never leaves the machine it is typed
on.

```sh
make web-serve   # http://localhost:8000
```

## What it is

Five static files and a `.wasm`:

| | |
| --- | --- |
| `index.html`, `style.css`, `favicon.svg` | the page |
| `app.js` | the front end: the editor, the drawing, the zoom, the link |
| `server.worker.js` | the worker the language server runs in |
| `sysml-lsp.wasm` | [`sysmlv2-wasm`](../crates/sysml-wasm) — the language server, 6.3 MiB, 1.7 MiB over the wire |
| `sysml.tmLanguage.json`, `onig.wasm` | what colours the editor, and the engine that reads it |

The page and the module speak the Language Server Protocol, the same
conversation the [VSCode extension](../editors/vscode) has:
`textDocument/didChange` for a keystroke, `textDocument/publishDiagnostics`
for what comes back under the editor, `textDocument/codeAction` for what
a name that resolved to nothing might have meant, and the custom
`sysml/diagram` for the SVG. The worker is the extension's own file, bundled from where it
lives rather than copied here — two copies of a binary interface is one
copy that quietly stops matching the `.wasm`.

The standard libraries are inside the module: 94 KerML and SysML v2
files, read and resolved once while the page says *loading*, which is why
`ISQ::MassValue` resolves in a tab that has fetched one file. A keystroke
after that is answered in single-figure milliseconds.

## What colours the editor

The grammar the [VSCode extension](../editors/vscode) ships, read by
`vscode-textmate` and `vscode-oniguruma` — the engine VSCode itself
tokenises with. So the colouring here is the colouring there, and there
is one grammar to keep right rather than two;
`editors/vscode/test/grammar.test.mjs` is what keeps it right, by reading
the reserved words straight out of the lexer's own table in
`crates/sysml-syntax/src/kind.rs`.

It is a `<pre>` under a transparent `<textarea>`, which is the whole
trick: the text is typed into the textarea and read off the layer
beneath. The layer is only handed the showing once the grammar has
arrived, so a grammar that never loads leaves a plain editor rather than
an empty one. Past 50,000 characters it leaves one too — replacing the
whole painted layer costs about 0.7 ms per thousand characters, and a
keystroke is not worth more than the frame it lands in.

## What it draws

Three views, the same three the preview in VSCode has:

- **Definitions** — the definitions in the model and the specializations
  between them, laid out by the Eclipse Layout Kernel.
- **Internal structure** — one definition from the inside: the parts it
  is assembled from, their ports, and what is connected to what. The box
  beside the menu names which definition.
- **Tree** — the membership hierarchy, which can show any model at all.

## Problems, and what to do about them

What the server publishes goes under the editor, each one saying where it
is written. Clicking one puts the caret there and asks the server what
the name might have meant — two answers, kept apart because they are not
the same mistake:

- **did you mean `Garage::Wheel`?** — nothing declares the name, so it
  was mistyped.
- **`Garage::Wheel`, declared elsewhere** — something does declare it;
  nothing brought it into scope here.

Either is an edit, and clicking it applies it; `Escape` puts them away
again. It is asked for on the click rather than published with the
problem: the walk behind it is over every declared name, sixty thousand
of them with the standard library loaded, which is nothing once and a
stutter on every keystroke.

An edit the page makes — applying one of these, or `Tab` indenting — goes
through `execCommand`, which is deprecated and is still the only edit a
page can make that joins the textarea's own undo history. So the usual
`Ctrl`/`Cmd`+`Z` takes back a fix as readily as it takes back typing.

## Saving

One button, and one dialog: the page's own, in every browser. Three
things can come out of it, and the name and the menu beside it say which
together:

| | |
| --- | --- |
| `.sysml` | the model as it was typed |
| `.svg` | the drawing, with the theme it is being read in written into it |
| `.png` | the same, rasterised at twice its size, on the page's own background |

The box holds a name and the menu holds the extension, so the two read as
one file name and neither can contradict the other. A name typed with an
extension on it moves the menu to match; what is saved takes that
extension off again and puts the menu's on, so `model.sysml` saved as SVG
is `model.svg`. Only one of the three comes off that way — `wheel.v2` is
a name, so saved as a model it is `wheel.v2.sysml`.

`showSaveFilePicker` is what this used at first, since it can choose the
folder as well. What it does with the *name*, though, is the browser's:
on at least one platform it ignores the type the reader picks and puts
the first type's extension onto a name that already shows one, so a
drawing asked for as SVG arrives as `model.sysml.sysml`. That is not
something the page can reach in and correct, so it does not reach for it
at all. Where the file lands is the browser's download setting, which can
be told to ask.

## The handle between the panes

Drag it to give the model more of the window or less; double-click it to
go back to half and half. It takes the keyboard too — `Tab` to it, then
the arrow keys — and neither pane can be shut away entirely.

It is the border between the panes as well, and stays one pixel wide:
what can be grabbed reaches three pixels either side of what is drawn,
so the handle is easy to aim at without a thick line down the middle of
the window. The highlight waits 300ms, as VSCode's sash does, so a
pointer crossing on its way somewhere else does not light it up — while
taking hold of it lights it at once, with no wait it did not ask for.

## The link

The address bar carries the model, the way PlantUML's does, so a link is
the whole thing a reader needs:

```text
https://haradama.github.io/sysml-v2-rs/#view=definitions&m=cGFja2FnZSDigKY
```

It is a fragment, and a browser never sends a fragment. Sharing one is
between the two people holding it and no server, this one included.

## Building it

```sh
npm install
npm run build     # into dist/, which is the whole site
npm run serve     # the same, watched, at http://localhost:8000
npm test          # the built page, driven against the built .wasm
```

`npm run build` needs the module. `make web` builds both:

```sh
cargo build -p sysmlv2-wasm --target wasm32-unknown-unknown --profile wasm
```

The test loads `dist/index.html` into a DOM, gives it a `Worker` running
`dist/server.worker.js`, and types into it — so what it checks is the
whole crossing, against the module that is about to be published rather
than a description of it.

## Published by

[`.github/workflows/pages.yml`](../.github/workflows/pages.yml), on every
push to `main` that can change what the site is. Pages has to be told to
take its content from Actions: **Settings → Pages → Build and deployment
→ Source → GitHub Actions**.

## License

MIT or Apache-2.0, at your option — except two things that are not this
project's to license. The standard libraries inside the module are the
OMG's and are EPL-2.0. `onig.wasm` is Oniguruma, under a licence that
asks a binary redistribution to reproduce its notice, and
`vscode-textmate` and `vscode-oniguruma` around it are MIT.
`dist/LICENSE.txt` carries all of it, which is what each of them asks of
whoever distributes it.

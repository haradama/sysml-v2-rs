# Changelog

Ten crates are published from this repository and each carries its own
version, because what a number moves for is a change in the crate it is
on. So an entry here names the crates it is about and the version each
went to; a crate not named in an entry did not change in it.

Dates are the day the tag was cut. Anything under **Unreleased** is on
`main` and is not on crates.io yet.

## 2026-09-17

Nine of the ten went up together: `sysmlv2-syntax` 0.1.3, `sysmlv2-model`
0.1.2, `sysmlv2-semantics` 0.1.2, `sysmlv2-stdlib` 0.1.1,
`sysmlv2-interchange` 0.2.0, `sysmlv2-diagram` 0.2.1, `sysmlv2-rust`
0.1.1, `sysmlv2-lsp` 0.2.0 and `sysmlv2-cli` 0.1.4. `sysmlv2-corpus` is
not published and never was.

### A name is drawn in a face wider than the one it was measured in

`sysmlv2-diagram` **0.2.1**

- A package's name no longer hangs over the end of the tab it is written
  in. A name is set bold, and a bold face is about a tenth wider than the
  regular one at the same size, which boxes have allowed for since they
  were written and packages and swimlanes had not.
  `ArduinoCompatibleHardware` came to 198 px in DejaVu Sans Bold — what a
  Linux box renders `Arial, Helvetica, sans-serif` as — and was given
  190; thirteen of the fifty-seven package names in the corpus were over
  their tabs the same way.
- A frame now leaves room for the corner its tab is cut off at as well,
  so a package holding one small box under a long name is not widened to
  the name and then narrowed again by the cut.

### A language server that runs where there is no machine

`sysmlv2-lsp` **0.2.0**, and `sysmlv2-wasm`, which is new and is not
published: it is the inside of the VSCode extension.

- The server no longer reads its messages from a channel or its files
  from a filesystem, because a browser has neither to offer.
  `Session::handle` takes one message and hands back the messages that
  answer it, and `Files` says where a project's files come from — a
  disk, or the client, which in a browser is the only side of the
  connection that can read a workspace. `run` over stdio is that session
  with a loop around it; the binary and every other editor's client are
  unchanged.
- Two additions to what a client may say, for a client that has to do
  the reading: `initializationOptions.files` is the set of files the
  project starts as, and `sysml/files` is how it changes afterwards —
  each `{ "uri", "text" }`, and a `text` of `null` a file that is gone.
  The set travels in the handshake rather than after it so that the
  first document opened is not diagnosed against a project of nothing.
- `sysmlv2-wasm` compiles all of it to `wasm32-unknown-unknown`: 6.3 MiB,
  1.7 MiB over the wire, holding the parser, the metamodel, name
  resolution, the constraints, the diagrams and the standard library.
  The toolchain is `rustup target add wasm32-unknown-unknown` and
  `cargo build` — what crosses between the two languages is one string
  each way, which is not worth a binding generator pinned to a crate's
  version.
- The VSCode extension (**0.2.0**) ships that module instead of five
  platform binaries, and runs in a browser as well as on a machine. The
  package is 1.8 MB where each platform's was 3.9 MB.
- What the server wrote to standard error it now says in
  `window/logMessage`. Standard error in a worker goes nowhere at all,
  and a library path that would not open was a setting that went wrong
  in silence.
- One crate in the workspace does not take the workspace's
  `unsafe_code = "forbid"`: `sysmlv2-wasm` keeps it at `deny`, with the
  exception on the module holding the four exports, because
  `#[no_mangle]` is itself an unsafe attribute and a `cdylib` has no
  other way to name what it exports. Everything the model is actually
  made of still forbids it.

### A reserved word written as a name

`sysmlv2-syntax` **0.1.3**, `sysmlv2-model` **0.1.2**

- A role keyword is the role of the declaration it leads, and of no
  other. It used to be whichever role keyword turned up anywhere in the
  declaration — and a declaration collects loose keywords, because a
  reserved word written as a name is swept up into it. So
  `part frame : R;` came out as a framed concern, which the standard
  makes a kind of requirement constraint, and
  `validateRequirementConstraintMembershipIsComposite` and
  `validateRequirementConstraintMembershipOwningType` were both violated
  by a file that declares no requirement at all. Nothing in the message
  pointed at the name, which is where the mistake was.
- A definition named after a reserved word is reported, with the
  quoting that fixes it: ``part def frame;`` now says ``` `frame` is
  reserved; write `'frame'` to use it as a name ```. The word used to be
  swept up the same way and the definition came out with no name, in
  silence, because nothing had asked it for one. The word is kept as the
  name it was written as, so an editor can still find and rename it.
- Only where a definition is named. A usage's name is not so certain:
  `return part : Engine;` is a return parameter that is a part, and
  `succession first [0..1] a then [1] b;` opens the succession's own
  clause — in both the keyword is doing its own job in the very position
  a name would take, and the official corpus writes both.

### What a name that resolved to nothing might have meant, in an editor

`sysmlv2-lsp` **0.2.0**

- `textDocument/codeAction` answers with the spellings a dangling name
  might have wanted, as edits. `Workspace::suggestions` has told the two
  mistakes apart since `sysml check` and the MCP server first asked it —
  a name nothing declares was mistyped, a name something declares was
  never in scope here — and the server was the one front end that had
  never asked. Both come back as `quickfix`, and the title says which.
- Asked rather than published. That walk is over every declared name,
  sixty thousand with the standard library loaded, which is nothing on
  the click that wants it and a stutter on every keystroke.

### SysML v2 in a browser tab, with nothing behind it

`web`, which is new, publishes no crate and is not on crates.io: it is a
static site.

- The toolchain as a page, at
  [haradama.github.io/sysml-v2-rs](https://haradama.github.io/sysml-v2-rs/)
  — a model on the left, the diagram of it on the right, redrawn as it is
  typed. The PlantUML web server's arrangement without the server: the
  same `.wasm` the VSCode extension ships runs in the reader's tab, so a
  model is never uploaded and the whole site is five static files and a
  module. The three views the preview has — definitions, internal
  structure, tree — and the diagnostics under the editor, each naming
  where it is written.
- The address bar carries the model, so a link is the whole thing a
  reader needs. It is a fragment, which a browser never sends.
- One **Save** button in place of the two that wrote the drawing out, and
  one dialog behind it — the page's own, in every browser. The box holds
  a name and the menu beside it the extension: `.sysml` is the model as
  it was typed, `.svg` and `.png` the drawing. `showSaveFilePicker` can
  choose the folder as well and was what this used at first, but what it
  does with the name is the browser's, and on at least one platform it
  ignores the type the reader picks and hands back `model.sysml.sysml`
  for a drawing asked for as SVG.
- The panes are divided by a handle: drag it, double-click it for half
  and half, or reach it with the keyboard and use the arrow keys.
- What the page edits, a reader can take back. A textarea keeps its own
  undo history and nothing written into `value` joins it, so applying a
  fix and indenting with `Tab` both go through `execCommand` — deprecated,
  and still the only edit a page can make that `Ctrl`+`Z` knows about.
- The editor numbers its lines, and clicking a problem asks the server
  what the name might have meant — `did you mean \`Garage::Wheel\`?` where
  nothing declares it, `\`Garage::Wheel\`, declared elsewhere` where
  something does and nothing brought it into scope. Clicking the answer
  applies it, and `Escape` puts them away.
- The editor is coloured by the grammar the VSCode extension ships, read
  by the engine VSCode itself tokenises with — so the colouring is the
  same in both and there is one grammar to keep right, held to the
  lexer's own keyword table by the extension's test. It is a `<pre>`
  under a transparent `<textarea>`; the layer is handed the showing only
  once the grammar has arrived, so a grammar that does not load leaves a
  plain editor rather than an empty one.
- The worker that drives the module is the extension's own file, bundled
  from where it lives rather than copied. Two copies of a binary
  interface is one copy that quietly stops matching the `.wasm`.
- `.github/workflows/pages.yml` builds the module, builds the site, types
  into the built page against the module that is about to be published,
  and deploys. `make web-serve` is the same thing at
  `http://localhost:8000`.

### The standard interchange, read as well as written

`sysmlv2-cli` **0.1.4**

- `sysml import <json>` reads a standard interchange document back into a
  model and says what it holds, or draws it with `--diagram`. The reader
  it calls has been round-trip tested over the whole standard library
  since it was written and was reachable from no command at all, so a
  document off a model server — which `sysml api` will fetch for you —
  had nowhere to go.
- A document that names elements it does not carry is refused in words
  rather than by a bare UUID. That is what `sysml export` writes without
  `--include-library`, since every definition implicitly specializes
  something in the library, and it is also the shape a model server
  returns for one page of elements.

### A drawing's skin answers with an error, not with a string

`sysmlv2-diagram` **0.2.0** (breaking), `sysmlv2-lsp`, `sysmlv2-cli`

- `skin::read` answers `Result<Skin, SkinError>` where it answered
  `Result<Skin, String>`. It was the one public function in the
  workspace whose error was a bare `String`, which is nobody's error but
  its own: it cannot go into a `Box<dyn Error>`, and `?` cannot carry it
  into a function that returns one. `SkinError` displays the same
  sentence, so a caller that only printed it needs no change; one that
  matched on the text does.

### What the specification says about each metaclass

`sysmlv2-model`, `sysmlv2-syntax`

- The generator lifts the paragraph the OMG metamodel writes about each
  of the 175 metaclasses and each enumeration and its literals into the
  doc comment on `ElementKind` and on the enumerations. It was there all
  along, in the same comment a constraint's `says` is read from, and
  nothing had read it. `NOTICE` says which parts of the generated file
  are the OMG's and this is now among them.
- `missing_docs` is on in every crate here. It had been off in
  `sysmlv2-model`, whose surface is generated, and in `sysmlv2-syntax`,
  where the argument was that the syntax kinds are their own
  documentation — true of the kinds, and it had been excusing `AstNode`,
  `Parse` and every accessor of the typed tree along with them. The
  exemption is now taken on `SyntaxKind` itself, where it can only cover
  what it was argued for.
- Every crate takes its lints from `[workspace.lints]` rather than
  repeating the same attributes and the same paragraph of reasoning in
  ten `lib.rs` files, where it had gone stale in eight of them. The
  binaries are held to them too, which the per-library attributes never
  reached.

### A document that is never a document

`sysmlv2-interchange` **0.2.0**

- `write_json` writes a model out an element at a time, and `read_json`
  reads one back the same way. Between them, a document of the standard
  library no longer has to exist whole as a `serde_json::Value`:

  | | before | after |
  | --- | --- | --- |
  | `sysml export --include-library` | 6,746 MB | **83 MB** |
  | `sysml import` of what it wrote | 6,938 MB | **862 MB** |

  The model is 47 MB and the document is 754 MB of text. The six
  gigabytes between them were the `Value` — a hundred and twenty times
  the model, built only to be turned into text and dropped. What is left
  on the way in is the text itself, which is what `@id` references are
  resolved against.
- Export is also quicker for it (79 s to 61 s), and the bytes it writes
  are the same bytes. Import is 21% slower (38 s to 46 s): the array is
  read twice, once for the two fields that pass one wants and once for
  each element as pass two reaches it.
- `ImportError::NotJson` says that text is not JSON at all, which is not
  what `NotAnArray` says — JSON this reader can read but did not expect.
  `serde` tells them apart and so does the message.
- `sysml api push` still builds the document whole: it sends a body
  rather than writes a file, so `to_json`/`from_json` stay as they are.
- The four tests that drive a whole document run at once again, as the
  harness would have them: 3.4 GB together, where they used to ask for
  27 and be killed for it.

### Housekeeping

- Three lines nothing executed, which the coverage gate had not been
  able to say so about: the gate runs after the measurement, and the
  measurement was the job being killed. An `assert!` evaluates its
  message only when it fails, so a sum written both as the condition and
  as the message is a line that never runs while the test passes -- it
  is named once now and said once. A `sysml/files` this server cannot
  read, and a buffer that is no file, are both answered in a test.
- The four tests that drive a whole interchange document take it in
  turns and share the one document. A document that carries the library
  is 754 MB, `export` peaks around 6.7 GB writing it and `import` around
  6.9 GB reading it back, and the test harness runs as many at once as
  the machine has cores -- so four asked for 27 GB, which is more than a
  CI runner has. What a runner does about that is kill the process, so
  two jobs came back signalled with no test having failed and nothing
  said about why. One at a time is 6.8 GB and 760 MB on disk.

- `sysml rustgen` writes its `#![allow(...)]` header the way `rustfmt`
  would, so the first thing it generates is no longer the first thing
  that comes back as a diff.
- `tools/render` and `examples/arduino-uno` are held to `cargo fmt` and
  `cargo clippy` in CI. Both sit outside the workspace, so neither gate
  had ever read them, and both were failing `fmt`.
- The `sysml` tool is built for five platforms on each `v*` tag and
  attached to the release with checksums. `cargo install` wants a
  toolchain, and the people this is for are systems engineers.
- `cargo deny check` runs weekly and on any change to what is depended
  on: advisories, yanked crates, and the licence of everything in the
  tree. What it answers changes without anything here changing, which is
  why it has a schedule of its own. `deny.toml` names each licence and
  what is under it — EPL-2.0 scoped to `sysmlv2-stdlib` alone, since
  carrying the OMG library is the reason that crate exists.

## Earlier

The versions on crates.io before this were cut from `main` without a file
like this one. What each number moved for is written beside it in
`Cargo.toml` — the workspace manifest for the dependency floors, each
crate's own for its version — and the commit that moved it says the rest.

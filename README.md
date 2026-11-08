# sysml-v2-rs

Rust libraries for [SysML v2](https://www.omg.org/sysml/sysmlv2/) — the OMG
systems modeling language, adopted in 2025, built on KerML.

**[Try it in a browser](https://haradama.github.io/sysml-v2-rs/)** —
type a model, watch it drawn, share the link. No install, no server, no
upload: the whole toolchain is a WebAssembly module in the page.

The toolchain covers the textual notation end to end: an error-tolerant
parser with a lossless syntax tree, an element model generated from the
official metamodel, name resolution, the specification's well-formedness
constraints, standard JSON interchange, SVG diagrams, a formatter, a
language server, a REST API client, and a CLI.

**All 403 files of the official
[SysML-v2-Release](https://github.com/Systems-Modeling/SysML-v2-Release)
corpus parse, and every reference in them resolves** — the standard
libraries on their own (17411/17411), with the SysML examples
(25057/25057), and the KerML examples beside them (19146/19146). That
counts the operands of `connect`, `bind`, `allocate`, `first ... then`
and `satisfy ... by`, what an `alias` is `for`, the path each `import`
writes, and the names inside expressions, alongside every typing and
specialization.

## Install

A built `sysml` for Linux, macOS and Windows is attached to each
[release](https://github.com/haradama/sysml-v2-rs/releases), with the
checksums beside it. Unpack it and put the binary on your `PATH`; nothing
else is needed. From source:

```sh
cargo install sysmlv2-cli
```

The standard library is built into the binary, so this resolves models
rather than asking for a path first. The corpus tests read the example
models, which arrive as a submodule:

```sh
git submodule update --init --depth 1
```

## Usage

```console
$ sysml check model/
error: `Wheeel` resolves to nothing
 --> model/car.sysml:5:18
  |
5 |         part w : Wheeel;
  |                  ^^^^^^
  |
help: did you mean `Cars::Wheel`?
```

`check` parses, resolves every name and puts the specification's own
constraints to what is left. A finding is drawn the way rustc draws one,
and `--format json` hands a program the same span, plus what an
unresolved name might have meant: the import it wants, or the name it was
nearly spelt as.

| Command | |
| --- | --- |
| `sysml parse <files>` | does it parse (`--tree` dumps the syntax tree) |
| `sysml check <paths>` | parse, resolve, and check the specification's constraints |
| `sysml fmt <files>` | format (four spaces, idempotent; `--width` says where a line gives way, 0 nowhere) |
| `sysml diagram <file>` | SVG: definitions, `--internal`, `--sequence`, `--browser`, `--skin` |
| `sysml plan <paths>` | what the model implies for code, in no language in particular (`--schema` for what a generator reads) |
| `sysml codegen --lang <lang> <paths>` | code in any language: the generator for the language, found on `PATH`, handed the plan (`rust` is the one built in) |
| `sysml export <files>` | standard JSON interchange |
| `sysml import <json>` | the same read back: what it holds, or `--diagram` to see it |
| `sysml api …` | the SysML v2 API & Services REST client |
| `sysml import-rust <json>` | a Rust crate's API as a SysML package (`--surface` for the JSON it is read as) |
| `sysml import-code <inputs>` | code in any language as a SysML package: a surface JSON, or the importer for the language, found on `PATH` |
| `sysml rustgen <files>` | the Rust a model implies |
| `sysml mcp` | Model Context Protocol server, over stdio |

`stats` and `corpus` count what a file or a tree of them holds, and how
much of it resolves.

As a library:

```rust
let parse = sysml_syntax::parse("part def Vehicle { attribute mass : Real; }");
assert!(parse.ok());

let mut ws = sysml_semantics::Workspace::new();
ws.add_file("vehicle.sysml", "part def Engine :> PowerSource;\n");
ws.resolve_all();
```

## Crates

Published as `sysmlv2-*`, one directory up from what the directories
are called: `sysml-model` and `sysml-cli` on crates.io are other
people's crates. Each keeps its library name, so `use sysml_model::…`
is what the code reads either way.

| Crate | | |
| --- | --- | --- |
| [`sysmlv2-syntax`](crates/sysml-syntax) | Lexer, recursive-descent parser, lossless CST and typed AST. Parsing never fails: bad input still reproduces the source, with diagnostics beside it | [![crates.io](https://img.shields.io/crates/v/sysmlv2-syntax?style=flat-square&color=fc8d62&label=)](https://crates.io/crates/sysmlv2-syntax) [![docs.rs](https://img.shields.io/docsrs/sysmlv2-syntax?style=flat-square&color=66c2a5&label=docs)](https://docs.rs/sysmlv2-syntax) |
| [`sysmlv2-model`](crates/sysml-model) | The abstract syntax: 175 metaclasses generated from the OMG metamodel, arena storage, and the builder that turns an AST into one | [![crates.io](https://img.shields.io/crates/v/sysmlv2-model?style=flat-square&color=fc8d62&label=)](https://crates.io/crates/sysmlv2-model) [![docs.rs](https://img.shields.io/docsrs/sysmlv2-model?style=flat-square&color=66c2a5&label=docs)](https://docs.rs/sysmlv2-model) |
| [`sysmlv2-semantics`](crates/sysml-semantics) | Name resolution — imports, aliases, inheritance, implicit library specializations, connector ends, the names inside expressions — and the 180 constraints the specification states in OCL, evaluated. Three answers rather than two: what cannot be checked is *not evaluated* rather than a violation | [![crates.io](https://img.shields.io/crates/v/sysmlv2-semantics?style=flat-square&color=fc8d62&label=)](https://crates.io/crates/sysmlv2-semantics) [![docs.rs](https://img.shields.io/docsrs/sysmlv2-semantics?style=flat-square&color=66c2a5&label=docs)](https://docs.rs/sysmlv2-semantics) |
| [`sysmlv2-interchange`](crates/sysml-interchange) | Standard JSON: every metaclass's full property set, derived ownership and naming, reified memberships, deterministic UUIDs. Round-trip tested over the whole library | [![crates.io](https://img.shields.io/crates/v/sysmlv2-interchange?style=flat-square&color=fc8d62&label=)](https://crates.io/crates/sysmlv2-interchange) [![docs.rs](https://img.shields.io/docsrs/sysmlv2-interchange?style=flat-square&color=66c2a5&label=docs)](https://docs.rs/sysmlv2-interchange) |
| [`sysmlv2-diagram`](crates/sysml-diagram) | Diagrams in the standard's own notation, laid out by the Eclipse Layout Kernel. [What is drawn](crates/sysml-diagram#what-is-drawn) | [![crates.io](https://img.shields.io/crates/v/sysmlv2-diagram?style=flat-square&color=fc8d62&label=)](https://crates.io/crates/sysmlv2-diagram) [![docs.rs](https://img.shields.io/docsrs/sysmlv2-diagram?style=flat-square&color=66c2a5&label=docs)](https://docs.rs/sysmlv2-diagram) |
| [`sysmlv2-rust`](crates/sysml-rust) | Both directions between Rust and a model: a crate's API imported as SysML, and the Rust a model implies — with a list of what it left for a person to write. Rust is the language that ships; the seam it goes through, a `surface` in and a `plan` out, is stated as data so that any other language is yours to add | [![crates.io](https://img.shields.io/crates/v/sysmlv2-rust?style=flat-square&color=fc8d62&label=)](https://crates.io/crates/sysmlv2-rust) [![docs.rs](https://img.shields.io/docsrs/sysmlv2-rust?style=flat-square&color=66c2a5&label=docs)](https://docs.rs/sysmlv2-rust) |
| [`sysmlv2-lsp`](crates/sysml-lsp) | Language server: diagnostics, navigation, rename, completion, hover, symbols, formatting, and a live diagram preview | [![crates.io](https://img.shields.io/crates/v/sysmlv2-lsp?style=flat-square&color=fc8d62&label=)](https://crates.io/crates/sysmlv2-lsp) [![docs.rs](https://img.shields.io/docsrs/sysmlv2-lsp?style=flat-square&color=66c2a5&label=docs)](https://docs.rs/sysmlv2-lsp) |
| [`sysmlv2-cli`](crates/sysml-cli) | The `sysml` tool, including the MCP server | [![crates.io](https://img.shields.io/crates/v/sysmlv2-cli?style=flat-square&color=fc8d62&label=)](https://crates.io/crates/sysmlv2-cli) [![docs.rs](https://img.shields.io/docsrs/sysmlv2-cli?style=flat-square&color=66c2a5&label=docs)](https://docs.rs/sysmlv2-cli) |
| [`sysmlv2-stdlib`](crates/sysml-stdlib) | The KerML and SysML v2 standard libraries as text, built into every binary that resolves names (EPL-2.0) | [![crates.io](https://img.shields.io/crates/v/sysmlv2-stdlib?style=flat-square&color=fc8d62&label=)](https://crates.io/crates/sysmlv2-stdlib) [![docs.rs](https://img.shields.io/docsrs/sysmlv2-stdlib?style=flat-square&color=66c2a5&label=docs)](https://docs.rs/sysmlv2-stdlib) |

## For an AI agent

SysML v2 is new enough that a language model writes something that looks
right and names things that do not exist. `sysml mcp` answers the
questions it should have asked instead: does this parse and resolve, what
names are legal here, what does the library actually declare — and, where
the name is the thing it is missing, what the library *says* it means.
`notation` shows how each construct is written, with an example this
toolchain has checked; `generation_plan` states what a model implies for
code so an agent can write a language this toolchain has never heard of,
and `import_code` takes the same seam the other way: what an agent can
see of code in any language, stated as a surface, comes back as a
checked package.

```json
{ "mcpServers": { "sysml": { "command": "sysml", "args": ["mcp"] } } }
```

## Any language, both ways

Rust is the language that ships: `sysml import-rust` reads a crate
through rustdoc and `sysml rustgen` writes Rust. Every other language
is yours to add, and comes and goes the way Doxygen takes one it has
never heard of: through a program that writes what the tool already
reads, or reads what it already writes. Both are found by name on
`PATH`, the way `cargo` and `git` find a subcommand, and both halves of
the seam live in [`sysmlv2-rust`](crates/sysml-rust), stated as data in
no language in particular.

**In.** A *surface* is what some code declares — its records,
enumerations, interfaces and functions — as JSON, in no language in
particular, and `sysml import-code` is the one writer that turns any
surface into a package whose every definition carries a `@code`
binding. Which names are quoted, how two members that want one name are
told apart, what is refused for naming a type nothing declares: decided
once, there, and not by each importer again. An importer may write the
surface or the SysML itself; either way the package is parsed and
resolved against the standard library before it is written out, so an
importer that got a name wrong is told here rather than by the first
model that imports what it wrote.

```sh
sysml import-code inventory.py          # runs sysml-import-py from PATH
sysml import-code --lang python src/    # runs sysml-import-python
sysml import-code surface.json          # reads a surface a tool wrote
sysml import-code --schema              # what a surface is
```

**Out.** The *plan* is what a model implies for code — every
definition's shape, its features with what they bottom out in and how
many of them there are, a state machine's transitions and the state it
starts in — as JSON, in no language in particular, and `sysml codegen`
hands it to the generator for a language on standard input and takes
what comes back as the code. A model that does not resolve is refused
before any generator runs.

```sh
sysml codegen --lang python model/      # runs sysml-gen-python
sysml codegen --lang rust model/        # the generator built in
sysml plan --schema                     # what a generator reads
```

[`examples/import-python`](examples/import-python) and
[`examples/generate-python`](examples/generate-python) are one of each,
in a few hundred lines of Python; `sysml import-rust --surface` shows
the surface the Rust importer reads off a real crate, and `sysml
--format json plan` the plan a generator is handed.

## Playground

[`web`](web) is the toolchain as a page:
[haradama.github.io/sysml-v2-rs](https://haradama.github.io/sysml-v2-rs/).
A model on the left, the diagram of it on the right, redrawn as you type
— the PlantUML web server's arrangement, without the server. The parser,
name resolution, the constraints, the layout and the renderer are the
same `.wasm` the extension ships, running in the reader's tab, so nothing
is uploaded and the whole site is five static files.

The address bar carries the model, so a link is the whole thing a reader
needs; a fragment is never sent, so sharing one is between the two people
holding it.

```sh
make web-serve   # http://localhost:8000
```

## VSCode

[`editors/vscode`](editors/vscode) packages the language server:
as-you-type diagnostics, completion, navigation, rename, hover, symbols,
formatting, highlighting generated from the lexer's own keyword table,
and a live diagram preview that follows unsaved edits.

```sh
make vscode   # build, bundle, package, install
```

The whole workspace is the model: every `.sysml`/`.kerml` file under the
open folders resolves against every other, so a file that imports a
sibling is not reported as broken for having been opened alone. The
server and the standard library travel inside the package, so nothing
needs configuring.

The server it ships is [`sysmlv2-wasm`](crates/sysml-wasm) — the same
language server, compiled to WebAssembly and run in a worker. One
package for Linux, macOS and Windows, and the same one in a browser, at
vscode.dev or github.dev, where there is no machine under the editor to
run a program on. The module holds the parser, the metamodel, name
resolution, the constraints, the diagrams and the standard library:
6.3 MiB, 1.7 MiB over the wire, half a second to load a library and
single-figure milliseconds to answer a keystroke.

## Development

```sh
cargo test
cargo clippy --all-targets --all-features
markdownlint-cli2
cargo deny check
```

CI runs those, the corpus regressions and a line-coverage gate;
`.github/workflows/ci.yml` is the list, and it can be run locally with
[act](https://github.com/nektos/act).

`cargo deny check` is on its own schedule as well, because what it
answers changes without anything here changing: an advisory is published
against a dependency, and a licence arrives through a dependency of a
dependency. [`deny.toml`](deny.toml) says which licences are allowed and
why each is on the list.

[`tools/render`](tools/render) rasterizes an SVG so a drawing can be
looked at in a review. It sits outside the workspace on purpose: it
exists to look at the output, not to be part of it.

## License

Copyright © 2026 sysml-v2-rs contributors. MIT or Apache-2.0, at your
option — except for two parts that are not this project's to license,
each of which says so beside itself:

- [`crates/sysml-stdlib/library/`](crates/sysml-stdlib) is the standard
  model libraries, taken unchanged from the OMG SysML v2 Release under
  the **Eclipse Public License 2.0**. That crate is separate for this
  reason and carries the licence and a `NOTICE`; anything bundling it —
  the `.vsix`, a published binary — carries them too.
- `crates/sysml-model/src/generated.rs` carries the OCL of each
  constraint the specification states and the sentence it states in
  words, reproduced from the OMG normative metamodel under the terms of
  use the OMG grants with its specifications.
  [`crates/sysml-model/NOTICE`](crates/sysml-model/NOTICE) says which
  parts.

OMG®, SysML®, and Systems Modeling Language® are registered trademarks of
the Object Management Group. This project is not affiliated with or
endorsed by the OMG.

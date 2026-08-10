# sysml-v2-rs

Rust libraries for [SysML v2](https://www.omg.org/sysml/sysmlv2/) — the OMG
systems modeling language (adopted 2025) built on KerML.

The toolchain covers the textual notation end to end: an error-tolerant
parser with a lossless syntax tree, an in-memory element model generated
from the official metamodel, name resolution, standard JSON interchange,
SVG diagram rendering, a REST API client, a formatter, a language server,
and a CLI. Both are validated against the official
[SysML-v2-Release](https://github.com/Systems-Modeling/SysML-v2-Release)
corpus — all 403 `.sysml`/`.kerml` files: the complete standard libraries
(`sysml.library`) and every official example, training and validation
model. **All 403 files parse cleanly and every reference in any of them
resolves**: the standard library on its own (12757/12757), the library
together with all official SysML examples (17700/17700), and the KerML
examples alongside it (13794/13794), counting the operands of `connect`,
`bind`, `allocate`, `first ... then ...` and `satisfy ... by ...`
alongside every typing and specialization.

Eleven of those used to resolve to themselves. A feature that declares
no name of its own answers to the name of what it redefines -- which is
the very name being looked up while that redefinition is resolved -- so
`attribute :>> nothingHere;` was reported as sound. Stopping that showed
what had never been found, and each was a rule this resolver was
missing: a name reached through `'$'` rather than the root `$`, a
redefining member losing to a general one because its supertype was
written second, `include x[0..*]` reading its multiplicity as an index,
`render asElementTable` reading the rendering as a name, an `objective`
standing for the one its type declares, a feature redeclared by naming
it the same, a `baseType` chosen by a condition, and `variant x;`
naming a usage the model already has.

To run the corpus tests, fetch the submodule first:

```sh
git submodule update --init --depth 1
cargo run -p sysml-cli -- corpus vendor/sysml-v2-release/sysml.library
```

## Crates

| Crate | Description |
| --- | --- |
| [`sysml-syntax`](crates/sysml-syntax) | Lexer (logos) + recursive-descent parser + lossless CST (rowan) + typed AST |
| [`sysml-model`](crates/sysml-model) | Element model: 175 metaclasses generated from the official Ecore metamodel, arena storage, AST→model builder |
| [`sysml-semantics`](crates/sysml-semantics) | Name resolution (imports, aliases, inheritance, implicit library specializations, connector ends), relationship reification and implied-relationship materialization — the whole standard library resolves |
| [`sysml-interchange`](crates/sysml-interchange) | Standard JSON interchange: the complete property set of every metaclass, derived ownership/naming/inheritance-closure/import properties, reified memberships down to `ParameterMembership`/`SubjectMembership`/`StateSubactionMembership`/... with visibility and kind, deterministic UUIDs; resolved whole-library round-trip tested |
| [`sysml-diagram`](crates/sysml-diagram) | Definition/specialization diagrams: layered layout and SVG rendering with no external engine, or PlantUML-style Graphviz layout (`dot` for positions, the drawing stays ours) |
| [`sysml-rust`](crates/sysml-rust) | The Rust side of a model, both ways. `import` reads an existing crate's rustdoc JSON as a SysML package whose definitions carry `@rust` binding metadata; `generate` writes Rust from a resolved model: definitions become structs/enums (multiplicities as containers, declared values as `Default`, inheritance flattened, cycles boxed), calculations become functions and methods with simple result expressions translated, `abstract` calculations and action definitions become traits, an action whose dataflow the model wired completely becomes the body that performs it, state definitions become state machines (guards translated where they read the event payload), API-bound ports become generics and `perform`ed actions delegating methods |
| [`sysml-lsp`](crates/sysml-lsp) | Language server: diagnostics, go-to-definition, find-references, rename, completion, hover, symbols, formatting — with a [VSCode extension](editors/vscode) as its client |
| [`sysml-mcp`](crates/sysml-mcp) | Model Context Protocol server: lets an AI agent ask whether a model parses and resolves, what names are legal at a point, and what the standard library actually declares |
| [`sysml-codegen`](crates/sysml-codegen) | Generates `sysml-model`'s metamodel code from [`vendor/metamodel`](vendor/metamodel) |
| [`sysml-cli`](crates/sysml-cli) | `sysml` command-line tool (`parse`, `fmt`, `check`, `stats`, `export`, `diagram`, `import-rust`, `rustgen`, `api`, `corpus`); `api` is the client for the SysML v2 API & Services REST standard, and `api push` sends what `export` writes |

## Usage

```console
$ cargo run -p sysml-cli -- parse examples/vehicle.sysml
examples/vehicle.sysml: ok (0 error(s))

$ cargo run -p sysml-cli -- parse --tree examples/vehicle.sysml   # dump the syntax tree

$ cargo run -p sysml-cli -- diagram examples/vehicle.sysml -o vehicle.svg
wrote 5 box(es), 1 specialization(s), 2 composition(s), 0 connection(s), 0 transition(s) and 0 satisfaction(s) to vehicle.svg
```

`diagram` draws one box per definition — its keyword, name and the features
it declares — and two kinds of edge between them: a hollow triangle pointing
from each subtype at the supertype it specializes (`part def Engine :>
PowerSource`), and a filled diamond on the whole of each composition
(`part def Vehicle { part eng : Engine; }`). An `abstract` definition has
its name set in italic, the UML way. Only specializations order the layers,
since a whole is not a subtype of its parts — but a whole and its parts are
kept side by side within a layer, so an unrelated definition declared
between them does not come between them on the canvas. A line that would
otherwise pass under an unrelated box — and so read as a connection to that
box — steps around it instead: it leaves both boxes squarely and crosses in
one of the clear gaps between the rows, or, when no single gap reaches, goes
round whole rows by way of a column nothing is drawn in. Every route is
checked against the boxes it would actually cross before it is drawn, and a
straight line is kept rather than a detour that crosses anyway. The drawing
follows the specification's own notation figures (`bnf/images` in the
release): black ink on white in Arial, definitions square-cornered, usages
rounded, abstract names in italic. Layering, crossing reduction and the SVG
itself are produced in-process, so the output is deterministic and needs no
renderer beyond a browser.

`--internal <NAME>` switches to the internal structure of one definition:
a box per part, state or action it is composed of, and an edge per
statement relating two of them. A part box lists the features its type
declares, since `part w : Wheel;` writes none of its own -- so the ports a
connection can attach to are visible whether or not anything connects to
them -- and holds the parts it is itself assembled from as boxes drawn
inside it, one level deep. The same flag therefore serves a `part def`
(parts wired by `connect`), a `state def` (states linked by `transition`)
and an `action def` (actions sequenced by `first ... then`); connections are
plain lines, transitions carry an open arrowhead labelled the UML way --
name, the payload they wait for, the condition guarding them and the action
they perform (`subscribing accept sub : Subscribe [ready] / send action`) --
and an `entry; then x;` succession is drawn from the filled circle the
machine starts at. Port names and transition labels sit in the gap between
the boxes they belong to, and only that gap is widened to hold them, so one
long label does not push the rest of the row apart with it.

```console
$ cargo run -p sysml-cli -- diagram car.sysml --internal Car -o car.svg
wrote 3 box(es), 0 specialization(s), 0 composition(s), 2 connection(s), 0 transition(s) and 0 satisfaction(s) to car.svg
```

Requirements are drawn too: `satisfy r by p;` becomes a dashed dependency
from the satisfying feature to the requirement, and an n-ary
`connection { end ::> a; end ::> b; end ::> c; }` -- how a derivation is
written -- fans out from the end written first.

Connector ends are matched by the feature chain name resolution records, so
`connect w.hub to a.mount` links the boxes for `w` and `a` even when several
parts share one type. Each end is drawn the SysML way -- a small square
straddling the box border -- with the port's name beside it, and the two
names of one connection land on opposite sides of the line. Connections
sharing a pair of boxes are spread apart so they stay separate lines, closing
up when there are more of them than the borders have room for, and the gap
between boxes widens to fit the names drawn in it. Connections reaching outside the definition, and those between
two features of the same part, are left undrawn.

`--browser` draws the other standard view that needs nothing but the model
itself: the membership hierarchy, as an indented tree with one row per named
element.

```console
$ cargo run -p sysml-cli -- diagram examples/vehicle.sysml --browser -o tree.svg
wrote 14 row(s) to tree.svg
```

A feature's type is labelled only once it resolves. `--library` loads
supporting models for name resolution without drawing them, which is how a
model gets its library types labelled while staying a diagram of its own
definitions:

```console
$ cargo run -p sysml-cli -- diagram examples/vehicle.sysml \
    --library vendor/sysml-v2-release/sysml.library -o vehicle.svg
wrote 5 box(es), 1 specialization(s), 2 composition(s), 0 connection(s), 0 transition(s) and 0 satisfaction(s) to vehicle.svg
```

`--graphviz` hands the positions to Graphviz `dot` the way PlantUML does --
the boxes, edges and labels are still drawn here, in the same style, so only
the arrangement changes. It needs Graphviz installed (`--dot` names the
command) and trades the built-in layout's reproducible bytes for `dot`'s
crossing minimization:

```console
$ cargo run -p sysml-cli -- diagram examples/vehicle.sysml --graphviz -o vehicle.svg
```

Without it the compartment reads `attribute mass`; with it, `attribute mass
: Real = 1200.0` -- a declared multiplicity and a declared value travel with
the feature (`part wheels : Wheel[4]`), since the model keeps them as the
`MultiplicityRange` and `FeatureValue` elements the standard stores. Either
way the diagram keeps its five boxes rather than gaining the library's
1337.

A layer wider than `Style::max_row_width` (1600 px) wraps onto further rows,
so a model with many unrelated definitions stays a readable page instead of
one very long strip.

As a library:

```rust
let parse = sysml_syntax::parse("part def Vehicle { attribute mass : Real; }");
assert!(parse.ok());
let file = sysml_syntax::ast::SourceFile::cast(parse.syntax()).unwrap();
```

### Importing an existing Rust API

`import-rust` turns a crate's public API into a SysML package, so a system
model can type its ports with the crate's traits and `perform` its
functions -- and so name resolution catches the model drifting from the
API. Every definition carries a `@rust { ... }` metadata usage naming the
Rust item it binds to, which is what a code generator calls instead of
inventing parallel types:

```console
$ cd that-crate && cargo +nightly rustdoc -- -Zunstable-options --output-format json
$ sysml import-rust that-crate/target/doc/that_crate.json -o ThatCrateApi.sysml
wrote 12 definition(s) to ThatCrateApi.sysml
```

Structs become `item def`s (with `Vec`/`Option` as multiplicities), plain
enums `enum def`s, traits `port def`s with an `action def` per method
(`in` parameters, `out result`, an `out error [0..1]` for `Result`), free
functions `action def`s. Doc comments travel along. What has no
monomorphic SysML shape -- generics, tuple structs, data-carrying enum
variants -- is listed at the end of the package rather than dropped
silently. Regenerating in CI and diffing detects API drift.

### The round trip, end to end

[`examples/order-system`](examples/order-system) closes the loop:

```console
$ make demo
...
SKU-042: 7 in stock (threshold 10) -> replenish
```

1. `sysml import-rust` turns the in-house crate's rustdoc JSON into
   `model/InventoryStoreApi.sysml`
2. `model/order_system.sysml` -- the hand-written system model -- imports
   it, types a port with the API's trait and `perform`s its actions
3. `sysml check` proves every reference resolves (an API change that
   breaks the model fails right here)
4. `sysml rustgen` writes `src/generated.rs`: the part as a struct generic
   over the real `inventory_store::InventoryStore` trait, each performed
   action a method with the API's own signature (`async`, `Result` and
   receivers included), the model's `calc def` as a plain function and
   the state machine's guard translated into its hook's default body
5. `cargo run` compiles the generated code against the real crate and runs
   it with a stub implementation

A test regenerates every stage and holds it equal to what is checked in,
so drift anywhere in the chain fails the build.

### A behaviour the model wires up

[`examples/riscv`](examples/riscv) goes the other way: nothing existing
to import, a model written first. Its `action def Step` says what one
turn of a RISC-V instruction cycle is made of and how the pieces are
wired -- fetch, then decode, then execute, with the word and the
instruction handed along -- and that wiring is the generated body, with
the three parts as supertrait bounds. The hand-written half implements
only the parts.

```console
$ cd examples/riscv && cargo run
x1=7 x2=5 x3=12 mem[16]=12
```

Where a dataflow is short of something -- an input nothing feeds, a
result nothing produces -- no body is written and the generated
documentation names the gap, rather than guessing at it.

### VSCode

[`editors/vscode`](editors/vscode) packages the language server for Visual
Studio Code: as-you-type syntax and name-resolution diagnostics, completion,
navigation, rename, hover, symbols, formatting, TextMate highlighting
generated from the lexer's own keyword table, and a live diagram preview
(definitions, internal structure or membership tree) served by the language
server over a custom `sysml/diagram` request, so it follows unsaved edits.
The preview asks Graphviz for the layout when `dot` is installed
(`sysml.diagram.layout`, `sysml.diagram.dot`) and falls back to the built-in
layout when it is not.

```sh
make vscode   # build server, bundle it with the standard library, package, install
```

The packaged extension carries the server and `sysml.library` inside it, so
nothing needs configuring; `sysml.server.path` / `sysml.library.path`
override the bundled copies when set. For extension development,
`cd editors/vscode && npm install && npm run compile` and press F5.

## Development

```sh
cargo test
cargo clippy --all-targets
markdownlint-cli2            # lint the Markdown files (config: .markdownlint.yaml)
```

Diagrams ship as SVG, which needs a viewer to judge. [`tools/render`](tools/render)
rasterizes one so it can be looked at -- in a review, or by a tool that reads
images but not SVG:

```sh
cargo run -p sysml-cli -- diagram examples/vehicle.sysml -o vehicle.svg
cargo run --manifest-path tools/render/Cargo.toml -- vehicle.svg vehicle.png 2
```

It sits outside the workspace on purpose: it exists to look at the output,
not to be part of it, and it pulls in a rasterizer the published crates have
no business depending on.

`crates/sysml-diagram/tests/corpus_faithfulness.rs` checks the other half:
over every corpus model it asserts that each box stands for an element in
scope, that nothing is drawn twice or lost, that every line ends on a box,
and that the specializations drawn are exactly the ones the model holds --
worked out from the model rather than from the drawing code.

CI (`.github/workflows/ci.yml`) runs markdownlint, rustfmt, clippy, the
test suite (including the corpus regressions) and the C0-coverage gate.
The workflow can be run locally with [act](https://github.com/nektos/act)
(defaults live in `.actrc`):

```sh
act -l                       # list jobs
act -j markdownlint          # run a single job
act                          # run everything (the rust jobs take a while)
```

With a non-standard Docker socket (Rancher Desktop, colima, ...) point act
at it first: `export DOCKER_HOST=$(docker context inspect --format
'{{.Endpoints.docker.Host}}')`.

## License

Copyright © 2026 sysml-v2-rs contributors.

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or
[MIT License](LICENSE-MIT), at your option.

`crates/sysml-model/src/generated.rs` is generated from the normative
machine-readable metamodel files published by the OMG with the KerML 1.0
and SysML 2.0 specifications, whose terms of use permit creating and
distributing software based on the specifications — see
[vendor/metamodel/README.md](vendor/metamodel/README.md) for provenance.

OMG®, SysML®, and Systems Modeling Language® are registered trademarks of
the Object Management Group. This project is not affiliated with or
endorsed by the OMG.

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
resolves**: the standard library on its own (16173/16173), the library
together with all official SysML examples (22611/22611), and the KerML
examples alongside it (17323/17323), counting the operands of `connect`,
`bind`, `allocate`, `first ... then ...` and `satisfy ... by ...`, and
the names written inside expressions -- a constraint body, the result of
a `calc`, the value after `=` -- alongside every typing and
specialization.

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
| [`sysml-model`](crates/sysml-model) | Element model: 175 metaclasses generated from the official Ecore metamodel, arena storage, AST→model builder; the generator that writes it from [`vendor/metamodel`](vendor/metamodel) is in the same crate behind the `codegen` feature |
| [`sysml-semantics`](crates/sysml-semantics) | Name resolution (imports, aliases, inheritance, implicit library specializations, connector ends, the names inside expressions), relationship reification and implied-relationship materialization — the whole standard library resolves |
| [`sysml-interchange`](crates/sysml-interchange) | Standard JSON interchange: the complete property set of every metaclass, derived ownership/naming/inheritance-closure/import properties, reified memberships down to `ParameterMembership`/`SubjectMembership`/`StateSubactionMembership`/... with visibility and kind, deterministic UUIDs; resolved whole-library round-trip tested |
| [`sysml-diagram`](crates/sysml-diagram) | Definition/interconnection diagrams in the standard's own notation — labelled compartment stacks, ports on the border, composite and reference memberships told apart by the diamond the specification draws — laid out here or by the Eclipse Layout Kernel (`elkrs` for the arrangement and the routes) |
| [`sysml-rust`](crates/sysml-rust) | The Rust side of a model, both ways. `import` reads an existing crate's rustdoc JSON as a SysML package whose definitions carry `@rust` binding metadata; `generate` writes Rust from a resolved model: definitions become structs/enums (multiplicities as containers, declared values as `Default`, inheritance flattened, cycles boxed), calculations become functions and methods with simple result expressions translated, `abstract` calculations and action definitions become traits, an action whose dataflow the model wired completely becomes the body that performs it, state definitions become state machines (guards translated where they read the event payload), API-bound ports become generics and `perform`ed actions delegating methods |
| [`sysml-lsp`](crates/sysml-lsp) | Language server: diagnostics, go-to-definition, find-references, rename, completion, hover, symbols, formatting — with a [VSCode extension](editors/vscode) as its client |
| [`sysml-cli`](crates/sysml-cli) | `sysml` command-line tool (`parse`, `fmt`, `check`, `stats`, `export`, `diagram`, `import-rust`, `rustgen`, `api`, `mcp`, `corpus`); `mcp` speaks the Model Context Protocol over stdio, so an AI agent can ask whether a model parses and resolves, what names are legal at a point, and what the standard library actually declares; `api` is the client for the SysML v2 API & Services REST standard, and `api push` sends what `export` writes |

## Usage

```console
$ cargo run -p sysml-cli -- parse vehicle.sysml
vehicle.sysml: ok (0 error(s))

$ cargo run -p sysml-cli -- parse --tree vehicle.sysml   # dump the syntax tree

$ cargo run -p sysml-cli -- diagram vehicle.sysml -o vehicle.svg
wrote 5 box(es), 1 specialization(s), 2 composition(s), 0 reference(s), 0 subsetting(s), 0 connection(s), 0 flow(s), 0 allocation(s), 0 transition(s), 0 dependency(ies) and 0 satisfaction(s) to vehicle.svg
```

`diagram` draws one box per definition — its keyword, name and the features
it declares — and two kinds of edge between them: a hollow triangle pointing
from each subtype at the supertype it specializes (`part def Engine :>
PowerSource`), and a filled diamond on the whole of each composition
(`part def Vehicle { part eng : Engine; }`). A portion is a composite
membership too, and the standard draws it differently -- a timeslice is
part of an occurrence in a way a wheel is not part of a car -- so
`snapshot s : O;` gets the filled marker `portion-relationship` carries
instead. An `abstract` definition has
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
them -- and holds what it is itself assembled from as boxes drawn inside
it, one level deep, along with what wires those together. That is the
standard's `interconnection-view` (`(interconnection-element)*`), and
the same view serves an action's `action-flow-compartment` and a state's
`state-transition-compartment`: a behaviour drawn as an empty frame is
one whose steps went unsaid. Whatever became a box inside is not listed
in a compartment as well. The same flag therefore serves a `part def`
(parts wired by `connect`), a `state def` (states linked by `transition`)
and an `action def` (actions sequenced by `first ... then`); connections are
plain lines, a succession is the dashed line `aflow-succession` draws --
a step following a step is not a machine changing state -- and transitions
carry an open arrowhead labelled the UML way --
name, the payload they wait for, the condition guarding them and the action
they perform (`subscribing accept sub : Subscribe [ready] / send action`) --
and an `entry; then x;` succession is drawn from the filled circle the
machine starts at. A control node is drawn as the glyph the standard gives
it rather than as a box, because a flow does not split at an action: `fork`
and `join` are bars, `merge` and `decide` diamonds, `terminate` a cross. An
`if` or a loop says what it asks -- `if-condition`, `while-condition`,
`for iterator` -- and holds its body as a flow drawn inside it, whether or
not the node was given a name of its own.
Port names and transition labels sit in the gap between
the boxes they belong to, and only that gap is widened to hold them, so one
long label does not push the rest of the row apart with it.

```console
$ cargo run -p sysml-cli -- diagram car.sysml --internal Car -o car.svg
wrote 3 box(es), 0 specialization(s), 0 composition(s), 0 reference(s), 0 subsetting(s), 2 connection(s), 0 flow(s), 0 allocation(s), 0 transition(s), 0 dependency(ies) and 0 satisfaction(s) to car.svg
```

What a definition answers for is drawn rather than only listed:
`satisfy r by p;`, `assert c;`, `assume constraint c;`, `require c;`,
`perform a;`, `exhibit s;` and `event occurrence ev;` each become the line
the standard gives them
-- a plain one with an open arrowhead, keyworded «satisfy»,
«assert» and so on. `dependency use from A to B;` is the one dashed
line in the notation, drawn from each client to each supplier. And an n-ary
`connection { end ::> a; end ::> b; end ::> c; }` -- how a derivation is
written -- meets at the dot the standard draws for it
(`n-ary-connection = n-ary-connection-dot n-ary-segment+`), with one
segment out to each end and the connection's own name beside the dot.
Each end is written the way `connection-graphical` writes it, `rolename
multiplicity c-adornment`: `bead [1] in ordered`, `rim redefines seat`.

The keyword above a box's name is the one the standard's name compartment
writes, which is not always the metaclass name spelled out: `«analysis
def»` rather than `«analysis case def»`, `«calc»`, `«enum»`, `«assign»`,
`«if»`, and `«loop»` for a loop of either kind. A state's entry, do and
exit actions say which of the three they are.

A two-ended statement is drawn the way its own production draws it, since
the standard gives each a different line: `bind a = b` is a plain line
written `=`, `interface i connect a to b` one keyworded `«interface»`,
`allocate a to b` an open arrowhead with `«allocate»`, a `flow` the filled
arrowhead and what it carries (`fuel of Fuel`), a `succession flow` the same
keyworded `«succession flow»`, and a `message` the open dart the standard
keeps for it.

Every port a box declares is drawn on its border, the way the standard has
it (`part-def = part-def-name-compartment interconnection-view
compartment-stack port-l* port-r* port-t* port-b*`): a small square
straddling the border with `name : Type` beside it, on the side facing
whatever it is connected to. Connector ends are matched by the feature chain
name resolution records, so `connect w.hub to a.mount` links the boxes for
`w` and `a` even when several parts share one type, and the line arrives at
the port rather than drawing a second square of its own. An action's
parameters go on its border the same way (`param-l | param-r | param-t |
param-b`), drawn rounded rather than square, and a port or parameter with
a declared direction carries the arrow the standard puts inside it: one
head for `in` or `out`, one at each end for `inout`. An end that reaches
the whole of a part has no rolename to write, so no square is drawn for it.
Connections
sharing a pair of boxes are spread apart so they stay separate lines, closing
up when there are more of them than the borders have room for, and the gap
between boxes widens to fit the names drawn in it. Connections reaching outside the definition, and those between
two features of the same part, are left undrawn.

What a definition relates to that is not on the canvas is said in words
rather than left unsaid: the standard's `relationships-compartment`
(`el-prefix? relationship-name QualifiedName`) holds `specializes
Integer` for a definition whose supertype is in the library and so has
no box to point at.

What a definition documents about itself is drawn: the standard's
`documentation-compartment` holds the prose of a `doc`, and a `rep` says
which language it is in and what it says in it. A line longer than the
box holds ends in the `…` the standard uses for exactly that.

A view says what it exposes and what it filters by:
`exposes-compartment` and `filters-compartment` hold what `expose
P::Thing;` and `filter @Safety;` wrote, neither of which is a feature and
neither of which was drawn at all.

`--sequence <NAME>` draws the interaction a definition declares as the
standard's sequence view: a head node per participant across the top, a
dashed lifeline under each, and one arrow per message or succession
between them. The participant a lifeline stands for is the feature chain
a message reaches through without the event it ends at, so
`vehicle.cruiseController.setSpeedReceived` belongs to
`vehicle.cruiseController`. An arrow has to reach an event, which is
what tells `first m1 then m2` -- an ordering of two messages -- from a
succession that joins two lifelines.

```console
$ cargo run -p sysml-cli -- diagram interaction.sysml --sequence CruiseControlInteraction -o seq.svg
wrote 4 lifeline(s) and 3 message(s) to seq.svg
```

`--browser` draws the other standard view that needs nothing but the model
itself: the membership hierarchy, as an indented tree with one row per named
element.

```console
$ cargo run -p sysml-cli -- diagram vehicle.sysml --browser -o tree.svg
wrote 14 row(s) to tree.svg
```

A feature's type is labelled only once it resolves. `--library` loads
supporting models for name resolution without drawing them, which is how a
model gets its library types labelled while staying a diagram of its own
definitions:

```console
$ cargo run -p sysml-cli -- diagram vehicle.sysml \
    --library vendor/sysml-v2-release/sysml.library -o vehicle.svg
wrote 5 box(es), 1 specialization(s), 2 composition(s), 0 reference(s), 0 subsetting(s), 0 connection(s), 0 flow(s), 0 allocation(s), 0 transition(s), 0 dependency(ies) and 0 satisfaction(s) to vehicle.svg
```

`--elk` hands the arrangement to the Eclipse Layout Kernel -- the boxes,
edges and labels are still drawn here, in the same style, so only where they
sit changes. ELK picks the positions and the orthogonal routes between them
together, and both are drawn; it trades the built-in layout's reproducible
bytes for ELK's crossing minimization, and needs one Rust binary on the path
(`--elk-command` names it):

```console
$ cargo install elkrs
$ cargo run -p sysml-cli -- diagram vehicle.sysml --elk -o vehicle.svg
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

### VSCode

[`editors/vscode`](editors/vscode) packages the language server for Visual
Studio Code: as-you-type syntax and name-resolution diagnostics, completion,
navigation, rename, hover, symbols, formatting, TextMate highlighting
generated from the lexer's own keyword table, and a live diagram preview
(definitions, internal structure or membership tree) served by the language
server over a custom `sysml/diagram` request, so it follows unsaved edits.
The preview asks ELK for the layout when `elkrs` is installed
(`sysml.diagram.layout`, `sysml.diagram.elk`) and falls back to the built-in
layout when it is not.

```sh
make vscode   # build server, bundle it with the standard library, package, install
```

The preview opens beside the editor when a model file is opened and
follows what you edit; closing it keeps it closed
(`sysml.preview.openAutomatically`).

The whole workspace is the model: every `.sysml`/`.kerml` file under the
open folders resolves against every other, whether or not it is in a tab,
so a file that imports a sibling is not reported as broken for having been
opened alone. `sysml.workspace.exclude` leaves out directories that are not
yours to edit -- a vendored corpus, someone else's model.

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
cargo run -p sysml-cli -- diagram vehicle.sysml -o vehicle.svg
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

# sysml-v2-rs

Rust libraries for [SysML v2](https://www.omg.org/sysml/sysmlv2/) — the OMG
systems modeling language (adopted 2025) built on KerML.

The toolchain covers the textual notation end to end: an error-tolerant
parser with a lossless syntax tree, an in-memory element model generated
from the official metamodel, name resolution, standard JSON interchange,
SVG diagram rendering, a REST API client, a formatter, a language server,
and a CLI. Parsing and
name resolution are validated against **100% of the official
[SysML-v2-Release](https://github.com/Systems-Modeling/SysML-v2-Release)
corpus** — all 403 `.sysml`/`.kerml` files: the complete standard libraries
(`sysml.library`) and every official example, training and validation
model.

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
| [`sysml-semantics`](crates/sysml-semantics) | Name resolution (imports, aliases, inheritance, implicit library specializations) and relationship reification — 100% of the standard library **and all official examples** resolve |
| [`sysml-interchange`](crates/sysml-interchange) | Standard JSON interchange with deterministic UUIDs; whole-library round-trip tested |
| [`sysml-diagram`](crates/sysml-diagram) | Definition/specialization diagrams: layered layout and SVG rendering, no external engine |
| [`sysml-api-client`](crates/sysml-api-client) | REST client for the SysML v2 API & Services standard (projects/commits/elements) |
| [`sysml-lsp`](crates/sysml-lsp) | Language server: diagnostics, go-to-definition, find-references, rename, completion, hover, symbols, formatting |
| [`sysml-codegen`](crates/sysml-codegen) | Generates `sysml-model`'s metamodel code from [`vendor/metamodel`](vendor/metamodel) |
| [`sysml-cli`](crates/sysml-cli) | `sysml` command-line tool (`parse`, `fmt`, `check`, `stats`, `export`, `diagram`, `corpus`) |

## Usage

```console
$ cargo run -p sysml-cli -- parse examples/vehicle.sysml
examples/vehicle.sysml: ok (0 error(s))

$ cargo run -p sysml-cli -- parse --tree examples/vehicle.sysml   # dump the syntax tree

$ cargo run -p sysml-cli -- diagram examples/vehicle.sysml -o vehicle.svg
wrote 5 definition(s) and 1 specialization(s) to vehicle.svg
```

`diagram` draws one box per definition — its keyword, name and the features
it declares — with a hollow triangle pointing from each subtype at the
supertype it specializes. Layering, crossing reduction and the SVG itself
are produced in-process, so the output is deterministic and needs no
renderer beyond a browser.

A feature's type is labelled only once it resolves. `--library` loads
supporting models for name resolution without drawing them, which is how a
model gets its library types labelled while staying a diagram of its own
definitions:

```console
$ cargo run -p sysml-cli -- diagram examples/vehicle.sysml \
    --library vendor/sysml-v2-release/sysml.library -o vehicle.svg
wrote 5 definition(s) and 1 specialization(s) to vehicle.svg
```

Without it the compartment reads `attribute mass`; with it, `attribute mass
: Real`. Either way the diagram keeps its five boxes rather than gaining the
library's 1337.

A layer wider than `Style::max_row_width` (1600 px) wraps onto further rows,
so a model with many unrelated definitions stays a readable page instead of
one very long strip.

As a library:

```rust
let parse = sysml_syntax::parse("part def Vehicle { attribute mass : Real; }");
assert!(parse.ok());
let file = sysml_syntax::ast::SourceFile::cast(parse.syntax()).unwrap();
```

## Development

```sh
cargo test
cargo clippy --all-targets
markdownlint-cli2            # lint the Markdown files (config: .markdownlint.yaml)
```

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

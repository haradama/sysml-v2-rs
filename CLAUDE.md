# Working in this repository

Rust libraries for SysML v2. `README.md` says what each crate is; this
file says what an agent working here needs to know that the code does not
say for itself.

## Do not guess at SysML — ask the toolchain

SysML v2 was adopted in 2025 and there is little of it in any model's
training data. Names, and which of them are visible where, are the thing
to get wrong. This repository can answer, so ask it rather than guessing:

```sh
sysml --format json parse <file>                       # does it parse
sysml --format json check <file> vendor/sysml-v2-release/sysml.library
```

`check` without the standard library reports every reference into it as
unresolved, which is a false alarm rather than a finding. Both commands
exit non-zero on a finding and answer in JSON, with each finding placed
by `offset`, `line` and `column` (lines and columns from one).

`.claude/hooks/check-sysml.sh` runs both after any edit to a `.sysml` or
`.kerml` file and hands what it finds straight back — so a model written
here is checked as it is written. It says nothing when the file is
clean.

The official corpus is the ground truth: all 403 files parse and every
reference in them resolves. If something in `vendor/sysml-v2-release`
disagrees with what you believe about the language, it is right.

## Generated code is not yours to edit

These are written by generators and are checked in. Change the input or
the generator, then regenerate — never the file:

| File | Written by |
| --- | --- |
| `crates/sysml-model/src/generated.rs` | `cargo run -p sysml-model --features codegen --bin sysml-codegen` (from `vendor/metamodel`) |

Each carries a `DO NOT EDIT` header, and a test regenerates it and holds
it equal to what is checked in, so an edit fails the build rather than
surviving.

What `sysml rustgen` leaves abstract becomes a **trait**, not a `todo!()`.
The implementation belongs next to the generated file, never in it.

## What must hold before you are done

CI gates all of these, and the coverage one is stricter than most:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --release --all-features
cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 99
cargo llvm-cov report --show-missing-lines     # must name no file at all
```

**Every line must have run.** Adding a branch means adding the test that
takes it. A branch that cannot be reached is removed by refactoring, not
covered by a contrived test — see the note in `DESIGN.md`. Generated code
is held to this too, which is why the metamodel generator emits tests alongside
the accessors it writes. It lives behind the `codegen` feature, so the
gates above pass `--all-features` to reach it.

## Prose

Comments and documentation explain why, in plain sentences, and are
written for a reader who knows Rust but not this codebase. Match the
surrounding density: this repository comments the reasoning behind a
decision, not the mechanics of the code under it.

## Things that are true and easy to get wrong

- `sysml fmt` uses four spaces.
- The standard library lives in a submodule
  (`git submodule update --init --depth 1`). Corpus-dependent tests
  detect its absence and skip rather than fail.
- `tools/render` is excluded from the workspace, so `cargo test` does not
  build it, and nothing else covers it.

# sysmlv2-stdlib

The KerML and SysML v2 standard model libraries, as text a program can
load.

Almost nothing in a SysML model resolves without these. `part def
Vehicle;` specializes `Parts::Part`, every feature subsets
`Base::things`, and a `calc` reaches into the Kernel Function Library —
so a toolchain that cannot find the standard library reports every name
in every model as unresolved, which is a lot of noise for a missing path.

Getting one used to mean cloning a repository whose history is two
gigabytes to obtain one and a third megabytes of model. Here they are
instead, in the binary:

```rust
let mut ws = sysml_semantics::Workspace::new();
for (name, text) in sysml_stdlib::FILES {
    ws.add_file(*name, text);
}
```

`FILES` pairs the path each file is known by, relative to the library's
own root, with what it says. `RELEASE` names the release they came from,
because the standard library moves with the specification and an answer
that cannot be reproduced is worth less than one that can.

## What this crate is

Data, and nothing else. It depends on nothing, parses nothing and knows
nothing about the rest of sysml-v2-rs — which is what lets it be a
dependency of whatever wants it without dragging a toolchain along, and
what lets it carry its own licence.

## Licence

**EPL-2.0**, unlike the rest of sysml-v2-rs.

Everything under `library/` is the OMG SysML v2 Release's, unchanged,
under the Eclipse Public License 2.0. The rest of
[sysml-v2-rs](https://github.com/haradama/sysml-v2-rs) is MIT or
Apache-2.0, at your option. They are separate crates so that which files
are under which licence is a question with a one-word answer. See
[`NOTICE`](NOTICE).

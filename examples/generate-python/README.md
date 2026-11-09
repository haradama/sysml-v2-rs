# Generating Python

A generator for a language this toolchain has never heard of, in about
three hundred lines of that language. `sysml codegen` hands it the
*plan* -- what the model implies for code, in no language in particular,
as `sysml plan` prints it -- on standard input, and takes what it prints
as the code. What SysML says is decided in the plan, once: whether four
wheels are a list, whether a part is owned or referred to, what
`ISQ::MassValue` bottoms out in, which state a machine starts in. What
Python says is decided here.

This is Doxygen's arrangement the other way round. A Doxygen filter
writes what Doxygen already reads; `sysml-gen-python` reads what `sysml
plan` already writes.

```sh
sysml codegen --via "python3 sysml-gen-python" model/
```

Or, with this directory on `PATH`, by name: the tool finds
`sysml-gen-python` the way `cargo` and `git` find their subcommands.
`--lang rust` is the generator built in, which is what `rustgen` runs.

```sh
PATH="$PWD:$PATH" sysml codegen --lang python model/ -o thermostat.py
```

`thermostat.py` is what comes out for `model/thermostat.sysml`, checked
in so that a change to either side shows; a test in `sysmlv2-cli` holds
it to what the two write. It imports and runs as it is:

```pycon
>>> from thermostat import *
>>> room = Room(current=Celsius(20.0), thermometer=None, display=None)
>>> room.target, room.mode
(21.0, <Mode.off: 1>)
>>> machine = Regulation()
>>> machine.warm(Celsius(2.0)); machine.state
<RegulationState.heating: 2>
```

## What it maps

| The plan says | Python |
| --- | --- |
| `record`, `abstract` | a keyword-only `@dataclass`, deriving from what it `specializes` (and from `ABC` where abstract); a `perform`ed action a method |
| `value` | a class deriving from the primitive it bottoms out in: `class Celsius(float)` |
| `enumeration` | an `Enum` |
| `variation` | a `Union` of its variants |
| `port` | a `Protocol` of its features |
| `function` | a `def`, the model's expression left as `NotImplementedError` to translate |
| `state machine` | an `Enum` of states and a class with a method per transition, guards as comments |
| `requirement` | a `check_...` function carrying what it requires and assumes |
| `behaviour` | a `def` whose docstring lists the steps in order and what flows between them |
| a constant of the package | a module-level name |
| multiplicity `[0..*]`, `[0..1]` | `list[T]`, `T \| None` |
| a default that is a literal, or an enumeration's member | the field's default |
| `@code { writtenIn = "python"; path = "sensors.Thermometer"; }` | `from sensors import Thermometer`, and nothing written for it |

What the model leaves open -- an expression to translate, a transition's
guard, the body of a behaviour -- is left open here too, and listed at
the end of the file and on standard error, so that what is still a
person's to write is stated rather than missing.

## Writing one for another language

`sysml plan --schema` prints the JSON schema of what a generator reads,
and `sysml --format json plan model/` prints a plan to look at. A
generator is any program that reads the plan on standard input and
writes the code on standard output; a failing exit is reported as the
failure it was, and what it says on standard error is passed through. A
model that does not resolve is refused before any generator runs, since
code generated from it would quietly leave out whatever the model
misspelt.

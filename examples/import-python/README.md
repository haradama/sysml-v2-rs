# Importing Python

An importer for a language this toolchain has never heard of, in about
two hundred lines of that language. `sysml import-code` reads a
*surface* -- what a reader of some code has to say about it, as JSON, in
no language in particular -- and writes the SysML package. Everything
SysML decides is decided there, once: which names are quoted because
they are reserved words, how two members that want one name are told
apart, what is refused for naming a type nothing declares. An importer
has only to say what the code declares.

This is Doxygen's arrangement. A Doxygen filter is a program that writes
what Doxygen already reads; `sysml-import-python` is a program that
writes what `sysml import-code` already reads.

```sh
sysml import-code --via "python3 sysml-import-python" inventory.py
```

Or, with this directory on `PATH`, no flag at all: the tool finds the
importer by name, the way `cargo` and `git` find their subcommands.
`sysml-import-python` answers to `--lang python`, and a copy called
`sysml-import-py` would answer to the `.py` extension on its own:

```sh
PATH="$PWD:$PATH" sysml import-code --lang python inventory.py
```

`inventory.sysml` is what comes out, checked in so that a change to
either side shows; a test in `sysmlv2-cli` holds it to what the two
write.

## What it maps

| Python | Surface | SysML |
| --- | --- | --- |
| a class with annotated attributes | `record` | `item def`, each attribute an `attribute` |
| a class deriving from `Enum` | `enumeration` | `enum def` |
| a class deriving from `Protocol` or `ABC` | `interface` | `port def`, each method an `action def` |
| a module-level `def` | `function` | `action def` |
| `int`, `float`, `bool`, `str` | `Integer`, `Real`, `Boolean`, `String` | the standard library's scalars |
| `bytes` | a scalar `PyBytes` under `String` | `attribute def PyBytes :> String` |
| `list[T]` | `many` | `[*]` |
| `Optional[T]`, `T \| None` | `optional` | `[0..1]` |
| `async def` | `"async": true` | `isAsync = true` |
| a body that `raise`s | `"fallible": true` | `isFallible = true` |

Anything else -- a `dict`, a list of lists, a parameter with no
annotation, a class with no annotated attributes -- is listed as skipped
rather than guessed at. The list is in the JSON, in comments at the end
of the package, and in the count `sysml import-code` prints.

The files are read with `ast` and never imported, so this runs on code
whose dependencies are not installed here. Every class is collected
before any signature is read, so that a function written above the
class it returns can name it -- and so that a class that was refused is
never named.

## Writing one for another language

`sysml import-code --schema` prints the JSON schema of a surface, and
`sysml import-rust --surface` writes the surface the Rust importer reads
off a real crate, which is the quickest way to see what one says. An
importer may also write SysML directly, as a Doxygen filter writes what
Doxygen reads; `import-code` checks either before writing it out, and an
importer that wrote a name that resolves to nothing is told so there,
rather than by the first model that imports the package.

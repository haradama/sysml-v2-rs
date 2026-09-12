# sysmlv2-diagram

Definition/interconnection diagrams in the standard's own notation —
labelled compartment stacks, ports on the border, composite and
reference memberships told apart by the diamond the specification
draws.

```rust
let diagram = sysml_diagram::definition_diagram(&model, &roots);
let svg = sysml_diagram::render(&diagram, &sysml_diagram::Style::default());
```

Where the boxes go and how the lines run is the Eclipse Layout Kernel's
answer, through [`elkrs`](https://crates.io/crates/elkrs) -- ELK's
algorithms ported to Rust and linked in, so there is nothing to install
and no process to spawn. Everything visible is drawn here, and the
engine decides nothing at random: one model renders to the same bytes
every time.

## Skins

A skin says colour, face and weight; the notation says everything else.
`--skin mono` is the drawing with nothing said about the dark, which is
what paper and every renderer that reads no media query see anyway.
`--skin <file>` paints it as the file says:

```json
{
  "light": {
    "page": "#ffffff", "fill": "#ffffff", "ink": "#000000",
    "kinds": {
      "part def": { "fill": "#e8f0fe", "line": "#1a3a6b" },
      "requirement def": "#fdecea",
      "package": "#f6f6f6"
    }
  },
  "dark": null
}
```

A kind is the word the drawing writes in guillemets -- `part def`,
`state def` -- with `package` for the frame and `comment` for the folded
note, which write none. A bare colour is a fill, which is what a reader
usually means; `fill`, `line` and `text` say more. `page` is what a name
written over a line is haloed in, so it wants to be the colour of
whatever the drawing is put on rather than the colour of a box.

Colour says nothing the SysML v2 notation defines: the shapes carry the
meaning, and a skin never changes which marker means what. A drawing
nobody painted is the drawing this wrote before skins existed, byte for
byte.

## What is drawn

Three views, each the standard's own. `sysml diagram` in
[`sysml-cli`](../sysml-cli) is the command-line way to all of them.

**Definitions** (the default) -- one box per definition, with its
keyword, name and declared features in labelled compartments. A hollow
triangle points from each subtype at what it specializes; a filled
diamond marks the whole of a composition, and the different marker
`portion-relationship` draws marks a timeslice or snapshot. Abstract
names are italic. Only specializations order the layers -- a whole is
not a subtype of its parts -- though a whole and its parts are kept side
by side within one. A line that would pass under an unrelated box steps
around it, through a clear gap between rows or round whole rows by a
column nothing is drawn in; every route is checked against the boxes it
would cross before it is drawn.

What a definition relates to that is not on the canvas is said in words
instead, in the `relationships-compartment`: `specializes Integer` for a
supertype that lives in the library. What it answers for is drawn --
`satisfy`, `assert`, `assume`, `require`, `perform`, `exhibit` and
`event occurrence` each get the keyworded line the standard gives them,
and `dependency` the one dashed line in the notation. What it documents
is drawn too, in the `documentation-compartment`, and a `comment about
A, B` becomes the folded-corner note with a dashed link to each thing it
is about. A view says what it exposes and filters by.

**Internal structure** (`--internal <NAME>`) -- the standard's
`interconnection-view`: a box per part, state or action the definition
is composed of, and an edge per statement relating two of them. The same
view serves a `part def` (parts wired by `connect`), a `state def`
(states linked by `transition`) and an `action def` (steps sequenced by
`first ... then`), so a behaviour drawn as an empty frame is one whose
steps went unsaid. A part box lists the features its *type* declares,
since `part w : Wheel;` writes none of its own, and holds what it is
itself assembled from as boxes inside it, one level deep.

Each two-ended statement is drawn the way its own production draws it:
`bind` a plain line written `=`, `interface` keyworded, `allocate` an
open arrowhead, a `flow` the filled arrowhead and what it carries, a
`message` the open dart. A succession is dashed -- a step following a
step is not a machine changing state -- and a transition carries an open
arrowhead labelled name, payload, guard and effect. A control node is
the glyph the standard gives it rather than a box: `fork` and `join`
bars, `merge` and `decide` diamonds, `terminate` a cross. An n-ary
connection meets at the dot the standard draws for it.

Ports sit on the border as small squares with `name : Type` beside them,
on the side facing whatever they connect to; a name that reaches
*through* the border is the standard's proxy circle instead. Connector
ends are matched by the feature chain name resolution records, so
`connect w.hub to a.mount` links the right boxes even where several
parts share a type. An action's parameters go on its border the same
way, rounded rather than square, with the arrow a declared direction
carries. `perform`ed actions partition the view into swimlanes, one
column per performer.

**Sequence** (`--sequence <NAME>`) -- a head node per participant, a
dashed lifeline under each, one arrow per message. The participant is
the feature chain a message reaches through without the event it ends
at. **Membership tree** (`--browser`) -- one indented row per named
element.

Packages are the folder the standard draws round what they hold, worked
out as the boxes are placed rather than fitted round them afterwards, so
two frames can never overlap. A layer wider than `Style::max_row_width`
wraps onto further rows.

## Part of

[sysml-v2-rs](https://github.com/haradama/sysml-v2-rs) -- Rust libraries
for SysML v2. See the workspace README for how the crates fit together
and what has been validated against the official corpus.

## License

MIT OR Apache-2.0.

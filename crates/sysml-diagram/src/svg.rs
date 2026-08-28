//! Serializing a laid-out diagram as a standalone SVG document.

use std::collections::HashMap;
use std::fmt::Write;

use crate::graph::{lines, Node};
use crate::layout::child_boxes;
use crate::{Diagram, Edge, Feature, Layout, Placed, Relation, Shape, Style};

/// Font stack for the drawing: the same families a browser would pick for
/// UI text, so a diagram looks native wherever it is embedded.
/// The typeface the specification's own notation figures are set in.
const FONT: &str = "Arial, Helvetica, sans-serif";

/// Colours for both viewer themes, after the specification's figures:
/// black ink on white boxes, keywords included. The dark palette keeps the
/// same print-like contrast for dark viewers (the VSCode preview among
/// them); the document is self-contained, so the palette travels with it.
const CSS: &str = "\
:root { --box: #ffffff; --line: #000000; --text: #000000; --muted: #000000; }\n\
@media (prefers-color-scheme: dark) {\n\
  :root { --box: #1e1e1e; --line: #d4d4d4; --text: #d4d4d4; --muted: #d4d4d4; }\n\
}\n\
.box { fill: var(--box); stroke: var(--line); stroke-width: 1; }\n\
.rule, .edge { stroke: var(--line); stroke-width: 1; fill: none; }\n\
.arrow { fill: var(--box); stroke: var(--line); stroke-width: 1; }\n\
.diamond { fill: var(--line); stroke: var(--line); stroke-width: 1; }\n\
.hollow { fill: var(--box); stroke: var(--line); stroke-width: 1; }\n\
.tip { fill: none; stroke: var(--line); stroke-width: 1; }\n\
.initial { fill: var(--line); }\n\
.port { fill: var(--box); stroke: var(--line); stroke-width: 1; }\n\
.guide { stroke: var(--muted); stroke-width: 1; opacity: 0.4; }\n\
.dependency { stroke: var(--line); stroke-width: 1; fill: none; stroke-dasharray: 6 4; }\n\
.succession { stroke: var(--line); stroke-width: 1; fill: none; stroke-dasharray: 4 3; }\n\
.lifeline { stroke: var(--line); stroke-width: 1; fill: none; stroke-dasharray: 3 4; }\n\
.name { fill: var(--text); font-weight: bold; }\n\
.abstract { font-style: italic; }\n\
.keyword, .feature { fill: var(--muted); }\n\
.compartment { fill: var(--muted); font-style: italic; }\n";

/// The arrowheads and diamonds every view draws with, defined once so a
/// document that uses one carries it.
pub(crate) fn markers() -> String {
    let mut out = String::new();
    writeln!(
        out,
        "<defs>\
             <marker id=\"specialization\" viewBox=\"0 0 12 10\" refX=\"12\" refY=\"5\" \
             markerWidth=\"12\" markerHeight=\"10\" orient=\"auto\">\
             <path class=\"arrow\" d=\"M0,0 L12,5 L0,10 z\"/></marker>\
             <marker id=\"composition\" viewBox=\"0 0 16 10\" refX=\"0\" refY=\"5\" \
             markerWidth=\"16\" markerHeight=\"10\" orient=\"auto\">\
             <path class=\"diamond\" d=\"M0,5 L8,0 L16,5 L8,10 z\"/></marker>\
             <marker id=\"subsetting\" viewBox=\"0 0 12 10\" refX=\"12\" refY=\"5\" \
             markerWidth=\"12\" markerHeight=\"10\" orient=\"auto\">\
             <path class=\"hollow\" d=\"M0,0 L12,5 L0,10 z\"/></marker>\
             <marker id=\"redefinition\" viewBox=\"0 0 16 10\" refX=\"16\" refY=\"5\" \
             markerWidth=\"16\" markerHeight=\"10\" orient=\"auto\">\
             <path class=\"hollow\" d=\"M4,0 L16,5 L4,10 z\"/>\
             <path class=\"tip\" d=\"M2,0 L2,10\"/></marker>\
             <marker id=\"reference\" viewBox=\"0 0 16 10\" refX=\"0\" refY=\"5\" \
             markerWidth=\"16\" markerHeight=\"10\" orient=\"auto\">\
             <path class=\"hollow\" d=\"M0,5 L8,0 L16,5 L8,10 z\"/></marker>\
             <marker id=\"transition\" viewBox=\"0 0 10 8\" refX=\"10\" refY=\"4\" \
             markerWidth=\"10\" markerHeight=\"8\" orient=\"auto\">\
             <path class=\"tip\" d=\"M0,0 L10,4 L0,8\"/></marker>\
             <marker id=\"flow\" viewBox=\"0 0 10 9\" refX=\"10\" refY=\"4.5\" \
             markerWidth=\"10\" markerHeight=\"9\" orient=\"auto\">\
             <path class=\"diamond\" d=\"M0,0 L10,4.5 L0,9 z\"/></marker>\
             <marker id=\"message\" viewBox=\"0 0 10 9\" refX=\"10\" refY=\"4.5\" \
             markerWidth=\"10\" markerHeight=\"9\" orient=\"auto\">\
             <path class=\"tip\" d=\"M0,0 L10,4.5 L0,9 L2.5,4.5 z\"/></marker>\
             <marker id=\"portion\" viewBox=\"0 0 10 10\" refX=\"0\" refY=\"5\" \
             markerWidth=\"10\" markerHeight=\"10\" orient=\"auto\">\
             <circle class=\"diamond\" cx=\"5\" cy=\"5\" r=\"4\"/></marker></defs>"
    )
    .unwrap();
    out
}

/// Render a laid-out diagram. The output is a complete SVG document: it can
/// be written to a `.svg` file or inlined into HTML as-is.
pub fn to_svg(diagram: &Diagram, layout: &Layout, style: &Style) -> String {
    let mut out = markers();

    // edges first, so the boxes paint over the line ends. Ports sit on
    // those borders and must survive, so they are held back until after.
    let mut ports = String::new();
    // how far down a detour reached, so the canvas can grow to hold it
    let mut floor = 0.0_f64;
    let lanes = lanes(diagram);
    let arrivals = arrivals(diagram);
    for (index, (edge, &(lane, siblings))) in diagram.edges.iter().zip(&lanes).enumerate() {
        let from = &layout.placed[edge.from];
        let to = &layout.placed[edge.to];
        match edge.relation {
            // the layering already put the supertype above, so the line
            // runs from the subtype's top edge to the supertype's bottom
            Relation::Specialization if given(layout, index).is_some() => {
                let walked = given(layout, index).expect("just checked");
                writeln!(
                    out,
                    "<path class=\"edge\" fill=\"none\" d=\"{}\" \
                     marker-end=\"url(#specialization)\"/>",
                    polyline(walked)
                )
            }
            Relation::Specialization => {
                let (x1, y1) = (from.x + from.width / 2.0, from.y);
                // subtypes of one supertype would otherwise pile their
                // arrowheads on a single point of its border
                let (x2, y2) = (to.x + to.width * arrivals[index], to.y + to.height);
                // the gap under the supertype's row is where a hierarchy
                // usually gathers; the gap over the subtype's is the fallback
                let bands = (
                    band_above(layout, edge.from, style),
                    band_below(layout, edge.to, style),
                );
                let blocked = hidden(layout, (edge.from, edge.to), (x1, y1), (x2, y2));
                let channel = blocked
                    .then(|| {
                        channel_for(
                            layout,
                            ((x1, y1), (x2, y2)),
                            &[bands.1, bands.0, band_above(layout, edge.to, style)],
                        )
                    })
                    .flatten();
                match channel {
                    Some(channel) => writeln!(
                        out,
                        "<path class=\"edge\" fill=\"none\" d=\"M {x1:.1} {y1:.1} \
                         V {channel:.1} H {x2:.1} V {y2:.1}\" \
                         marker-end=\"url(#specialization)\"/>"
                    ),
                    // no single gap reaches: go round the rows in between
                    None => match blocked
                        .then(|| sidestep(layout, ((x1, y1), (x2, y2)), bands, style))
                        .flatten()
                    {
                        Some(column) => writeln!(
                            out,
                            "<path class=\"edge\" fill=\"none\" d=\"M {x1:.1} {y1:.1} \
                             V {:.1} H {column:.1} V {:.1} H {x2:.1} V {y2:.1}\" \
                             marker-end=\"url(#specialization)\"/>",
                            bands.0, bands.1
                        ),
                        None => writeln!(
                            out,
                            "<line class=\"edge\" x1=\"{x1:.1}\" y1=\"{y1:.1}\" \
                             x2=\"{x2:.1}\" y2=\"{y2:.1}\" \
                             marker-end=\"url(#specialization)\"/>"
                        ),
                    },
                }
            }
            // neither of these follows the layering, so the line runs
            // centre to centre clipped to both borders. Composition puts a
            // filled diamond on the side of the whole; a connection is
            // undirected and gets no marker at all.
            // A feature typed by the thing that declares it. The line
            // has nowhere to go but back, so it drops into the gap
            // under the row and returns -- downward because that is the
            // one direction the canvas grows to make room in.
            Relation::Composition | Relation::Reference if edge.from == edge.to => {
                let (marker, class) = pen(edge.relation);
                let bottom = from.y + from.height;
                // a loop has no side to be shifted onto, so what tells
                // two of them apart is which came first, not the signed
                // lane a pair of boxes shares
                let nth = lane + (siblings as f64 - 1.0) / 2.0;
                let band = band_below(layout, edge.from, style) + nth * style.line_height;
                let inset = nth * style.line_height / 2.0;
                let left = from.x + from.width / 3.0 + inset;
                let right = from.x + from.width * 2.0 / 3.0 - inset;
                floor = floor.max(band);
                writeln!(
                    out,
                    "<path{class} fill=\"none\" d=\"M {left:.1} {bottom:.1} V {band:.1} \
                     H {right:.1} V {bottom:.1}\"{marker}/>"
                )
            }
            Relation::Composition
            | Relation::Reference
            | Relation::Subsetting
            | Relation::Redefinition
            | Relation::Connection
            | Relation::Transition
            | Relation::Succession
            | Relation::Satisfy
            | Relation::Binding
            | Relation::Interface
            | Relation::Allocation
            | Relation::Flow
            | Relation::SuccessionFlow
            | Relation::Message
            | Relation::Assert
            | Relation::Assume
            | Relation::Require
            | Relation::Perform
            | Relation::Exhibit
            | Relation::Dependency
            | Relation::Portion
            | Relation::Event
            | Relation::Annotation => {
                // a connection ends at the port it names, where the
                // box declares one: the standard draws the port on the
                // border, and a second square beside it would be a
                // second port that the model never had
                let ((mut x1, mut y1), first_is_port) = match &edge.ends.0 {
                    Some(end) => at_port(diagram, layout, edge.from, end, centre_of(to)),
                    None => (border_point(from, centre_of(to)), false),
                };
                let ((mut x2, mut y2), second_is_port) = match &edge.ends.1 {
                    Some(end) => at_port(diagram, layout, edge.to, end, centre_of(from)),
                    None => (border_point(to, centre_of(from)), false),
                };
                // shift edges sharing a pair of boxes along the normal, so
                // two connections do not collapse into one line
                let (dx, dy) = (x2 - x1, y2 - y1);
                let length = dx.hypot(dy);
                if length > 0.0 {
                    let shift = lane * lane_spacing(from, to, siblings, style);
                    let (nx, ny) = (-dy / length * shift, dx / length * shift);
                    x1 += nx;
                    y1 += ny;
                    x2 += nx;
                    y2 += ny;
                }
                let (marker, class) = pen(edge.relation);
                // a straight line that runs under an unrelated box reads as
                // a connection to that box, so step around it instead: both
                // boxes are left downward and joined in a clear channel
                // beneath the row, one lane per shared pair
                let lane_shift = lane.abs() * style.line_height;
                let detour = (
                    (from.x + from.width / 2.0, from.y + from.height),
                    (to.x + to.width / 2.0, to.y + to.height),
                );
                let bands = (
                    band_below(layout, edge.from, style) + lane_shift,
                    band_below(layout, edge.to, style) + lane_shift,
                );
                let route = match given(layout, index) {
                    Some(walked) => Some(Detour::Given(walked)),
                    None => hidden(layout, (edge.from, edge.to), (x1, y1), (x2, y2))
                        .then(|| {
                            channel_for(layout, detour, &[bands.0, bands.1])
                                .map(Detour::Channel)
                                .or_else(|| {
                                    sidestep(layout, detour, bands, style).map(Detour::Sidestep)
                                })
                        })
                        .flatten(),
                };
                let (first_toward, second_toward, label_at) = match route {
                    None => {
                        writeln!(
                            out,
                            "<line{class} x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" \
                             y2=\"{y2:.1}\"{marker}/>"
                        )
                        .unwrap();
                        ((x2, y2), (x1, y1), ((x1 + x2) / 2.0, (y1 + y2) / 2.0))
                    }
                    Some(Detour::Channel(channel)) => {
                        ((x1, y1), (x2, y2)) = detour;
                        writeln!(
                            out,
                            "<path{class} fill=\"none\" d=\"M {x1:.1} {y1:.1} V {channel:.1} \
                             H {x2:.1} V {y2:.1}\"{marker}/>"
                        )
                        .unwrap();
                        floor = floor.max(channel);
                        (
                            (x1, channel),
                            (x2, channel),
                            ((x1 + x2) / 2.0, channel - 0.5 * style.line_height),
                        )
                    }
                    Some(Detour::Given(walked)) => {
                        writeln!(
                            out,
                            "<path{class} fill=\"none\" d=\"{}\"{marker}/>",
                            polyline(walked)
                        )
                        .unwrap();
                        (x1, y1) = walked[0];
                        (x2, y2) = walked[walked.len() - 1];
                        floor = floor.max(walked.iter().map(|&(_, y)| y).fold(0.0, f64::max));
                        let middle = walked[walked.len() / 2];
                        (
                            walked[1],
                            walked[walked.len() - 2],
                            (middle.0, middle.1 - 0.5 * style.line_height),
                        )
                    }
                    Some(Detour::Sidestep(column)) => {
                        ((x1, y1), (x2, y2)) = detour;
                        let (first, second) = bands;
                        writeln!(
                            out,
                            "<path{class} fill=\"none\" d=\"M {x1:.1} {y1:.1} V {first:.1} \
                             H {column:.1} V {second:.1} H {x2:.1} V {y2:.1}\"{marker}/>"
                        )
                        .unwrap();
                        floor = floor.max(first.max(second));
                        // the column can be hard against the margin, so the
                        // name goes over the gap it sets out along instead
                        (
                            (x1, first),
                            (x2, second),
                            ((x1 + x2) / 2.0, first - 0.5 * style.line_height),
                        )
                    }
                };
                // A connection meets each box at a port, drawn the SysML
                // way: a small square on the border, named beside it.
                // Where the box declares that port, it is already
                // drawn there and the line simply arrives at it.
                if let Some(first) = edge.ends.0.as_ref().filter(|_| !first_is_port) {
                    port(&mut ports, (x1, y1), first_toward, first, None, style);
                }
                if let Some(second) = edge.ends.1.as_ref().filter(|_| !second_is_port) {
                    port(&mut ports, (x2, y2), second_toward, second, None, style);
                }
                if let Some(label) = &edge.label {
                    beside(&mut out, label_at, (x2, y2), label, style);
                }
                Ok(())
            }
        }
        .unwrap();
    }

    for (at, placed) in layout.placed.iter().enumerate() {
        let node = &diagram.nodes[placed.node];
        if let Some(glyph) = glyph_of(node.shape, placed) {
            out.push_str(&glyph);
            // what a marker is called is read above it: the `cdot-label`
            // of an n-ary connection, or the name a control node was
            // declared with
            if !node.name.is_empty() {
                beside_with(
                    &mut out,
                    centre_of(placed),
                    (0.0, -1.0),
                    placed.height / 2.0,
                    "middle",
                    style,
                    &node.name,
                );
            }
            continue;
        }
        draw_box(
            &mut out,
            node,
            (placed.x, placed.y, placed.width, placed.height),
            style,
        );
        // The standard draws a part's ports on its border, on any of
        // the four sides: `part-def = part-def-name-compartment
        // interconnection-view compartment-stack port-l* port-r*
        // port-t* port-b*`. They go in with the other ports, after
        // every box, so a neighbour drawn later cannot cover one.
        border_ports(&mut ports, diagram, layout, at, style);
    }

    out.push_str(&ports);
    let height = layout.height.max(floor + style.margin);
    document(layout.width, height, style, &out)
}

/// The glyph the standard gives a node that is not a box: the filled
/// circle a flow starts at or an n-ary connection meets at, the bar it
/// splits at, the diamond it chooses at, the cross it stops at. A box has
/// no glyph -- it is drawn as a box.
fn glyph_of(shape: Shape, placed: &Placed) -> Option<String> {
    let (cx, cy) = centre_of(placed);
    let (w, h) = (placed.width, placed.height);
    Some(match shape {
        // a note and a box are both drawn with their text inside
        Shape::Note | Shape::Box => return None,
        Shape::Initial | Shape::ConnectionDot => format!(
            "<circle class=\"initial\" cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{:.1}\"/>\n",
            w / 2.0
        ),
        Shape::Bar => format!(
            "<rect class=\"initial\" x=\"{:.1}\" y=\"{:.1}\" width=\"{w:.1}\" \
             height=\"{h:.1}\"/>\n",
            placed.x, placed.y
        ),
        Shape::Diamond => format!(
            "<path class=\"box\" d=\"M {cx:.1} {:.1} L {:.1} {cy:.1} L {cx:.1} {:.1} \
             L {:.1} {cy:.1} z\"/>\n",
            placed.y,
            placed.x + w,
            placed.y + h,
            placed.x
        ),
        Shape::Cross => format!(
            "<path class=\"rule\" d=\"M {:.1} {:.1} L {:.1} {:.1} M {:.1} {:.1} \
             L {:.1} {:.1}\"/>\n",
            placed.x,
            placed.y,
            placed.x + w,
            placed.y + h,
            placed.x + w,
            placed.y,
            placed.x,
            placed.y + h
        ),
    })
}

/// Do the two boxes share a band of the canvas -- that is, is one drawn
/// beside the other rather than above or below it?
fn alongside(one: &Placed, other: &Placed) -> bool {
    one.y < other.y + other.height && other.y < one.y + one.height
}

/// The gap under the row `node` sits in. Nothing is drawn there, so a line
/// can cross the whole width of the diagram along it.
fn band_below(layout: &Layout, node: usize, style: &Style) -> f64 {
    let row = &layout.placed[node];
    layout
        .placed
        .iter()
        .filter(|rect| alongside(rect, row))
        .map(|rect| rect.y + rect.height)
        .fold(f64::MIN, f64::max)
        + style.v_gap * 0.4
}

/// The gap over the row `node` sits in, the counterpart of [`band_below`].
fn band_above(layout: &Layout, node: usize, style: &Style) -> f64 {
    let row = &layout.placed[node];
    layout
        .placed
        .iter()
        .filter(|rect| alongside(rect, row))
        .map(|rect| rect.y)
        .fold(f64::MAX, f64::min)
        - style.v_gap * 0.4
}

/// Where a three-segment route should cross, for a line that would
/// otherwise run under a box. Returns the first candidate channel that
/// clears everything, and `None` when the straight line is already fine or
/// no candidate is any better -- a detour that still crosses a box is worth
/// nothing, and the straight line at least reads as a straight line.
fn channel_for(
    layout: &Layout,
    detour: ((f64, f64), (f64, f64)),
    candidates: &[f64],
) -> Option<f64> {
    // a detour leaves both boxes squarely through a border, so nothing is
    // excused here: a leg that turns back into its own box is no good either
    let (start, finish) = detour;
    candidates.iter().copied().find(|&channel| {
        !obstructed(
            layout,
            None,
            &[start, (start.0, channel), (finish.0, channel), finish],
        )
    })
}

/// Would the straight line between two boxes disappear under a third? The
/// line may start just inside a border once it has been shifted into its
/// lane, so its own two boxes do not count against it.
fn hidden(layout: &Layout, ends: (usize, usize), start: (f64, f64), finish: (f64, f64)) -> bool {
    obstructed(layout, Some(ends), &[start, finish])
}

/// How a line that cannot be drawn straight gets round what is in the way.
enum Detour<'a> {
    /// The layout engine already said where the line goes.
    Given(&'a [(f64, f64)]),
    /// One gap between the rows carries the whole crossing.
    Channel(f64),
    /// Two gaps do, joined by a column nothing is drawn in.
    Sidestep(f64),
}

/// The path the layout engine chose for one edge, where it chose one
/// worth drawing.
fn given(layout: &Layout, edge: usize) -> Option<&[(f64, f64)]> {
    layout
        .routes
        .get(edge)
        .map(Vec::as_slice)
        .filter(|walked| walked.len() >= 2)
}

/// A run of points as an SVG path.
fn polyline(walked: &[(f64, f64)]) -> String {
    let mut out = String::new();
    for (at, (x, y)) in walked.iter().enumerate() {
        let step = if at == 0 { 'M' } else { 'L' };
        write!(
            out,
            "{}{step} {x:.1} {y:.1}",
            if at == 0 { "" } else { " " }
        )
        .unwrap();
    }
    out
}

/// A way round whole rows of boxes, for a line that cannot reach its
/// supertype through any one gap: up into the gap over its own row, along
/// to a column nothing is drawn in, up to the gap under the supertype, and
/// across to it. Returns the two gaps and the column between them.
fn sidestep(
    layout: &Layout,
    detour: ((f64, f64), (f64, f64)),
    (first, second): (f64, f64),
    style: &Style,
) -> Option<f64> {
    let (start, finish) = detour;
    if (first - second).abs() < f64::EPSILON {
        // both gaps are the same one, which [`channel_for`] has already tried
        return None;
    }
    let mut columns: Vec<f64> = layout
        .placed
        .iter()
        .flat_map(|rect| {
            [
                rect.x - style.h_gap / 2.0,
                rect.x + rect.width + style.h_gap / 2.0,
            ]
        })
        .chain([style.margin / 2.0, layout.width - style.margin / 2.0])
        // a column outside the canvas would take the line off the drawing
        .filter(|&column| {
            (style.margin / 2.0..=layout.width - style.margin / 2.0).contains(&column)
        })
        .collect();
    // the shortest way round is the one nearest the boxes it joins
    let middle = (start.0 + finish.0) / 2.0;
    columns.sort_by(|one, other| (one - middle).abs().total_cmp(&(other - middle).abs()));
    columns.into_iter().find(|&column| {
        !obstructed(
            layout,
            None,
            &[
                start,
                (start.0, first),
                (column, first),
                (column, second),
                (finish.0, second),
                finish,
            ],
        )
    })
}

/// Does a box get in the route's way, the `spare` pair aside?
fn obstructed(layout: &Layout, spare: Option<(usize, usize)>, route: &[(f64, f64)]) -> bool {
    layout.placed.iter().enumerate().any(|(other, rect)| {
        !spare.is_some_and(|(from, to)| other == from || other == to)
            && route.windows(2).any(|leg| crosses(rect, leg[0], leg[1]))
    })
}

/// Does the segment cross the inside of the rectangle? Used to tell a line
/// that merely passes near a box from one that disappears under it, so the
/// borders themselves do not count as a crossing.
fn crosses(rect: &Placed, (x1, y1): (f64, f64), (x2, y2): (f64, f64)) -> bool {
    const GRAZE: f64 = 1.0;
    let (dx, dy) = (x2 - x1, y2 - y1);
    let (mut enter, mut leave) = (0.0_f64, 1.0_f64);
    let sides = [
        (-dx, x1 - (rect.x + GRAZE)),
        (dx, rect.x + rect.width - GRAZE - x1),
        (-dy, y1 - (rect.y + GRAZE)),
        (dy, rect.y + rect.height - GRAZE - y1),
    ];
    for (towards, room) in sides {
        if towards == 0.0 {
            // parallel to this side: outside it means outside the box
            if room < 0.0 {
                return false;
            }
        } else if towards < 0.0 {
            enter = enter.max(room / towards);
        } else {
            leave = leave.min(room / towards);
        }
    }
    enter < leave
}

/// The marker and the class a centre-to-centre relation is drawn with.
/// A specialization never comes this way: it draws along the layering,
/// with its own hollow-triangle marker.
fn pen(relation: Relation) -> (&'static str, &'static str) {
    match relation {
        Relation::Composition => (" marker-start=\"url(#composition)\"", " class=\"edge\""),
        // the same hollow triangle a subclassification carries, and for
        // a redefinition a bar across the line with it
        Relation::Subsetting => (" marker-end=\"url(#subsetting)\"", " class=\"edge\""),
        Relation::Redefinition => (" marker-end=\"url(#redefinition)\"", " class=\"edge\""),
        // what the owner refers to but is not made of: the same diamond,
        // left hollow, which is how a drawing has told the two apart
        // since long before SysML
        Relation::Reference => (" marker-start=\"url(#reference)\"", " class=\"edge\""),
        Relation::Transition => (" marker-end=\"url(#transition)\"", " class=\"edge\""),
        // `aflow-succession` is dashed where `transition` is plain: one
        // step following another is not a machine changing state
        Relation::Succession => (" marker-end=\"url(#transition)\" class=\"succession\"", ""),
        // `allocate-relationship` draws the same open arrowhead a
        // transition does, and says which it is with `«allocate»`
        Relation::Allocation => (" marker-end=\"url(#transition)\"", " class=\"edge\""),
        // `assert-edge`, `assume-edge`, `require-edge`, `perform-edge`,
        // `exhibit-edge` and `satisfy-edge` are one figure with six
        // keywords: a plain line and the same open arrowhead
        Relation::Assert
        | Relation::Assume
        | Relation::Require
        | Relation::Perform
        | Relation::Exhibit
        | Relation::Event => (" marker-end=\"url(#transition)\"", " class=\"edge\""),
        // `portion-relationship` marks the whole the way a composition
        // does, with a filled glyph of its own
        Relation::Portion => (" marker-start=\"url(#portion)\"", " class=\"edge\""),
        // what flows has the filled head; a message has the open dart the
        // standard keeps for it
        Relation::Flow | Relation::SuccessionFlow => {
            (" marker-end=\"url(#flow)\"", " class=\"edge\"")
        }
        Relation::Message => (" marker-end=\"url(#message)\"", " class=\"edge\""),
        // `satisfy-edge` is drawn the same way, and the specification
        // draws it solid rather than dashed
        Relation::Satisfy => (" marker-end=\"url(#transition)\"", " class=\"edge\""),
        // `binary-dependency` is the one dashed line in the notation
        Relation::Dependency => (" marker-end=\"url(#transition)\" class=\"dependency\"", ""),
        // `annotation-link` is dashed too, and carries nothing at either
        // end: which is the note is plain from the shapes
        Relation::Annotation => ("", " class=\"dependency\""),
        // a connection, an interface and a binding are undirected and get
        // no marker at all -- what each is, its label says
        _ => ("", " class=\"edge\""),
    }
}

/// Wrap `body` in the SVG shell every view shares: the canvas, the font and
/// the palette that travels with the document.
pub(crate) fn document(width: f64, height: f64, style: &Style, body: &str) -> String {
    let mut out = String::new();
    writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" \
         viewBox=\"0 0 {width:.0} {height:.0}\" font-family=\"{FONT}\" \
         font-size=\"{:.0}\">",
        style.font_size
    )
    .expect("writing to a String cannot fail");
    writeln!(out, "<style>\n{CSS}</style>").unwrap();
    out.push_str(body);
    writeln!(out, "</svg>").unwrap();
    out
}

/// Draw one box and, inside it, the parts it is assembled from.
fn draw_box(out: &mut String, node: &Node, rect: (f64, f64, f64, f64), style: &Style) {
    let (x, y, width, height) = rect;
    // `comment-node` is the folded-corner note, holding its text and
    // nothing else -- no keyword line, no compartments
    if node.shape == Shape::Note {
        let fold = style.line_height;
        writeln!(
            out,
            "<path class=\"box\" d=\"M {x:.1} {y:.1} H {:.1} L {:.1} {:.1} V {:.1} \
             H {x:.1} z\"/>\n\
             <path class=\"rule\" fill=\"none\" d=\"M {:.1} {y:.1} V {:.1} H {:.1}\"/>",
            x + width - fold,
            x + width,
            y + fold,
            y + height,
            x + width - fold,
            y + fold,
            x + width,
        )
        .unwrap();
        let mut line = y + style.padding + 0.75 * style.line_height;
        if !node.keyword.is_empty() {
            writeln!(
                out,
                "<text class=\"keyword\" x=\"{:.1}\" y=\"{line:.1}\">\u{ab}{}\u{bb}</text>",
                x + style.padding,
                escape(&node.keyword)
            )
            .unwrap();
            line += style.line_height;
        }
        // the text of a comment, or what a metadata usage was declared as
        // and the values it sets
        for written in std::iter::once(node.name.clone()).chain(lines(node).map(Feature::label)) {
            writeln!(
                out,
                "<text class=\"feature\" x=\"{:.1}\" y=\"{line:.1}\">{}</text>",
                x + style.padding,
                escape(&written)
            )
            .unwrap();
            line += style.line_height;
        }
        return;
    }
    {
        let placed = Placed {
            node: 0,
            x,
            y,
            width,
            height,
        };
        let centre = placed.x + placed.width / 2.0;
        let header = placed.y + style.padding;

        writeln!(
            out,
            "<g>\n<rect class=\"box\" x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"{}\"/>",
            placed.x,
            placed.y,
            placed.width,
            placed.height,
            // the notation rounds usages and leaves definitions square
            if node.rounded { 10 } else { 0 }
        )
        .unwrap();
        writeln!(
            out,
            "<text class=\"keyword\" x=\"{centre:.1}\" y=\"{:.1}\" text-anchor=\"middle\">\u{ab}{}\u{bb}</text>",
            header + 0.75 * style.line_height,
            escape(&node.keyword)
        )
        .unwrap();
        writeln!(
            out,
            "<text class=\"name{}\" x=\"{centre:.1}\" y=\"{:.1}\" \
             text-anchor=\"middle\">{}</text>",
            // UML sets an abstract classifier's name in italic
            if node.is_abstract { " abstract" } else { "" },
            header + 1.75 * style.line_height,
            escape(&node.name)
        )
        .unwrap();

        // the standard stacks labelled compartments under the name:
        // `extended-def = extended-def-name-compartment compartment-stack`
        let mut top = header + 2.0 * style.line_height;
        for compartment in &node.compartments {
            let rule = top + style.padding / 2.0;
            writeln!(
                out,
                "<line class=\"rule\" x1=\"{:.1}\" y1=\"{rule:.1}\" x2=\"{:.1}\" y2=\"{rule:.1}\"/>",
                placed.x,
                placed.x + placed.width
            )
            .unwrap();
            writeln!(
                out,
                "<text class=\"compartment\" x=\"{:.1}\" y=\"{:.1}\">{}</text>",
                placed.x + style.padding,
                top + style.padding + 0.75 * style.line_height,
                escape(compartment.label)
            )
            .unwrap();
            for (row, line) in compartment.lines.iter().enumerate() {
                writeln!(
                    out,
                    "<text class=\"feature\" x=\"{:.1}\" y=\"{:.1}\">{}</text>",
                    placed.x + 2.0 * style.padding,
                    top + style.padding + (row as f64 + 1.75) * style.line_height,
                    escape(&line.label())
                )
                .unwrap();
            }
            top += style.padding + (1 + compartment.lines.len()) as f64 * style.line_height;
        }
        writeln!(out, "</g>").unwrap();
    }
    let inners = child_boxes(node, rect, style);
    // the lines first, so the boxes paint over their ends -- the same
    // order the canvas itself is drawn in
    interconnections(out, node, &inners, style);
    for (child, inner) in node.children.iter().zip(inners) {
        draw_box(out, child, inner, style);
    }
}

/// What joins the parts drawn inside a box: the other half of
/// `interconnection-view`, drawn straight between the two children with
/// the same pen the canvas uses and a port square at each named end.
fn interconnections(out: &mut String, node: &Node, inners: &[(f64, f64, f64, f64)], style: &Style) {
    let placed = |at: usize| {
        let (x, y, width, height) = inners[at];
        Placed {
            node: at,
            x,
            y,
            width,
            height,
        }
    };
    for link in &node.links {
        let (from, to) = (placed(link.from), placed(link.to));
        let first = border_point(&from, centre_of(&to));
        let second = border_point(&to, centre_of(&from));
        let (marker, class) = pen(link.relation);
        writeln!(
            out,
            "<path{class} fill=\"none\" d=\"M {:.1} {:.1} L {:.1} {:.1}\"{marker}/>",
            first.0, first.1, second.0, second.1
        )
        .unwrap();
        if let Some(name) = &link.ends.0 {
            port(out, first, second, name, None, style);
        }
        if let Some(name) = &link.ends.1 {
            port(out, second, first, name, None, style);
        }
        if let Some(label) = &link.label {
            beside(
                out,
                ((first.0 + second.0) / 2.0, (first.1 + second.1) / 2.0),
                second,
                label,
                style,
            );
        }
    }
}

/// For each edge, its perpendicular offset factor and how many edges share
/// its pair of boxes. Edges of one pair are spread symmetrically about the
/// straight line between them.
fn lanes(diagram: &Diagram) -> Vec<(f64, usize)> {
    let pair = |edge: &Edge| (edge.from.min(edge.to), edge.from.max(edge.to));
    let mut total: HashMap<(usize, usize), usize> = HashMap::new();
    for edge in &diagram.edges {
        *total.entry(pair(edge)).or_default() += 1;
    }
    let mut taken: HashMap<(usize, usize), usize> = HashMap::new();
    diagram
        .edges
        .iter()
        .map(|edge| {
            let siblings = total[&pair(edge)];
            let slot = taken.entry(pair(edge)).or_default();
            let index = *slot;
            *slot += 1;
            (index as f64 - (siblings as f64 - 1.0) / 2.0, siblings)
        })
        .collect()
}

/// Where along a supertype's border each specialization lands, as a
/// fraction of its width. Subtypes are spread evenly so their arrowheads
/// stay apart; anything else arrives at the middle.
fn arrivals(diagram: &Diagram) -> Vec<f64> {
    let mut total: HashMap<usize, usize> = HashMap::new();
    for edge in &diagram.edges {
        if edge.relation == Relation::Specialization {
            *total.entry(edge.to).or_default() += 1;
        }
    }
    let mut taken: HashMap<usize, usize> = HashMap::new();
    diagram
        .edges
        .iter()
        .map(|edge| {
            if edge.relation != Relation::Specialization {
                return 0.5;
            }
            let slot = taken.entry(edge.to).or_default();
            let index = *slot;
            *slot += 1;
            (index as f64 + 1.0) / (total[&edge.to] as f64 + 1.0)
        })
        .collect()
}

/// How far apart to hold edges sharing a pair of boxes.
///
/// A line has to leave through a border, so the whole spread must fit within
/// the extent the two boxes share. Enough of them and the preferred spacing
/// would push the outermost lines clear off the boxes entirely.
fn lane_spacing(from: &Placed, to: &Placed, siblings: usize, style: &Style) -> f64 {
    let spread = (siblings as f64 - 1.0).max(1.0);
    let room = from.height.min(to.height).min(from.width).min(to.width) * 0.8;
    (room / spread).min(style.line_height)
}

/// Draw the port a connection attaches to: a small square centred on the
/// box border at `at`, with its name set just clear of it along the edge
/// and `across` (-1 or 1) to one side of it.
/// The ports a box declares, drawn on its border with their labels.
///
/// They are spread over the left and right sides, top to bottom, which
/// is where a reader looks for them and where the gap between columns
/// leaves room for a name.
fn border_ports(out: &mut String, diagram: &Diagram, layout: &Layout, at: usize, style: &Style) {
    for place in port_places(diagram, layout, at) {
        let label = match &place.feature.ty {
            Some(ty) => format!("{} : {ty}", place.feature.name),
            None => place.feature.name.clone(),
        };
        marked(
            out,
            place.at,
            place.toward,
            &label,
            place.feature.direction,
            place.rounded,
            style,
        );
    }
}

/// Where a connection meets a box: the port it names, when the box
/// declares one by that name, and the border otherwise.
fn at_port(
    diagram: &Diagram,
    layout: &Layout,
    at: usize,
    end: &str,
    toward: (f64, f64),
) -> ((f64, f64), bool) {
    match port_places(diagram, layout, at)
        .into_iter()
        .find(|place| place.feature.name == end)
    {
        Some(place) => (place.at, true),
        None => (border_point(&layout.placed[at], toward), false),
    }
}

/// One port on a border: what it is, where it sits, and the point its
/// name reads towards.
struct Placement<'a> {
    feature: &'a Feature,
    at: (f64, f64),
    toward: (f64, f64),
    /// A parameter, drawn as the rounded rectangle `param-l` has rather
    /// than the square of a port.
    rounded: bool,
}

/// Where each of a box's ports sits on its border, and which way its
/// name reads from there.
///
/// Worked out once and used twice: to draw the ports, and to end a
/// connection at the port it names rather than at a square of its own.
/// They alternate sides so a box with several does not stack them all
/// down one edge.
fn port_places<'a>(diagram: &'a Diagram, layout: &Layout, at: usize) -> Vec<Placement<'a>> {
    let placed = &layout.placed[at];
    let node = &diagram.nodes[placed.node];
    // An action's parameters sit on its border exactly as a part's ports
    // do -- `param-l | param-r | param-t | param-b` against `port-l ...`
    // -- and are drawn rounded rather than square.
    let listed: Vec<(&Feature, bool)> = node
        .compartments
        .iter()
        .filter_map(|compartment| match compartment.label {
            "ports" => Some((compartment, false)),
            "parameters" => Some((compartment, true)),
            _ => None,
        })
        .flat_map(|(compartment, rounded)| {
            compartment.lines.iter().map(move |line| (line, rounded))
        })
        .collect();

    // A port faces whatever it is connected to -- the standard puts one
    // on any of the four sides, and the side a reader expects is the
    // one the line comes from. A port nothing reaches falls back to the
    // sides a name has room to grow out of.
    let mut placements = Vec::new();
    let mut spare = 0usize;
    for (feature, rounded) in listed {
        let peer = diagram.edges.iter().find_map(|edge| {
            let (mine, theirs) = match (edge.from == at, edge.to == at) {
                (true, _) => (0, edge.to),
                (_, true) => (1, edge.from),
                _ => return None,
            };
            let named = if mine == 0 {
                &edge.ends.0
            } else {
                &edge.ends.1
            };
            (named.as_deref() == Some(feature.name.as_str()))
                .then(|| centre_of(&layout.placed[theirs]))
        });
        let (point, away) = match peer {
            Some(peer) => facing(placed, peer),
            None => {
                let left = spare % 2 == 0;
                let down =
                    placed.y + (spare / 2 + 1) as f64 * placed.height / (spare as f64 / 2.0 + 2.0);
                spare += 1;
                let x = if left {
                    placed.x
                } else {
                    placed.x + placed.width
                };
                ((x, down), (if left { -1.0 } else { 1.0 }, 0.0))
            }
        };
        placements.push(Placement {
            feature,
            at: point,
            toward: (point.0 + away.0 * 8.0, point.1 + away.1 * 8.0),
            rounded,
        });
    }
    placements
}

/// Where on a box's border a port facing `peer` sits, and which way its
/// name reads from there.
fn facing(placed: &Placed, peer: (f64, f64)) -> ((f64, f64), (f64, f64)) {
    let point = border_point(placed, peer);
    let centre = centre_of(placed);
    let (dx, dy) = (point.0 - centre.0, point.1 - centre.1);
    let away = if dx.abs() * placed.height >= dy.abs() * placed.width {
        (dx.signum(), 0.0)
    } else {
        (0.0, dy.signum())
    };
    (point, away)
}

fn port(
    out: &mut String,
    at: (f64, f64),
    toward: (f64, f64),
    name: &str,
    direction: Option<&str>,
    style: &Style,
) {
    marked(out, at, toward, name, direction, false, style)
}

/// The glyph straddling a border and the name beside it: `port-l` draws a
/// square, `param-l` the same rounded, and the direction arrow goes inside
/// either.
#[allow(clippy::too_many_arguments)]
fn marked(
    out: &mut String,
    at: (f64, f64),
    toward: (f64, f64),
    name: &str,
    direction: Option<&str>,
    rounded: bool,
    style: &Style,
) {
    let side = 0.6 * style.line_height;
    let corner = if rounded { side / 3.0 } else { 0.0 };
    writeln!(
        out,
        "<rect class=\"port\" x=\"{:.1}\" y=\"{:.1}\" width=\"{side:.1}\" \
         height=\"{side:.1}\" rx=\"{corner:.1}\"/>",
        at.0 - side / 2.0,
        at.1 - side / 2.0
    )
    .unwrap();

    // `max` keeps the direction finite when the two boxes somehow coincide
    let (dx, dy) = (toward.0 - at.0, toward.1 - at.1);
    let length = dx.hypot(dy).max(f64::EPSILON);
    let (ux, uy) = (dx / length, dy / length);
    // just past the port and growing away from the box, so a long name
    // cannot fall back across the border it belongs to
    let anchor = if ux > 0.3 {
        "start"
    } else if ux < -0.3 {
        "end"
    } else {
        "middle"
    };
    if let Some(direction) = direction {
        direction_arrow(out, at, (ux, uy), direction, side);
    }
    beside_with(
        out,
        at,
        (ux, uy),
        0.45 * style.line_height,
        anchor,
        style,
        name,
    );
}

/// The arrow the standard draws inside a port's square -- `pdh` on a left
/// or right border, `pdv` on a top or bottom one. `out` runs the way the
/// port faces, `in` runs back into the box it belongs to, and `inout` is
/// the one shaft with a head at each end.
fn direction_arrow(out: &mut String, at: (f64, f64), away: (f64, f64), direction: &str, side: f64) {
    let (reach, head) = (0.42 * side, 0.3 * side);
    let (fx, fy) = if direction == "in" {
        (-away.0, -away.1)
    } else {
        away
    };
    let tip = (at.0 + fx * reach, at.1 + fy * reach);
    let tail = (at.0 - fx * reach, at.1 - fy * reach);
    writeln!(
        out,
        "<path class=\"rule\" d=\"M {:.1} {:.1} L {:.1} {:.1}\"/>",
        tail.0, tail.1, tip.0, tip.1
    )
    .unwrap();
    arrowhead(out, tip, (fx, fy), head);
    if direction == "inout" {
        arrowhead(out, tail, (-fx, -fy), head);
    }
}

/// An open arrowhead at `tip`, opening back against the way it points.
fn arrowhead(out: &mut String, tip: (f64, f64), (ux, uy): (f64, f64), size: f64) {
    let (px, py) = (-uy * size * 0.7, ux * size * 0.7);
    writeln!(
        out,
        "<path class=\"tip\" d=\"M {:.1} {:.1} L {:.1} {:.1} L {:.1} {:.1}\"/>",
        tip.0 - ux * size + px,
        tip.1 - uy * size + py,
        tip.0,
        tip.1,
        tip.0 - ux * size - px,
        tip.1 - uy * size - py,
    )
    .unwrap();
}

/// Set `text` beside the point `at`, offset to one side of the line running
/// toward `toward`.
fn beside(out: &mut String, at: (f64, f64), toward: (f64, f64), text: &str, style: &Style) {
    let (dx, dy) = (toward.0 - at.0, toward.1 - at.1);
    let length = dx.hypot(dy).max(f64::EPSILON);
    beside_with(
        out,
        at,
        (dx / length, dy / length),
        0.0,
        "middle",
        style,
        text,
    );
}

/// Shared placement: `along` the given direction from `at`, then off to one
/// side of it.
fn beside_with(
    out: &mut String,
    at: (f64, f64),
    (ux, uy): (f64, f64),
    along: f64,
    anchor: &str,
    style: &Style,
    text: &str,
) {
    let across = -0.7 * style.line_height;
    writeln!(
        out,
        "<text class=\"feature\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"{anchor}\" \
         dominant-baseline=\"middle\">{}</text>",
        at.0 + ux * along - uy * across,
        at.1 + uy * along + ux * across,
        escape(text)
    )
    .unwrap();
}

fn centre_of(rect: &Placed) -> (f64, f64) {
    (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
}

/// Where the segment from the centre of `rect` toward `target` crosses the
/// rectangle's border, so a line between two boxes stops at their edges
/// instead of running underneath them.
fn border_point(rect: &Placed, target: (f64, f64)) -> (f64, f64) {
    let (cx, cy) = centre_of(rect);
    let (dx, dy) = (target.0 - cx, target.1 - cy);
    let horizontal = rect.width / 2.0 / dx.abs();
    let vertical = rect.height / 2.0 / dy.abs();
    let scale = horizontal.min(vertical);
    if scale.is_finite() {
        (cx + dx * scale, cy + dy * scale)
    } else {
        // the two centres coincide: there is no direction to clip along
        (cx, cy)
    }
}

/// Escape the five characters that cannot appear literally in XML text.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::resolved;
    use crate::{definition_diagram, interconnection_diagram, layout, render, Node};
    use sysml_model::{ElementKind, Model, Value};

    fn svg_of(source: &str) -> String {
        let ws = resolved(source);
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        render(&diagram, &Style::default())
    }

    #[test]
    fn draws_a_box_per_definition_and_a_line_per_specialization() {
        let svg = svg_of("part def A;\npart def B :> A;\n");
        assert_eq!(svg.matches("<rect class=\"box\"").count(), 2);
        assert_eq!(svg.matches("<line class=\"edge\"").count(), 1);
        assert!(svg.contains("marker-end=\"url(#specialization)\""));
        assert!(svg.contains("\u{ab}part def\u{bb}"));
    }

    #[test]
    fn the_arrowhead_points_at_the_supertype() {
        let ws = resolved("part def Super;\npart def Sub :> Super;\n");
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let style = Style::default();
        let placed = layout(&diagram, &style);
        let svg = to_svg(&diagram, &placed, &style);

        let sub = diagram.nodes.iter().position(|n| n.name == "Sub").unwrap();
        let sup = diagram
            .nodes
            .iter()
            .position(|n| n.name == "Super")
            .unwrap();
        let (sub, sup) = (&placed.placed[sub], &placed.placed[sup]);
        // the line starts on the subtype's top edge and ends on the
        // supertype's bottom edge, where the marker draws the triangle
        assert!(svg.contains(&format!("y1=\"{:.1}\"", sub.y)));
        assert!(svg.contains(&format!("y2=\"{:.1}\"", sup.y + sup.height)));
    }

    #[test]
    fn composition_is_drawn_with_a_filled_diamond_at_the_whole() {
        let svg = svg_of(
            "part def Engine;\n\
             part def Vehicle {\n\
             	part eng : Engine;\n\
             }\n",
        );
        assert_eq!(svg.matches("marker-start=\"url(#composition)\"").count(), 1);
        assert!(!svg.contains("marker-end=\"url(#specialization)\""));
        assert!(svg.contains("class=\"diamond\""));
    }

    #[test]
    fn a_composition_line_stops_on_both_borders() {
        let ws = resolved("part def Engine;\npart def Vehicle {\n\tpart eng : Engine;\n}\n");
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let style = Style::default();
        let placed = layout(&diagram, &style);
        let edge = &diagram.edges[0];
        let (whole, part) = (&placed.placed[edge.from], &placed.placed[edge.to]);

        let (x1, y1) = border_point(whole, centre_of(part));
        // the two boxes share a row, so the line leaves through a side
        assert!((x1 - whole.x).abs() < f64::EPSILON || (x1 - (whole.x + whole.width)).abs() < 0.1);
        assert!(y1 >= whole.y && y1 <= whole.y + whole.height);
        assert!(to_svg(&diagram, &placed, &style).contains(&format!("x1=\"{x1:.1}\"")));
        assert!(y1.is_finite());
    }

    #[test]
    fn an_abstract_name_is_set_in_italic() {
        let svg = svg_of("abstract part def PowerSource;\npart def Engine;\n");
        assert!(svg.contains("class=\"name abstract\""));
        assert_eq!(svg.matches("class=\"name\"").count(), 1);
        assert!(svg.contains(".abstract { font-style: italic; }"));
    }

    #[test]
    fn a_line_that_would_run_under_a_box_steps_around_it() {
        // `chs` and `rb` end up at opposite ends of the row, with the two
        // boxes of `Wheel`/`LugBolt` between them
        let svg = svg_of(
            "part def LugBolt;\n\
             part def Wheel { part lb : LugBolt; }\n\
             part def RollBar;\n\
             part def Chassis {\n\
             	part w : Wheel;\n\
             	part rb : RollBar;\n\
             }\n",
        );
        // the detour is a three-segment path, not a straight line
        assert!(svg.contains("<path class=\"edge\" fill=\"none\" d=\"M "));
        assert!(svg.contains(" V "));
        assert!(svg.contains(" H "));
        // and it still carries the diamond on the side of the whole
        assert_eq!(svg.matches("marker-start=\"url(#composition)\"").count(), 3);
    }

    #[test]
    fn a_specialization_that_would_run_under_a_box_steps_around_it() {
        let ws = resolved("part def Super;\npart def Sub :> Super;\npart def Blocker;\n");
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let style = Style::default();
        // placed by hand: the straight line from `Sub` up to `Super` would
        // pass through `Blocker`, but the gap under `Super`'s row is clear
        let placed = Layout {
            placed: vec![
                place(0, 0.0, 0.0, 100.0, 50.0),
                place(1, 400.0, 200.0, 100.0, 50.0),
                place(2, 120.0, 100.0, 300.0, 50.0),
            ],
            width: 520.0,
            height: 270.0,
            routes: Vec::new(),
        };
        let svg = to_svg(&diagram, &placed, &style);
        assert!(svg.contains("<path class=\"edge\" fill=\"none\" d=\"M 450.0 200.0 V "));
        assert!(svg.contains("marker-end=\"url(#specialization)\""));
        assert!(!svg.contains("<line class=\"edge\""));
    }

    /// The `d` of the first edge drawn as a path, the markers in the
    /// document's own definitions aside.
    fn edge_route(svg: &str) -> Option<&str> {
        svg.split("<path class=\"edge\" fill=\"none\" d=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
    }

    fn place(node: usize, x: f64, y: f64, width: f64, height: f64) -> Placed {
        Placed {
            node,
            x,
            y,
            width,
            height,
        }
    }

    /// Three rows, the middle one so wide that no single gap between the
    /// rows lets a line reach from the bottom row to the top.
    #[test]
    fn a_route_the_engine_chose_is_the_one_drawn() {
        // The renderer works out its own way round a box in the way.
        // Where a layout engine has already said where the line goes,
        // that is what is drawn -- it picked the positions expecting
        // its own bends, and a line cutting straight across them ends
        // up somewhere it left no room for.
        let ws =
            resolved("part def Super;\npart def Sub :> Super;\npart def W { part s : Sub; }\n");
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let style = Style::default();
        let straight = Layout {
            placed: (0..diagram.nodes.len())
                .map(|at| place(at, 0.0, 200.0 * at as f64, 100.0, 50.0))
                .collect(),
            width: 600.0,
            height: 700.0,
            routes: Vec::new(),
        };
        let bend = vec![(50.0, 40.0), (50.0, 120.0), (300.0, 120.0), (300.0, 200.0)];
        let bent = Layout {
            routes: vec![bend; diagram.edges.len()],
            ..straight.clone()
        };

        let drawn = to_svg(&diagram, &bent, &style);
        let spelled = "d=\"M 50.0 40.0 L 50.0 120.0 L 300.0 120.0 L 300.0 200.0\"";
        assert_eq!(
            drawn.matches(spelled).count(),
            diagram.edges.len(),
            "{drawn}"
        );
        // every kind of edge keeps the marker that says what it is
        assert!(
            drawn.contains("marker-end=\"url(#specialization)\""),
            "{drawn}"
        );
        assert!(
            drawn.contains("marker-start=\"url(#composition)\""),
            "{drawn}"
        );
        // without a route the renderer decides for itself, as before
        assert!(!to_svg(&diagram, &straight, &style).contains(spelled));
    }

    fn three_rows() -> Layout {
        Layout {
            placed: vec![
                place(0, 16.0, 16.0, 100.0, 50.0),
                place(1, 16.0, 116.0, 400.0, 50.0),
                place(2, 16.0, 216.0, 100.0, 50.0),
            ],
            width: 452.0,
            height: 300.0,
            routes: Vec::new(),
        }
    }

    #[test]
    fn a_specialization_goes_round_a_row_it_cannot_get_past() {
        let ws = resolved("part def Super;\npart def Blocker;\npart def Sub :> Super;\n");
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let style = Style::default();
        // declared in the order they are placed: `Super`, then `Blocker`
        // across the whole width, then `Sub` beneath it
        let svg = to_svg(&diagram, &three_rows(), &style);

        // five segments: out of `Sub`, along, up the clear column, along
        // again and into `Super`
        let route = edge_route(&svg).expect("the line is drawn as a path");
        assert_eq!(route.matches(" V ").count(), 3, "route: {route}");
        assert_eq!(route.matches(" H ").count(), 2, "route: {route}");
        assert!(!svg.contains("<line class=\"edge\""));
    }

    #[test]
    fn a_connection_goes_round_a_row_it_cannot_get_past() {
        let ws = resolved(
            "port def P;\n\
             part def A { port p : P; }\n\
             part def C { port q : P; }\n\
             part def Blocker;\n\
             part def Top {\n\
             	part a : A;\n\
             	part b : Blocker;\n\
             	part c : C;\n\
             	connect a.p to c.q;\n\
             }\n",
        );
        let model = ws.model();
        let top = model
            .descendants(ws.root())
            .into_iter()
            .find(|&id| model.name(id) == Some("Top"))
            .expect("Top is in the model");
        let diagram = interconnection_diagram(model, top);
        let style = Style::default();
        let svg = to_svg(&diagram, &three_rows(), &style);
        let route = edge_route(&svg).expect("the connection is drawn as a path");
        assert_eq!(route.matches(" V ").count(), 3, "route: {route}");
        assert_eq!(route.matches(" H ").count(), 2, "route: {route}");
        // the column it goes round by stays on the canvas
        for step in route.split(" H ").skip(1) {
            let column: f64 = step.split(' ').next().unwrap().parse().unwrap();
            assert!((0.0..=452.0).contains(&column), "off the canvas: {column}");
        }
        // and both ports are still named beside their own box, the way
        // `port-label = QualifiedName (':' QualifiedName)?` reads
        assert!(svg.contains(">p : P</text>"), "{svg}");
        assert!(svg.contains(">q : P</text>"), "{svg}");
    }

    #[test]
    fn boxes_in_one_row_have_no_row_to_go_round() {
        let style = Style::default();
        let side_by_side = Layout {
            placed: vec![
                place(0, 16.0, 16.0, 100.0, 50.0),
                place(1, 200.0, 16.0, 100.0, 50.0),
            ],
            width: 320.0,
            height: 100.0,
            routes: Vec::new(),
        };
        let band = band_below(&side_by_side, 0, &style);
        assert_eq!(
            sidestep(
                &side_by_side,
                ((66.0, 66.0), (250.0, 66.0)),
                (band, band),
                &style
            ),
            None
        );
    }

    #[test]
    fn a_detour_is_kept_inside_the_canvas() {
        let ws = resolved(
            "part def LugBolt;\n\
             part def Wheel { part lb : LugBolt; }\n\
             part def RollBar;\n\
             part def Chassis {\n\
             	part w : Wheel;\n\
             	part rb : RollBar;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let style = Style::default();
        let placed = layout(&diagram, &style);
        let svg = to_svg(&diagram, &placed, &style);

        let height: f64 = svg
            .split("height=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .and_then(|value| value.parse().ok())
            .expect("the canvas states a height");
        let channel: f64 = svg
            .split(" V ")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .and_then(|value| value.parse().ok())
            .expect("the detour states a channel");
        assert!(channel < height, "the detour runs off the canvas");
        assert!(height >= placed.height);
    }

    #[test]
    fn a_line_beside_a_box_is_left_straight() {
        // one part, so nothing can be in the way
        let svg = svg_of("part def Engine;\npart def Vehicle { part eng : Engine; }\n");
        assert!(!svg.contains("<path class=\"edge\""));
        assert!(svg.contains("<line class=\"edge\""));
    }

    #[test]
    fn a_box_beside_a_line_is_not_a_crossing() {
        let rect = Placed {
            node: 0,
            x: 10.0,
            y: 10.0,
            width: 40.0,
            height: 40.0,
        };
        // straight through the middle
        assert!(crosses(&rect, (0.0, 30.0), (100.0, 30.0)));
        // parallel and clear of it, on either side
        assert!(!crosses(&rect, (0.0, 100.0), (100.0, 100.0)));
        assert!(!crosses(&rect, (0.0, 0.0), (100.0, 0.0)));
        assert!(!crosses(&rect, (100.0, 0.0), (100.0, 100.0)));
        // ending short of it, and starting past it
        assert!(!crosses(&rect, (0.0, 30.0), (5.0, 30.0)));
        assert!(!crosses(&rect, (60.0, 30.0), (100.0, 30.0)));
        // grazing the border does not count
        assert!(!crosses(&rect, (0.0, 10.0), (100.0, 10.0)));
    }

    #[test]
    fn subtypes_of_one_supertype_arrive_at_different_points() {
        let ws = resolved(
            "part def Vehicle;\n\
             part def Car :> Vehicle;\n\
             part def Truck :> Vehicle;\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let spread = arrivals(&diagram);
        let mut landings: Vec<f64> = diagram
            .edges
            .iter()
            .zip(&spread)
            .filter(|(edge, _)| edge.relation == Relation::Specialization)
            .map(|(_, &at)| at)
            .collect();
        assert_eq!(landings.len(), 2);
        landings.sort_by(f64::total_cmp);
        assert_eq!(landings, vec![1.0 / 3.0, 2.0 / 3.0]);
    }

    #[test]
    fn anything_but_a_specialization_arrives_at_the_middle() {
        let ws = resolved("part def Engine;\npart def Vehicle { part eng : Engine; }\n");
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        assert_eq!(arrivals(&diagram), vec![0.5]);
    }

    #[test]
    fn a_port_name_is_written_away_from_its_own_box() {
        let ws = resolved(
            "port def P;\n\
             part def A { port p : P; }\n\
             part def B { port q : P; }\n\
             part def Top {\n\
             	part a : A;\n\
             	part b : B;\n\
             	connect a.p to b.q;\n\
             }\n",
        );
        let top = ws
            .model()
            .descendants(ws.root())
            .into_iter()
            .find(|&id| ws.model().name(id) == Some("Top"))
            .expect("Top is in the model");
        let diagram = interconnection_diagram(ws.model(), top);
        let style = Style::default();
        let svg = to_svg(&diagram, &layout(&diagram, &style), &style);
        // the standard puts a port on a side and its name outside
        // that side, so a left-hand port reads leftward and a
        // right-hand one rightward
        assert!(svg.contains("text-anchor=\"end\""), "{svg}");
    }

    #[test]
    fn a_port_on_a_horizontal_border_keeps_its_name_centred() {
        // a line arriving from below has no side to grow away along, so the
        // name sits over the port instead
        let mut out = String::new();
        port(
            &mut out,
            (50.0, 20.0),
            (50.0, 90.0),
            "p",
            None,
            &Style::default(),
        );
        assert!(out.contains("text-anchor=\"middle\""));
    }

    /// The tips a port's direction arrow draws, and which way each points:
    /// one head for `in` or `out`, one at each end for `inout`.
    fn tips(direction: &str) -> Vec<f64> {
        let mut out = String::new();
        // a port on the left border of a box that lies to the right, so
        // the way it faces is -x and `in` runs back into the box
        port(
            &mut out,
            (50.0, 20.0),
            (42.0, 20.0),
            "p",
            Some(direction),
            &Style::default(),
        );
        out.lines()
            .filter(|line| line.contains("class=\"tip\""))
            .map(|line| {
                // the head's own point is the middle of its three, and it
                // sits ahead of the shaft on the side it points to
                let x: Vec<f64> = line
                    .split(char::is_whitespace)
                    .filter_map(|word| word.parse().ok())
                    .collect();
                x[2] - x[0]
            })
            .collect()
    }

    #[test]
    fn a_parameter_sits_on_the_border_rounded() {
        // `param-l | param-r | param-t | param-b` against `port-l ...`:
        // an action's parameters are drawn on its border the way a part's
        // ports are, and rounded rather than square
        let ws = resolved(
            "attribute def Temp;\n\
             action def Heat {\n\
             \tin item cold : Temp;\n\
             \tout item hot : Temp;\n\
             }\n\
             part def Kettle { action h : Heat; }\n",
        );
        let kettle = ws
            .named_elements()
            .find(|(_, name)| *name == "Kettle")
            .map(|(id, _)| id)
            .unwrap();
        let diagram = interconnection_diagram(ws.model(), kettle);
        let svg = render(&diagram, &Style::default());
        let glyphs: Vec<&str> = svg
            .lines()
            .filter(|line| line.contains("class=\"port\""))
            .collect();
        assert_eq!(glyphs.len(), 2, "{svg}");
        assert!(
            glyphs.iter().all(|glyph| !glyph.contains("rx=\"0.0\"")),
            "a parameter is not square: {glyphs:?}"
        );
        assert!(svg.contains(">cold : Temp</text>"), "{svg}");
        assert!(svg.contains(">hot : Temp</text>"), "{svg}");
    }

    #[test]
    fn a_port_draws_the_direction_it_was_declared_with() {
        assert_eq!(tips("out").len(), 1);
        assert_eq!(tips("in").len(), 1);
        assert_eq!(tips("inout").len(), 2);
        // `out` faces away from the box, `in` back into it, and `inout`
        // does both from the one shaft
        assert!(tips("out")[0] < 0.0);
        assert!(tips("in")[0] > 0.0);
        let both = tips("inout");
        assert!(both[0] < 0.0 && both[1] > 0.0);
    }

    #[test]
    fn lane_spacing_shrinks_to_fit_the_boxes() {
        let style = Style::default();
        let boxed = Placed {
            node: 0,
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 84.0,
        };

        // a couple of edges fit at the preferred spacing
        assert_eq!(lane_spacing(&boxed, &boxed, 1, &style), style.line_height);
        assert_eq!(lane_spacing(&boxed, &boxed, 2, &style), style.line_height);

        // six do not, so they close up rather than run off the borders
        // they have to leave through
        let tight = lane_spacing(&boxed, &boxed, 6, &style);
        assert!(tight < style.line_height);
        assert!(5.0 * tight < boxed.height);
    }

    #[test]
    fn coincident_centres_clip_to_the_centre() {
        let rect = Placed {
            node: 0,
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        assert_eq!(border_point(&rect, centre_of(&rect)), (5.0, 5.0));
    }

    #[test]
    fn a_connection_is_drawn_as_a_plain_line() {
        let ws = resolved(
            "part def Wheel { port hub; }\n\
             part def Axle { port mount; }\n\
             part def Car {\n\
             \tpart w : Wheel;\n\
             \tpart a : Axle;\n\
             \tconnect w.hub to a.mount;\n\
             }\n",
        );
        let car = ws
            .named_elements()
            .find(|(_, name)| *name == "Car")
            .map(|(id, _)| id)
            .unwrap();
        let svg = render(&interconnection_diagram(ws.model(), car), &Style::default());

        // a connection is undirected, so neither end carries a marker
        assert_eq!(svg.matches("<line class=\"edge\"").count(), 1);
        assert!(!svg.contains("marker-start"));
        assert!(!svg.contains("marker-end"));
        assert!(svg.contains(">w : Wheel</text>"));
    }

    #[test]
    fn an_initial_node_is_drawn_as_a_filled_circle() {
        let ws = resolved(
            "state def Modes {\n\
             \tentry; then off;\n\
             \tstate off;\n\
             }\n",
        );
        let modes = ws
            .named_elements()
            .find(|(_, name)| *name == "Modes")
            .map(|(id, _)| id)
            .unwrap();
        let svg = render(
            &interconnection_diagram(ws.model(), modes),
            &Style::default(),
        );

        assert_eq!(svg.matches("<circle class=\"initial\"").count(), 1);
        // the circle carries no label of its own, only the state does
        assert_eq!(svg.matches("<rect class=\"box\"").count(), 1);
        assert!(svg.contains(">off</text>"));
    }

    #[test]
    fn a_named_transition_is_labelled_on_its_arrow() {
        let ws = resolved(
            "state def Modes {\n\
             \tstate off;\n\
             \tstate on;\n\
             \ttransition off_to_on first off then on;\n\
             }\n",
        );
        let modes = ws
            .named_elements()
            .find(|(_, name)| *name == "Modes")
            .map(|(id, _)| id)
            .unwrap();
        let svg = render(
            &interconnection_diagram(ws.model(), modes),
            &Style::default(),
        );
        assert!(svg.contains(">off_to_on</text>"), "{svg}");
    }

    #[test]
    fn a_nested_connection_is_drawn_inside_the_box_that_holds_it() {
        let ws = resolved(
            "port def Hub;\n\
             part def Wheel { port hub : Hub; }\n\
             part def Axle { port mount : Hub; }\n\
             part def Chassis {\n\
             \tpart w : Wheel;\n\
             \tpart a : Axle;\n\
             \tconnect w.hub to a.mount;\n\
             }\n\
             part def Car { part c : Chassis; }\n",
        );
        let car = ws
            .named_elements()
            .find(|(_, name)| *name == "Car")
            .map(|(id, _)| id)
            .unwrap();
        let diagram = interconnection_diagram(ws.model(), car);
        let layout = crate::layout(&diagram, &Style::default());
        let svg = to_svg(&diagram, &layout, &Style::default());
        // the line runs between the two nested boxes, and each end is
        // named by the port it attaches to
        assert!(svg.contains(">hub</text>"), "{svg}");
        assert!(svg.contains(">mount</text>"), "{svg}");
        let chassis = &layout.placed[0];
        for name in ["hub", "mount"] {
            let at = svg.find(&format!(">{name}</text>")).unwrap();
            let x: f64 = svg[..at]
                .rsplit_once("x=\"")
                .and_then(|(_, rest)| rest.split('"').next())
                .and_then(|n| n.parse().ok())
                .unwrap();
            assert!(
                x > chassis.x && x < chassis.x + chassis.width,
                "{name} at {x} is outside the box it belongs to"
            );
        }
    }

    #[test]
    fn a_portion_carries_its_own_filled_marker() {
        let ws = resolved(
            "occurrence def O;\n\
             part def P { snapshot s : O; }\n",
        );
        let svg = render(
            &definition_diagram(ws.model(), &[ws.root()]),
            &Style::default(),
        );
        assert!(svg.contains("url(#portion)"), "{svg}");
        assert!(!svg.contains("url(#composition)"), "{svg}");
    }

    #[test]
    fn a_note_is_drawn_with_its_corner_folded() {
        let ws = resolved(
            "package P {\n\
             \tpart def A;\n\
             \tcomment about A /* Said. */\n\
             }\n",
        );
        let svg = render(
            &definition_diagram(ws.model(), &[ws.root()]),
            &Style::default(),
        );
        // the fold is a rule of its own across the corner, and the line
        // to what it annotates carries nothing at either end
        assert!(svg.contains("<path class=\"box\" d=\"M"), "{svg}");
        assert!(svg.contains(">Said.</text>"), "{svg}");
        assert_eq!(svg.matches("class=\"dependency\"").count(), 1);
    }

    #[test]
    fn a_dependency_is_the_one_dashed_line_in_the_notation() {
        let ws = resolved(
            "package P {\n\
             \tpart def A;\n\
             \tpart def B;\n\
             \tdependency Use from A to B;\n\
             }\n",
        );
        let svg = render(
            &definition_diagram(ws.model(), &[ws.root()]),
            &Style::default(),
        );
        assert_eq!(svg.matches("class=\"dependency\"").count(), 1);
        assert!(svg.contains("stroke-dasharray"), "{svg}");
        assert!(svg.contains(">Use</text>"), "{svg}");
    }

    #[test]
    fn a_satisfaction_is_drawn_the_way_the_standard_draws_one() {
        let ws = resolved(
            "requirement def R;\n\
             part def P;\n\
             package K {\n\
             \trequirement r : R;\n\
             \tpart p : P;\n\
             \tsatisfy r by p;\n\
             }\n",
        );
        let package = ws
            .named_elements()
            .find(|(_, name)| *name == "K")
            .map(|(id, _)| id)
            .unwrap();
        let svg = render(
            &interconnection_diagram(ws.model(), package),
            &Style::default(),
        );

        // `satisfy-edge` is a plain line with the open arrowhead, said
        // in words: the specification draws no dependency here
        assert_eq!(svg.matches("url(#transition)").count(), 1);
        assert!(svg.contains(">\u{ab}satisfy\u{bb}</text>"), "{svg}");
    }

    #[test]
    fn a_succession_is_dashed_where_a_transition_is_not() {
        let ws = resolved(
            "state def Modes {\n\
             \tstate off;\n\
             \tstate on;\n\
             \ttransition off_to_on first off then on;\n\
             \tsuccession on then off;\n\
             }\n",
        );
        let modes = ws
            .named_elements()
            .find(|(_, name)| *name == "Modes")
            .map(|(id, _)| id)
            .unwrap();
        let svg = render(
            &interconnection_diagram(ws.model(), modes),
            &Style::default(),
        );
        assert_eq!(svg.matches("class=\"succession\"").count(), 1, "{svg}");
        assert!(svg.contains("stroke-dasharray: 4 3"), "{svg}");
    }

    #[test]
    fn a_transition_is_drawn_with_an_open_arrowhead() {
        let ws = resolved(
            "state def Modes {\n\
             \tstate off;\n\
             \tstate on;\n\
             \ttransition first off then on;\n\
             }\n",
        );
        let modes = ws
            .named_elements()
            .find(|(_, name)| *name == "Modes")
            .map(|(id, _)| id)
            .unwrap();
        let svg = render(
            &interconnection_diagram(ws.model(), modes),
            &Style::default(),
        );

        assert_eq!(svg.matches("marker-end=\"url(#transition)\"").count(), 1);
        assert!(svg.contains("class=\"tip\""));
        // a transition is directed, so nothing is drawn at its source
        assert!(!svg.contains("marker-start"));
    }

    #[test]
    fn parallel_connections_are_separated_and_labelled() {
        let ws = resolved(
            "part def Wheel { port hub; port rim; }\n\
             part def Axle { port mount; port brace; }\n\
             part def Car {\n\
             \tpart w : Wheel;\n\
             \tpart a : Axle;\n\
             \tconnect w.hub to a.mount;\n\
             \tconnect w.rim to a.brace;\n\
             }\n",
        );
        let car = ws
            .named_elements()
            .find(|(_, name)| *name == "Car")
            .map(|(id, _)| id)
            .unwrap();
        let svg = render(&interconnection_diagram(ws.model(), car), &Style::default());

        let lines: Vec<&str> = svg
            .lines()
            .filter(|line| line.starts_with("<line class=\"edge\""))
            .collect();
        assert_eq!(lines.len(), 2);
        // the pair shares both boxes, so only the lane offset keeps them apart
        assert_ne!(lines[0], lines[1]);

        for port in ["hub", "mount", "rim", "brace"] {
            assert!(svg.contains(&format!(">{port}</text>")), "{svg}");
        }
    }

    #[test]
    fn a_port_is_drawn_on_the_border_it_belongs_to() {
        // `part-def = part-def-name-compartment interconnection-view
        // compartment-stack port-l* port-r* port-t* port-b*` -- a port
        // only listed in a compartment is one a reader cannot see
        // anything connect to.
        let svg = svg_of(
            "port def Fuel;\n\
             port def Air;\n\
             part def Engine {\n\
             \tport fuelIn : Fuel;\n\
             \tport airIn : Air;\n\
             }\n",
        );
        assert_eq!(svg.matches("<rect class=\"port\"").count(), 2, "{svg}");
        // each is named beside itself, and still listed in the stack
        assert!(svg.contains(">fuelIn : Fuel</text>"), "{svg}");
        assert!(svg.contains(">airIn : Air</text>"), "{svg}");
        assert!(svg.contains(">port fuelIn : Fuel</text>"), "{svg}");
        // and the two sit on opposite edges rather than on top of one
        // another
        let squares: Vec<&str> = svg.matches("<rect class=\"port\"").collect();
        assert_eq!(squares.len(), 2);
        let first = svg.find("<rect class=\"port\"").unwrap();
        let second = svg[first + 1..].find("<rect class=\"port\"").unwrap() + first + 1;
        assert_ne!(&svg[first..first + 40], &svg[second..second + 40]);
    }

    #[test]
    fn each_compartment_gets_a_rule_a_label_and_one_line_each() {
        let svg = svg_of(
            "part def FuelPort;\n\
             part def Engine {\n\
             	attribute power;\n\
             	port fuelIn : FuelPort;\n\
             }\n",
        );
        // one rule and one label per compartment: the standard names
        // every compartment after what it holds
        assert_eq!(svg.matches("<line class=\"rule\"").count(), 2);
        assert!(svg.contains(">attributes</text>"), "{svg}");
        assert!(svg.contains(">ports</text>"), "{svg}");
        assert!(svg.contains(">attribute power</text>"));
        assert!(svg.contains(">port fuelIn : FuelPort</text>"));
    }

    #[test]
    fn nested_parts_are_drawn_as_boxes_inside_their_parent() {
        let ws = resolved(
            "part def Bolt;\n\
             part def Wheel { part bolt : Bolt; }\n\
             part def Car {\n\
             \tpart w : Wheel;\n\
             }\n",
        );
        let car = ws
            .named_elements()
            .find(|(_, name)| *name == "Car")
            .map(|(id, _)| id)
            .unwrap();
        let svg = render(&interconnection_diagram(ws.model(), car), &Style::default());

        // the part and the sub-part it holds
        assert_eq!(svg.matches("<rect class=\"box\"").count(), 2);
        assert!(svg.contains(">w : Wheel</text>"));
        assert!(svg.contains(">bolt : Bolt</text>"));
    }

    #[test]
    fn a_definition_without_features_gets_no_rule() {
        let svg = svg_of("part def Bare;\n");
        assert!(!svg.contains("class=\"rule\""));
        assert_eq!(svg.matches("<rect class=\"box\"").count(), 1);
    }

    #[test]
    fn the_document_carries_its_own_palette_and_size() {
        let svg = svg_of("part def A;\n");
        assert!(svg.contains("prefers-color-scheme: dark"));
        assert!(svg.contains("viewBox=\"0 0 "));
        assert!(svg.contains("font-family="));
    }

    #[test]
    fn markup_in_names_is_escaped() {
        let mut model = Model::new();
        let definition = model.create(ElementKind::PartDefinition);
        model.set(
            definition,
            "declaredName",
            Value::String("A<B>&\"C\"'D'".to_string()),
        );
        let diagram = definition_diagram(&model, &[definition]);
        let svg = render(&diagram, &Style::default());

        assert!(svg.contains("A&lt;B&gt;&amp;&quot;C&quot;&apos;D&apos;"));
        assert!(!svg.contains("A<B>"));
    }

    #[test]
    fn an_empty_diagram_still_renders_a_valid_document() {
        let svg = render(&Diagram::default(), &Style::default());
        assert!(svg.starts_with("<svg xmlns="));
        assert!(svg.ends_with("</svg>\n"));
        assert!(!svg.contains("<rect"));
    }

    #[test]
    fn escaping_leaves_ordinary_text_alone() {
        assert_eq!(escape("plain text 123"), "plain text 123");
        assert_eq!(escape(""), "");
    }

    #[test]
    fn every_box_carries_its_node_label() {
        let ws = resolved("part def Alpha;\npart def Beta;\n");
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let svg = render(&diagram, &Style::default());
        let names: Vec<&Node> = diagram.nodes.iter().collect();
        for node in names {
            assert!(svg.contains(&format!(">{}</text>", node.name)));
        }
    }
}

#[cfg(test)]
mod port_tests {
    use super::*;
    use crate::tests::resolved;
    use crate::{interconnection_diagram, layout, render};

    fn car() -> String {
        let ws = resolved(
            "part def Wheel { port hub; }\n\
             part def Axle { port mount; }\n\
             part def Car {\n\
             \tpart w : Wheel;\n\
             \tpart a : Axle;\n\
             \tconnect w.hub to a.mount;\n\
             }\n",
        );
        let car = ws
            .named_elements()
            .find(|(_, name)| *name == "Car")
            .map(|(id, _)| id)
            .unwrap();
        render(&interconnection_diagram(ws.model(), car), &Style::default())
    }

    #[test]
    fn a_connection_ends_at_a_port_square_on_each_border() {
        let svg = car();
        assert_eq!(svg.matches("<rect class=\"port\"").count(), 2);
        assert!(svg.contains(">hub</text>") && svg.contains(">mount</text>"));
    }

    #[test]
    fn ports_are_drawn_after_the_boxes_they_sit_on() {
        let svg = car();
        // a port straddles the border, so a box painted over it would cut
        // it in half
        let last_box = svg.rfind("<rect class=\"box\"").unwrap();
        let first_port = svg.find("<rect class=\"port\"").unwrap();
        assert!(first_port > last_box, "ports must come last");
    }

    #[test]
    fn the_two_names_of_one_connection_sit_on_opposite_sides() {
        let ws = resolved(
            "part def Wheel { port hub; }\n\
             part def Axle { port mount; }\n\
             part def Car {\n\
             \tpart w : Wheel;\n\
             \tpart a : Axle;\n\
             \tconnect w.hub to a.mount;\n\
             }\n",
        );
        let car = ws
            .named_elements()
            .find(|(_, name)| *name == "Car")
            .map(|(id, _)| id)
            .unwrap();
        let diagram = interconnection_diagram(ws.model(), car);
        let style = Style::default();
        let placed = layout(&diagram, &style);
        let svg = to_svg(&diagram, &placed, &style);

        let y_of = |name: &str| -> f64 {
            let at = svg.find(&format!(">{name}</text>")).unwrap();
            let head = &svg[..at];
            let start = head.rfind("y=\"").unwrap() + 3;
            head[start..].split('"').next().unwrap().parse().unwrap()
        };
        // each name sits beside its own port, on the border of the box
        // that declares it, rather than beside the line between them
        let x_of = |name: &str| -> f64 {
            let at = svg.find(&format!(">{name}</text>")).unwrap();
            let head = &svg[..at];
            let start = head.rfind("x=\"").unwrap() + 3;
            head[start..].split('"').next().unwrap().parse().unwrap()
        };
        // each sits nearer the box that declares it than the other one
        let near = |name: &str, at: usize| {
            let box_ = &placed.placed[at];
            let other = &placed.placed[1 - at];
            let mine = (x_of(name) - (box_.x + box_.width / 2.0)).abs();
            let theirs = (x_of(name) - (other.x + other.width / 2.0)).abs();
            assert!(mine < theirs, "`{name}` is written beside the wrong box");
        };
        near("hub", 0);
        near("mount", 1);
        // and neither is written inside the box it sits on
        let wheel = &placed.placed[0];
        assert!(
            x_of("hub") <= wheel.x || x_of("hub") >= wheel.x + wheel.width,
            "hub is written outside its box"
        );
        let _ = y_of("hub");
    }
}

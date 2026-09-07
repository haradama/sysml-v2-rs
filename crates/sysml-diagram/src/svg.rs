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
.lane { fill: none; stroke: var(--line); stroke-width: 1; }\n\
.name { fill: var(--text); font-weight: bold; }\n\
.abstract { font-style: italic; }\n\
.keyword, .feature { fill: var(--muted); }\n\
.aside { fill: var(--muted); paint-order: stroke; stroke: var(--box); stroke-width: 3; \
stroke-linejoin: round; }\n\
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
    // the package frames first, so every box and line sits on top of them
    for frame in &layout.packages {
        let tab = style.package_tab();
        // The tab widens as it descends, and is closed along the bottom,
        // so it reads as a tab of its own rather than as a step in the
        // outline -- the folder as PlantUML and the UML tools before it
        // have always drawn it.
        let slant = 0.3 * tab;
        let notch = (style.text_width(&frame.name) + 2.0 * style.padding)
            .min(frame.width - slant)
            .max(0.0);
        writeln!(
            out,
            "<path class=\"box\" d=\"M {:.1} {:.1} H {:.1} L {:.1} {:.1} H {:.1} V {:.1} \
             H {:.1} z\"/>\n\
             <line class=\"rule\" x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\"/>",
            frame.x,
            frame.y,
            frame.x + notch,
            frame.x + notch + slant,
            frame.y + tab,
            frame.x + frame.width,
            frame.y + frame.height,
            frame.x,
            frame.x,
            frame.y + tab,
            frame.x + notch + slant,
            frame.y + tab,
        )
        .unwrap();
        writeln!(
            out,
            "<text class=\"name\" x=\"{:.1}\" y=\"{:.1}\">{}</text>",
            frame.x + style.padding,
            frame.y + 0.5 * style.padding + 0.75 * style.line_height,
            escape(&frame.name)
        )
        .unwrap();
    }
    // the swimlanes next, so every box and line sits on top of them
    for column in &layout.lanes {
        writeln!(
            out,
            "<rect class=\"lane\" x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" \
             height=\"{:.1}\"/>\n\
             <line class=\"rule\" x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\"/>",
            column.x,
            column.top,
            column.width,
            column.height,
            column.x,
            column.top + 2.0 * style.padding + style.line_height,
            column.x + column.width,
            column.top + 2.0 * style.padding + style.line_height,
        )
        .unwrap();
        writeln!(
            out,
            "<text class=\"name\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{}</text>",
            column.x + column.width / 2.0,
            column.top + style.padding + 0.75 * style.line_height,
            escape(&column.name)
        )
        .unwrap();
    }

    // edges first, so the boxes paint over the line ends. Ports sit on
    // those borders and must survive, so they are held back until after.
    let mut ports = String::new();
    // where a line that did not run straight left the box, for the ports
    // drawn on those borders once every line is down
    let mut landings: Vec<Leaving> = Vec::new();
    // where each port's square came to rest, so that the second line to
    // name a port is drawn to the same square as the first rather than
    // to a point of its own that nothing is drawn at
    let mut anchors: Vec<Anchor> = Vec::new();
    // how far down a detour reached, so the canvas can grow to hold it
    let mut floor = 0.0_f64;
    // Every run of every line, and the names written along them. A name
    // is held back until all the lines are down, because where it reads
    // best depends on what else was drawn.
    let mut drawn: Vec<Leg> = Vec::new();
    let mut asides: Vec<Aside> = Vec::new();
    // the names already written over the drawing -- the ones beside the
    // ports and the ones along the lines alike -- so that no two of them
    // are put on one another
    let mut written: Vec<Placed> = Vec::new();
    let lanes = lanes(diagram);
    let arrivals = arrivals(diagram);
    let departures = departures(diagram);
    // Where every box's ports sit, worked out once. A port is asked
    // after twice per line that names one and again when the box itself
    // is drawn, and the answer cannot change between one and the next.
    let placements: Vec<Vec<Placement>> = (0..layout.placed.len())
        .map(|at| port_places(diagram, layout, at, &lanes, style))
        .collect();
    for (index, (edge, &(lane, siblings))) in diagram.edges.iter().zip(&lanes).enumerate() {
        let from = &layout.placed[edge.from];
        let to = &layout.placed[edge.to];
        // the path the engine chose for this line, if it chose one: read
        // once, and the same answer for whichever way the line is drawn
        let routed = given(layout, index);
        match edge.relation {
            // the layering already put the supertype above, so the line
            // runs from the subtype's top edge to the supertype's bottom
            Relation::Specialization if routed.is_some() => {
                let walked = routed.expect("the arm this route matched");
                note(&mut drawn, walked);
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
                // only these two: a channel above the supertype's own
                // row would have to come back down through the
                // supertype to reach its bottom border
                let channel = blocked
                    .then(|| channel_for(layout, ((x1, y1), (x2, y2)), &[bands.1, bands.0]))
                    .flatten();
                match channel {
                    Some(channel) => {
                        note(
                            &mut drawn,
                            &[(x1, y1), (x1, channel), (x2, channel), (x2, y2)],
                        );
                        writeln!(
                            out,
                            "<path class=\"edge\" fill=\"none\" d=\"M {x1:.1} {y1:.1} \
                             V {channel:.1} H {x2:.1} V {y2:.1}\" \
                             marker-end=\"url(#specialization)\"/>"
                        )
                    }
                    // no single gap reaches: go round the rows in between
                    None => match blocked
                        .then(|| sidestep(layout, ((x1, y1), (x2, y2)), bands, style))
                        .flatten()
                    {
                        Some(column) => {
                            note(
                                &mut drawn,
                                &[
                                    (x1, y1),
                                    (x1, bands.0),
                                    (column, bands.0),
                                    (column, bands.1),
                                    (x2, bands.1),
                                    (x2, y2),
                                ],
                            );
                            writeln!(
                                out,
                                "<path class=\"edge\" fill=\"none\" d=\"M {x1:.1} {y1:.1} \
                                 V {:.1} H {column:.1} V {:.1} H {x2:.1} V {y2:.1}\" \
                                 marker-end=\"url(#specialization)\"/>",
                                bands.0, bands.1
                            )
                        }
                        None => {
                            note(&mut drawn, &[(x1, y1), (x2, y2)]);
                            writeln!(
                                out,
                                "<line class=\"edge\" x1=\"{x1:.1}\" y1=\"{y1:.1}\" \
                                 x2=\"{x2:.1}\" y2=\"{y2:.1}\" \
                                 marker-end=\"url(#specialization)\"/>"
                            )
                        }
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
            Relation::Composition | Relation::Reference | Relation::Portion
                if edge.from == edge.to =>
            {
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
                // A feature typed by what declares it faces no other
                // box, so the border point a square would be worked out
                // from is the box's own middle -- inside it, on no
                // border at all. The loop sets out from the bottom
                // border and comes back to it, and that is where the
                // squares belong.
                let mut legs = [(left, bottom), (right, bottom)];
                for (nth, name) in [&edge.ends.0, &edge.ends.1].into_iter().enumerate() {
                    let (point, toward) = (legs[nth], (legs[nth].0, band));
                    // a port an earlier line already settled keeps the
                    // square that line left it on
                    let Some(name) = name.as_deref().filter(|name| {
                        anchored(&anchors, edge.from, Some(name)).is_none()
                            && port_at(&placements[edge.from], Some(name)).is_some()
                    }) else {
                        continue;
                    };
                    landings.push(Leaving {
                        at: edge.from,
                        name: name.to_string(),
                        point,
                        toward,
                    });
                    anchors.push(Anchor {
                        at: edge.from,
                        name: name.to_string(),
                        point,
                        away: heading(point, toward),
                    });
                    legs[nth] = off_port(point, toward, true, style);
                }
                let ((sx, sy), (ex, ey)) = (legs[0], legs[1]);
                note(&mut drawn, &[(sx, sy), (sx, band), (ex, band), (ex, ey)]);
                writeln!(
                    out,
                    "<path{class} fill=\"none\" d=\"M {sx:.1} {sy:.1} V {band:.1} \
                     H {ex:.1} V {ey:.1}\"{marker}/>"
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
            | Relation::Annotation
            | Relation::Client => {
                // a connection ends at the port it names, where the
                // box declares one: the standard draws the port on the
                // border, and a second square beside it would be a
                // second port that the model never had
                //
                // A port is one square, so a second line naming it is
                // drawn to where the first line left it rather than to a
                // place worked out afresh, which would leave one of the
                // two touching nothing.
                let held = (
                    anchored(&anchors, edge.from, edge.ends.0.as_deref()),
                    anchored(&anchors, edge.to, edge.ends.1.as_deref()),
                );
                let places = (
                    held.0
                        .or_else(|| port_at(&placements[edge.from], edge.ends.0.as_deref())),
                    held.1
                        .or_else(|| port_at(&placements[edge.to], edge.ends.1.as_deref())),
                );
                // the line starts on the square's outer face rather than
                // at its middle, so what it carries there -- a
                // composition's diamond -- sits against the port instead
                // of on top of it
                let clear = port_side(style) / 2.0;
                let outer = |(at, away): ((f64, f64), (f64, f64))| {
                    ((at.0 + away.0 * clear, at.1 + away.1 * clear), away)
                };
                let (((mut x1, mut y1), first_away), first_is_port) = match places.0 {
                    Some(place) => (outer(place), true),
                    None => (facing(from, centre_of(to)), false),
                };
                let (((mut x2, mut y2), second_away), second_is_port) = match places.1 {
                    Some(place) => (outer(place), true),
                    None => (facing(to, centre_of(from)), false),
                };
                // Hold edges sharing a pair of boxes apart, so two
                // connections do not collapse into one line. A line
                // leaves through a border, so what holds two of them
                // apart runs along that border: shifted across the line
                // instead, an end drifts off the box it belongs to --
                // outside it at one end, and inside it at the other.
                //
                // An end on a port has been slid already, by the same
                // amount and for the same reason, so that the square it
                // is drawn as stays on the border it straddles.
                let shift = lane * lane_spacing(from, to, siblings, style);
                if !first_is_port {
                    (x1, y1) = slid(from, (x1, y1), first_away, shift);
                }
                if !second_is_port {
                    (x2, y2) = slid(to, (x2, y2), second_away, shift);
                }
                let (marker, class) = pen(edge.relation);
                // a straight line that runs under an unrelated box reads as
                // a connection to that box, so step around it instead: both
                // boxes are left downward and joined in a clear channel
                // beneath the row, one lane per shared pair
                let lane_shift = lane.abs() * style.line_height;
                let Facing { mut detour, bands } =
                    facing_sides(layout, edge, style, lane_shift, departures[index]);
                // a detour would set out from a border of its own
                // choosing; where the square is already drawn, it sets
                // out from there instead
                if let Some((point, _)) = held.0 {
                    detour.0 = point;
                }
                if let Some((point, _)) = held.1 {
                    detour.1 = point;
                }
                let route = match routed {
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
                // a straight line crosses the border where a port is
                // already drawn; anything else leaves by a side of its
                // own choosing, and the square has to be told where
                let bent = route.is_some();
                // The run a name written on the line is set beside. It
                // has to be the run the name sits on rather than the
                // line as a whole: a detour bends, and a name offset to
                // one side of the line's far end lands back across the
                // run it is written over.
                let (first_toward, second_toward, run) = match route {
                    None => {
                        writeln!(
                            out,
                            "<line{class} x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" \
                             y2=\"{y2:.1}\"{marker}/>"
                        )
                        .unwrap();
                        note(&mut drawn, &[(x1, y1), (x2, y2)]);
                        ((x2, y2), (x1, y1), ((x1, y1), (x2, y2)))
                    }
                    Some(Detour::Channel(channel)) => {
                        ((x1, y1), (x2, y2)) = detour;
                        let (sx, sy) = off_port((x1, y1), (x1, channel), first_is_port, style);
                        let (ex, ey) = off_port((x2, y2), (x2, channel), second_is_port, style);
                        writeln!(
                            out,
                            "<path{class} fill=\"none\" d=\"M {sx:.1} {sy:.1} V {channel:.1} \
                             H {ex:.1} V {ey:.1}\"{marker}/>"
                        )
                        .unwrap();
                        floor = floor.max(channel);
                        note(
                            &mut drawn,
                            &[(sx, sy), (sx, channel), (ex, channel), (ex, ey)],
                        );
                        ((x1, channel), (x2, channel), ((x1, channel), (x2, channel)))
                    }
                    Some(Detour::Given(route)) => {
                        let mut walked = route.to_vec();
                        let last = walked.len() - 1;
                        // the engine routed each line to a border point
                        // of its own, and a port that is already drawn
                        // pulls the ones after the first back to it
                        let start = held.0.map_or(route[0], |(point, _)| point);
                        let finish = held.1.map_or(route[last], |(point, _)| point);
                        walked[0] = off_port(start, route[1], first_is_port, style);
                        walked[last] = off_port(finish, route[last - 1], second_is_port, style);
                        writeln!(
                            out,
                            "<path{class} fill=\"none\" d=\"{}\"{marker}/>",
                            polyline(&walked)
                        )
                        .unwrap();
                        (x1, y1) = start;
                        (x2, y2) = finish;
                        floor = floor.max(walked.iter().map(|&(_, y)| y).fold(0.0, f64::max));
                        let middle = walked.len() / 2;
                        note(&mut drawn, &walked);
                        (
                            route[1],
                            route[last - 1],
                            (walked[middle - 1], walked[middle]),
                        )
                    }
                    Some(Detour::Sidestep(column)) => {
                        ((x1, y1), (x2, y2)) = detour;
                        let (first, second) = bands;
                        let (sx, sy) = off_port((x1, y1), (x1, first), first_is_port, style);
                        let (ex, ey) = off_port((x2, y2), (x2, second), second_is_port, style);
                        writeln!(
                            out,
                            "<path{class} fill=\"none\" d=\"M {sx:.1} {sy:.1} V {first:.1} \
                             H {column:.1} V {second:.1} H {ex:.1} V {ey:.1}\"{marker}/>"
                        )
                        .unwrap();
                        floor = floor.max(first.max(second));
                        note(
                            &mut drawn,
                            &[
                                (sx, sy),
                                (sx, first),
                                (column, first),
                                (column, second),
                                (ex, second),
                                (ex, ey),
                            ],
                        );
                        // the column can be hard against the margin, so the
                        // name is written along the gap the line sets out
                        // in instead
                        ((x1, first), (x2, second), ((x1, first), (column, first)))
                    }
                };
                // A detour leaves both boxes downward and a route leaves
                // by the border it was routed out of, neither of which
                // is the border a straight line to the other box
                // crosses. The square is drawn on that border later, so
                // it is told where the line really left instead.
                //
                // Only the first line to name a port says where its
                // square goes; the lines after it were drawn to that
                // square and have nothing to add.
                for (name, at, held, place, point, toward) in [
                    (
                        &edge.ends.0,
                        edge.from,
                        held.0,
                        places.0,
                        (x1, y1),
                        first_toward,
                    ),
                    (
                        &edge.ends.1,
                        edge.to,
                        held.1,
                        places.1,
                        (x2, y2),
                        second_toward,
                    ),
                ] {
                    let (Some(name), None, Some(place)) = (name.as_deref(), held, place) else {
                        continue;
                    };
                    if bent {
                        landings.push(Leaving {
                            at,
                            name: name.to_string(),
                            point,
                            toward,
                        });
                    }
                    anchors.push(Anchor {
                        at,
                        name: name.to_string(),
                        // a straight line meets the port where the box
                        // itself puts it; a bent one leaves the square
                        // where it left the border
                        point: if bent { point } else { place.0 },
                        away: if bent {
                            heading(point, toward)
                        } else {
                            place.1
                        },
                    });
                }
                // A connection meets each box at a port, drawn the SysML
                // way: a small square on the border, named beside it.
                // Where the box declares that port, it is already
                // drawn there and the line simply arrives at it.
                if let Some(first) = edge.ends.0.as_ref().filter(|_| !first_is_port) {
                    port(
                        &mut ports,
                        &mut written,
                        (x1, y1),
                        first_toward,
                        first,
                        None,
                        style,
                    );
                }
                if let Some(second) = edge.ends.1.as_ref().filter(|_| !second_is_port) {
                    port(
                        &mut ports,
                        &mut written,
                        (x2, y2),
                        second_toward,
                        second,
                        None,
                        style,
                    );
                }
                if let Some(label) = &edge.label {
                    asides.push(Aside {
                        run,
                        text: label.clone(),
                    });
                }
                Ok(())
            }
        }
        .unwrap();
    }

    // Every line is down, so each name can be put where the least of it
    // is covered: a drawing gets dense enough that the spot opposite the
    // middle of a name's own run is taken -- by the leg of another
    // detour, or by a channel running the length of the page.
    let mut taken: Vec<Placed> = layout.placed.clone();
    taken.extend(written.iter().copied());
    for aside in &asides {
        let put = clear_spot(aside, &drawn, &taken, style);
        // a name written under the lowest channel is the lowest thing on
        // the canvas
        floor = floor.max(put.1);
        label_at(&mut out, put, "middle", &aside.text);
        // and it is something every name after it has to keep clear of
        let rect = text_box(put, "middle", &aside.text, style);
        taken.push(rect);
        written.push(rect);
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
        border_ports(
            &mut ports,
            &mut written,
            &placements[at],
            at,
            &landings,
            style,
        );
    }

    out.push_str(&ports);
    let height = layout.height.max(floor + style.margin);
    document(layout.width, height, style, &out)
}

/// How far past a border a name set beside a port is put: out along the
/// way the port faces, then off to one side of the line leaving it.
fn beside_room(style: &Style) -> f64 {
    0.45 * style.line_height + across_of(style).abs()
}

/// The room one box needs outside itself for the names drawn on its
/// border: its ports, its parameters, and the ends of the lines that
/// arrive at it, each of which reads outwards, away from the box.
fn label_room(diagram: &Diagram, at: usize, style: &Style) -> f64 {
    let node = &diagram.nodes[at];
    let named = |name: &String| style.text_width(name);
    let ports = std::iter::once(node)
        .chain(&node.children)
        .flat_map(|held| &held.compartments)
        .filter(|compartment| matches!(compartment.label, "ports" | "parameters"))
        .flat_map(|compartment| &compartment.lines)
        .map(|line| named(&line.name));
    // a connection end names its port whether or not the box declares
    // one, and a nested part's links are drawn inside this box
    let ends = diagram
        .edges
        .iter()
        .flat_map(|edge| {
            [
                (edge.from == at).then_some(&edge.ends.0),
                (edge.to == at).then_some(&edge.ends.1),
            ]
        })
        .chain(
            node.links
                .iter()
                .flat_map(|link| [Some(&link.ends.0), Some(&link.ends.1)]),
        )
        .flatten()
        .flatten()
        .map(named);
    let mut room = ports.chain(ends).fold(0.0f64, f64::max);
    if room > 0.0 {
        room += beside_room(style);
    }
    // a marker -- the dot a flow starts at, the bar it splits on -- is
    // named above itself rather than beside a border, centred, so half
    // the name hangs off each side of it
    if !matches!(node.shape, Shape::Box | Shape::Note) && !node.name.is_empty() {
        room = room.max(named(&node.name) / 2.0 + across_of(style).abs());
    }
    // and what a line between two nested parts says is written across
    // the middle of it, which is inside this box: a transition's trigger
    // and effect can be longer than everything they run between
    for link in &node.links {
        if let Some(label) = &link.label {
            room = room.max(named(label) / 2.0 + across_of(style).abs());
        }
    }
    room
}

/// The same layout with the canvas grown to hold the names drawn outside
/// the boxes, or `None` where it already holds them.
///
/// A port is a square on a border with its name reading outwards, and on
/// the outer border of an outermost box that name reads off the canvas.
/// The engines place boxes and know nothing of the words drawn round
/// them, so the room is made here, where the words are: everything is
/// slid clear of the top left corner and the canvas grows by as much as
/// it was short.
pub(crate) fn with_room_for_labels(
    diagram: &Diagram,
    layout: &Layout,
    style: &Style,
) -> Option<Layout> {
    // a name reads horizontally, so downwards it reaches only as far as
    // it was set from the border, plus the half line it is centred on
    let deep = beside_room(style) + 0.5 * style.font_size;
    let (mut left, mut top, mut right, mut bottom) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for placed in &layout.placed {
        let room = label_room(diagram, placed.node, style);
        if room == 0.0 {
            continue;
        }
        left = left.max(room - placed.x);
        right = right.max(placed.x + placed.width + room - layout.width);
        top = top.max(deep - placed.y);
        bottom = bottom.max(placed.y + placed.height + deep - layout.height);
    }
    // whole pixels, because the canvas is written out rounded to them
    let (left, top) = (left.max(0.0).ceil(), top.max(0.0).ceil());
    let (right, bottom) = (right.max(0.0).ceil(), bottom.max(0.0).ceil());
    if left + top + right + bottom == 0.0 {
        return None;
    }
    let mut grown = layout.clone();
    for placed in &mut grown.placed {
        placed.x += left;
        placed.y += top;
    }
    for route in &mut grown.routes {
        for point in route {
            point.0 += left;
            point.1 += top;
        }
    }
    for lane in &mut grown.lanes {
        lane.x += left;
        lane.top += top;
    }
    for frame in &mut grown.packages {
        frame.x += left;
        frame.y += top;
    }
    grown.width += left + right;
    grown.height += top + bottom;
    Some(grown)
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

/// Which border each box is left through on a detour, and the gap the
/// line runs along after it.
///
/// A line leaves a box on the side that faces the other one: where the
/// layering has put a whole above its parts, it leaves the whole
/// downward and arrives at the part from above. Between boxes in one row
/// there is no such side, and both are left downward -- the one direction
/// the canvas grows to make room in.
fn facing_sides(
    layout: &Layout,
    edge: &Edge,
    style: &Style,
    lane_shift: f64,
    along: (f64, f64),
) -> Facing {
    let (from, to) = (&layout.placed[edge.from], &layout.placed[edge.to]);
    let (middle, other) = (from.x + from.width * along.0, to.x + to.width * along.1);
    if from.y + from.height <= to.y {
        return Facing {
            detour: ((middle, from.y + from.height), (other, to.y)),
            bands: (
                band_below(layout, edge.from, style) + lane_shift,
                band_above(layout, edge.to, style) - lane_shift,
            ),
        };
    }
    if to.y + to.height <= from.y {
        return Facing {
            detour: ((middle, from.y), (other, to.y + to.height)),
            bands: (
                band_above(layout, edge.from, style) - lane_shift,
                band_below(layout, edge.to, style) + lane_shift,
            ),
        };
    }
    Facing {
        detour: ((middle, from.y + from.height), (other, to.y + to.height)),
        bands: (
            band_below(layout, edge.from, style) + lane_shift,
            band_below(layout, edge.to, style) + lane_shift,
        ),
    }
}

/// Where a detour leaves its two boxes, and the gap each end runs along
/// after it.
struct Facing {
    detour: ((f64, f64), (f64, f64)),
    bands: (f64, f64),
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
fn crosses(rect: &Placed, one: (f64, f64), other: (f64, f64)) -> bool {
    through(rect, one, other) > 0.0
}

/// How much of the segment lies inside the rectangle. A line drawn along
/// a name strikes it out where one merely crossing it costs a letter, so
/// what the two are told apart by is how much of the line falls inside.
fn through(rect: &Placed, (x1, y1): (f64, f64), (x2, y2): (f64, f64)) -> f64 {
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
                return 0.0;
            }
        } else if towards < 0.0 {
            enter = enter.max(room / towards);
        } else {
            leave = leave.min(room / towards);
        }
    }
    (leave - enter).max(0.0) * dx.hypot(dy)
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
        // end: which is the note is plain from the shapes. So is
        // `n-ary-dependency-client-link`, since the dot is not what the
        // client depends on.
        Relation::Annotation | Relation::Client => ("", " class=\"dependency\""),
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
    // the names inside one box are written among themselves, and the
    // ones outside it are already down
    let mut written: Vec<Placed> = Vec::new();
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
            port(out, &mut written, first, second, name, None, style);
        }
        if let Some(name) = &link.ends.1 {
            port(out, &mut written, second, first, name, None, style);
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

/// Where along each border a detour sets out from and comes back to, as
/// a fraction of the width.
///
/// Every detour leaving one box would otherwise set out from the middle
/// of it, so two of them pile their markers -- and the squares of any
/// ports they name -- on the one point. They are spread the way subtypes
/// arriving at one supertype are.
fn departures(diagram: &Diagram) -> Vec<(f64, f64)> {
    let spread = |edge: &Edge| edge.relation != Relation::Specialization && edge.from != edge.to;
    let mut total: HashMap<usize, usize> = HashMap::new();
    for edge in diagram.edges.iter().filter(|edge| spread(edge)) {
        *total.entry(edge.from).or_default() += 1;
        *total.entry(edge.to).or_default() += 1;
    }
    let mut taken: HashMap<usize, usize> = HashMap::new();
    diagram
        .edges
        .iter()
        .map(|edge| {
            if !spread(edge) {
                return (0.5, 0.5);
            }
            let mut share = |at: usize| {
                let slot = taken.entry(at).or_default();
                let index = *slot;
                *slot += 1;
                (index as f64 + 1.0) / (total[&at] as f64 + 1.0)
            };
            (share(edge.from), share(edge.to))
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
fn border_ports(
    out: &mut String,
    written: &mut Vec<Placed>,
    places: &[Placement<'_>],
    at: usize,
    landings: &[Leaving],
    style: &Style,
) {
    for place in places {
        // where the line for this port did not run straight it left by
        // a border of its own choosing, and said so
        let bent = landings
            .iter()
            .find(|landing| landing.at == at && landing.name == place.feature.name);
        // `port-label = QualifiedName (':' QualifiedName)?`: the name
        // alone, because every port drawn on a border was read out of
        // the box's own ports compartment, which says the type an inch
        // away. Saying it twice only gives a line more to run through.
        marked(
            out,
            written,
            Marked {
                at: bent.map_or(place.at, |landing| landing.point),
                toward: bent.map_or(place.toward, |landing| landing.toward),
                peer: place.peer,
                name: &place.feature.name,
                direction: place.feature.direction,
                rounded: place.rounded,
            },
            style,
        );
    }
}

/// The port a connection end names, where the box declares one by that
/// name: the middle of the square it is drawn as, and the way out of the
/// box through the border it sits on.
fn port_at(places: &[Placement<'_>], end: Option<&str>) -> Option<((f64, f64), (f64, f64))> {
    let end = end?;
    places
        .iter()
        .find(|place| place.feature.name == end)
        .map(|place| (place.at, place.away))
}

/// Where a port has already been settled, because an earlier line named
/// it: the same pair [`port_at`] gives, so that every line after the
/// first leaves the square rather than a point of its own.
fn anchored(anchors: &[Anchor], at: usize, end: Option<&str>) -> Option<((f64, f64), (f64, f64))> {
    let end = end?;
    anchors
        .iter()
        .find(|anchor| anchor.at == at && anchor.name == end)
        .map(|anchor| (anchor.point, anchor.away))
}

/// How wide the square the standard draws a port as is.
fn port_side(style: &Style) -> f64 {
    0.6 * style.line_height
}

/// Where a line that did not run straight left a box, and which port of
/// it the line was drawn to: what the square is moved onto, once every
/// line is down and the box's own ports come to be drawn.
struct Leaving {
    at: usize,
    name: String,
    point: (f64, f64),
    toward: (f64, f64),
}

/// Where one port's square ended up, so that the lines naming it after
/// the first are drawn to the same point.
///
/// A port is one square on one border. Which point that is depends on
/// how the first line naming it was drawn -- straight, and it is the
/// port's own place on the border; routed, and it is wherever the route
/// left the box -- so the first line settles it and the rest follow.
struct Anchor {
    at: usize,
    name: String,
    point: (f64, f64),
    away: (f64, f64),
}

/// One port on a border: what it is, where it sits, and the point its
/// name reads towards.
struct Placement<'a> {
    feature: &'a Feature,
    at: (f64, f64),
    toward: (f64, f64),
    /// The way out of the box through the border the port sits on.
    away: (f64, f64),
    /// What the line leaving this port reaches, where one does.
    peer: Option<(f64, f64)>,
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
fn port_places<'a>(
    diagram: &'a Diagram,
    layout: &Layout,
    at: usize,
    lanes: &[(f64, usize)],
    style: &Style,
) -> Vec<Placement<'a>> {
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
    // Which port faces a line is settled before any of them is placed:
    // the ones that face nothing are spread evenly down the side they
    // fall on, and how far apart that is depends on how many there are.
    let wired: Vec<Option<(usize, usize)>> = listed
        .iter()
        .map(|(feature, _)| {
            diagram.edges.iter().enumerate().find_map(|(index, edge)| {
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
                (named.as_deref() == Some(feature.name.as_str())).then_some((index, theirs))
            })
        })
        .collect();
    let free = wired.iter().filter(|joined| joined.is_none()).count();

    let mut placements = Vec::new();
    let mut spare = 0usize;
    for ((feature, rounded), joined) in listed.into_iter().zip(wired) {
        let peer = joined.map(|(_, theirs)| centre_of(&layout.placed[theirs]));
        let (point, away) = match joined {
            // Two features of one type face the same point of the
            // border, and the shift that keeps their lines apart has to
            // keep their squares apart too. It runs along the border
            // rather than across the line: that is the one direction a
            // square can be moved in and still straddle the border.
            Some((index, theirs)) => {
                let other = &layout.placed[theirs];
                let (point, away) = facing(placed, centre_of(other));
                let (lane, siblings) = lanes[index];
                let slide = lane * lane_spacing(placed, other, siblings, style);
                (slid(placed, point, away, slide), away)
            }
            None => {
                // they alternate sides, so the left takes the odd one
                // out; each side is then divided into as many equal
                // steps as it has ports, plus one
                let left = spare % 2 == 0;
                let down = placed.y
                    + (spare / 2 + 1) as f64 * placed.height
                        / (if left { free.div_ceil(2) } else { free / 2 } + 1) as f64;
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
            away,
            peer,
            rounded,
        });
    }
    // Two features can face two boxes that lie the same way and come to
    // the same point of the border. Spread whatever has piled up along
    // the border it sits on -- the lines follow, because this is where
    // they are told the port is.
    let points: Vec<(f64, f64)> = placements.iter().map(|place| place.at).collect();
    for pile in piles(&points).into_iter().filter(|pile| pile.len() > 1) {
        let spread = (pile.len() as f64 - 1.0) / 2.0;
        for (nth, &which) in pile.iter().enumerate() {
            let place = &mut placements[which];
            let slide = (nth as f64 - spread) * style.line_height;
            place.at = slid(placed, place.at, place.away, slide);
            place.toward = (
                place.at.0 + place.away.0 * 8.0,
                place.at.1 + place.away.1 * 8.0,
            );
        }
    }
    placements
}

/// How near two points have to be to count as one place. The drawing is
/// written out to a tenth of a pixel, so anything closer than half of one
/// lands on the same spot however the arithmetic came out.
const TOGETHER: f64 = 0.5;

/// The points that came out on top of each other, as groups of indices
/// into `points`.
///
/// Rounding each point into a bucket instead would separate two points a
/// tenth of a pixel apart whenever a bucket boundary happened to fall
/// between them, and leave their squares drawn over one another.
fn piles(points: &[(f64, f64)]) -> Vec<Vec<usize>> {
    let mut piles: Vec<Vec<usize>> = Vec::new();
    for (nth, &at) in points.iter().enumerate() {
        let together = piles.iter_mut().find(|pile| {
            let first = points[pile[0]];
            (first.0 - at.0).abs() < TOGETHER && (first.1 - at.1).abs() < TOGETHER
        });
        match together {
            Some(pile) => pile.push(nth),
            None => piles.push(vec![nth]),
        }
    }
    piles
}

/// An end of a line pulled back to the outer face of the square a port
/// is drawn as, where the end names one.
///
/// The square straddles the border the line sets out from, so a line
/// drawn to the border itself runs under it, and what it carries there
/// -- a composition's diamond, a reference's -- is drawn half inside the
/// square. `at_port` holds a straight line off by the same half square.
fn off_port(at: (f64, f64), toward: (f64, f64), is_port: bool, style: &Style) -> (f64, f64) {
    if is_port {
        along(at, toward, port_side(style) / 2.0)
    } else {
        at
    }
}

/// The point `by` along from `from` towards `to`.
fn along(from: (f64, f64), to: (f64, f64), by: f64) -> (f64, f64) {
    let (dx, dy) = heading(from, to);
    (from.0 + dx * by, from.1 + dy * by)
}

/// The direction from `at` towards `toward`, as a unit vector. `max`
/// keeps it finite when the two points coincide.
fn heading(at: (f64, f64), toward: (f64, f64)) -> (f64, f64) {
    let (dx, dy) = (toward.0 - at.0, toward.1 - at.1);
    let length = dx.hypot(dy).max(f64::EPSILON);
    (dx / length, dy / length)
}

/// A point on a box's border, slid `by` along it.
///
/// It slides rightwards on a top or bottom border and downwards on a left
/// or right one -- one direction per border, so both ends of a line slide
/// the same way rather than splaying it -- and stops at the corner, since
/// past that there is no border left to sit on.
fn slid(rect: &Placed, at: (f64, f64), away: (f64, f64), by: f64) -> (f64, f64) {
    let along = (away.1.abs(), away.0.abs());
    (
        (at.0 + along.0 * by).clamp(rect.x, rect.x + rect.width),
        (at.1 + along.1 * by).clamp(rect.y, rect.y + rect.height),
    )
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
    written: &mut Vec<Placed>,
    at: (f64, f64),
    toward: (f64, f64),
    name: &str,
    direction: Option<&str>,
    style: &Style,
) {
    marked(
        out,
        written,
        Marked {
            at,
            toward,
            // the line runs straight to the other end, so the name has a
            // side of it to be kept off
            peer: Some(toward),
            name,
            direction,
            rounded: false,
        },
        style,
    )
}

/// A port to draw: the square straddling a border, the name beside it,
/// and the direction the feature declares where it declares one.
struct Marked<'a> {
    /// The middle of the square, on the border it sits on.
    at: (f64, f64),
    /// A point the way out of the box, which the name is set beyond.
    toward: (f64, f64),
    /// What the line leaving the port reaches, where one does. The name
    /// goes on the other side of the port from it, so that the line does
    /// not run through the name it belongs to.
    peer: Option<(f64, f64)>,
    name: &'a str,
    direction: Option<&'a str>,
    rounded: bool,
}

/// How many rows out from its port a name may be set to clear one
/// already written before it stays where it belongs.
const ROWS: usize = 4;

/// The glyph straddling a border and the name beside it: `port-l` draws a
/// square, `param-l` the same rounded, and the direction arrow goes inside
/// either.
fn marked(out: &mut String, written: &mut Vec<Placed>, mark: Marked<'_>, style: &Style) {
    let Marked {
        at,
        toward,
        peer,
        name,
        direction,
        rounded,
    } = mark;
    let side = port_side(style);
    // `proxy-v`/`proxy-h` stand a circle in for what a name reaches
    // through the border rather than declares on it
    if name.contains('.') {
        writeln!(
            out,
            "<circle class=\"port\" cx=\"{:.1}\" cy=\"{:.1}\" r=\"{:.1}\"/>",
            at.0,
            at.1,
            side / 2.0
        )
        .unwrap();
    } else {
        let corner = if rounded { side / 3.0 } else { 0.0 };
        writeln!(
            out,
            "<rect class=\"port\" x=\"{:.1}\" y=\"{:.1}\" width=\"{side:.1}\" \
             height=\"{side:.1}\" rx=\"{corner:.1}\"/>",
            at.0 - side / 2.0,
            at.1 - side / 2.0
        )
        .unwrap();
    }

    // `max` keeps the direction finite when the two boxes somehow coincide
    let (dx, dy) = (toward.0 - at.0, toward.1 - at.1);
    let length = dx.hypot(dy).max(f64::EPSILON);
    let (ux, uy) = (dx / length, dy / length);
    // The name is set just past the port and off to one side of it: the
    // side the line is not on, so that the line and what it carries --
    // a composition's diamond -- are not drawn over the name.
    let along = 0.45 * style.line_height;
    let (px, py) = (-uy, ux);
    let across = across_of(style)
        * match peer {
            Some(peer) => {
                let (dx, dy) = (peer.0 - at.0, peer.1 - at.1);
                if dx * px + dy * py >= 0.0 {
                    1.0
                } else {
                    -1.0
                }
            }
            None => 1.0,
        };
    // and it grows the way it was offset, so a long name never runs back
    // over the square it belongs to
    let anchored = |put: (f64, f64)| if put.0 >= at.0 { "start" } else { "end" };
    // Two ports on one border can sit closer together than their names
    // are long, and one name written over another leaves neither
    // readable. A name that would land on one already written is set a
    // row further out, along the way its own port faces, where it still
    // reads as that port's.
    let row = |(nth, off): (usize, f64)| {
        beside_point(at, (ux, uy), along + nth as f64 * style.line_height, off)
    };
    let put = (0..ROWS)
        // the side the line is not on comes first, and the other one is
        // still better than a name nobody can read
        .flat_map(|nth| [(nth, across), (nth, -across)])
        .map(row)
        .find(|&put| {
            let rect = text_box(put, anchored(put), name, style);
            !written.iter().any(|other| overlaps(&rect, other))
        })
        .unwrap_or(row((0, across)));
    let anchor = anchored(put);
    written.push(text_box(put, anchor, name, style));
    if let Some(direction) = direction {
        direction_arrow(out, at, (ux, uy), direction, side);
    }
    label_at(out, put, anchor, name);
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

/// How far off the line a label set beside it is put.
fn across_of(style: &Style) -> f64 {
    -0.7 * style.line_height
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
    label_at(
        out,
        beside_point(at, (ux, uy), along, across_of(style)),
        anchor,
        text,
    );
}

/// Where a label set beside a line through `at` running `(ux, uy)` goes:
/// `along` the line from there, then `across` to one side of it.
fn beside_point(at: (f64, f64), (ux, uy): (f64, f64), along: f64, across: f64) -> (f64, f64) {
    (
        at.0 + ux * along - uy * across,
        at.1 + uy * along + ux * across,
    )
}

/// One straight run of a line, as it was drawn.
type Leg = ((f64, f64), (f64, f64));

/// A name written along a line, and the run of it the name belongs to.
struct Aside {
    run: Leg,
    text: String,
}

/// Remember where a line went, leg by leg.
fn note(drawn: &mut Vec<Leg>, walked: &[(f64, f64)]) {
    drawn.extend(walked.windows(2).map(|leg| (leg[0], leg[1])));
}

/// Where along its run a name is tried, the middle first and the rest
/// outwards from it, so a name only moves where moving buys it
/// something.
const STOPS: [f64; 9] = [0.5, 0.4, 0.6, 0.3, 0.7, 0.2, 0.8, 0.1, 0.9];

/// Where a name written along a run goes: off to one side of it, and
/// along it to wherever the least of the name is covered.
fn clear_spot(aside: &Aside, drawn: &[Leg], taken: &[Placed], style: &Style) -> (f64, f64) {
    let (start, finish) = aside.run;
    let (dx, dy) = (finish.0 - start.0, finish.1 - start.1);
    // `max` keeps the direction finite when a run has no length at all
    let length = dx.hypot(dy).max(f64::EPSILON);
    let along = (dx / length, dy / length);
    let cost = |put: &(f64, f64)| covered(*put, &aside.text, drawn, taken, style);
    STOPS
        .iter()
        .flat_map(|stop| {
            let on = (start.0 + dx * stop, start.1 + dy * stop);
            // the side the name would fall on anyway comes first
            [across_of(style), -across_of(style)].map(|across| beside_point(on, along, 0.0, across))
        })
        // ties go to the first, which is the stop nearest the middle
        .min_by(|one, other| {
            let (mine, theirs) = (cost(one), cost(other));
            mine.0
                .cmp(&theirs.0)
                .then_with(|| mine.1.total_cmp(&theirs.1))
        })
        .expect("a stop to write the name at")
}

/// How much of a name set at `put` is lost, worst first: how many boxes
/// and other names are drawn over it, and then the length of line drawn
/// through the words.
///
/// The two do not trade against each other. A box is drawn after the
/// names and a name under one is not dimmed but gone, and two names on
/// one spot leave neither readable; a line through a name costs it the
/// letters it runs through and no more, and a name stands on a ground of
/// its own that keeps even those.
fn covered(
    put: (f64, f64),
    text: &str,
    drawn: &[Leg],
    taken: &[Placed],
    style: &Style,
) -> (usize, f64) {
    let rect = text_box(put, "middle", text, style);
    (
        taken.iter().filter(|other| overlaps(&rect, other)).count(),
        drawn
            .iter()
            .map(|&(one, other)| through(&rect, one, other))
            .sum(),
    )
}

/// The ground a name takes up, set at `at` and growing the way `anchor`
/// says. `node` names nothing here: only the four sides are ever read.
fn text_box(at: (f64, f64), anchor: &str, text: &str, style: &Style) -> Placed {
    let width = style.text_width(text);
    let x = match anchor {
        "start" => at.0,
        "end" => at.0 - width,
        // "middle", the anchor everything written along a line carries
        _ => at.0 - width / 2.0,
    };
    Placed {
        node: 0,
        x,
        y: at.1 - style.font_size / 2.0,
        width,
        height: style.font_size,
    }
}

/// Do two rectangles share any ground at all?
fn overlaps(one: &Placed, other: &Placed) -> bool {
    one.x < other.x + other.width
        && other.x < one.x + one.width
        && one.y < other.y + other.height
        && other.y < one.y + one.height
}

/// One line of text written over the drawing rather than inside a box,
/// anchored so that it grows the way it was put.
///
/// It is drawn on a ground of its own. [`clear_spot`] puts a name where
/// the least of it is covered, but a drawing can be dense enough that
/// every spot on a run has a line through it, and a name is what a
/// reader came to the line for. The lines are all down by the time any
/// name is written and the boxes are not, so the ground reaches back
/// over the lines and never over a box.
fn label_at(out: &mut String, (x, y): (f64, f64), anchor: &str, text: &str) {
    writeln!(
        out,
        "<text class=\"aside\" x=\"{x:.1}\" y=\"{y:.1}\" text-anchor=\"{anchor}\" \
         dominant-baseline=\"middle\">{}</text>",
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
    fn a_channel_above_the_supertype_is_no_route_down_to_it() {
        // the last leg of such a route runs from over the supertype down
        // to its bottom border, straight through the box it is meant to
        // arrive at, so it is never one of the channels offered
        let boxes = |at: usize, y: f64| Placed {
            node: at,
            x: 100.0,
            y,
            width: 200.0,
            height: 100.0,
        };
        let layout = Layout {
            placed: vec![boxes(0, 200.0), boxes(1, 0.0)],
            width: 400.0,
            height: 400.0,
            ..Layout::default()
        };
        // from the subtype's top border to the supertype's bottom one
        let ends = ((150.0, 200.0), (200.0, 100.0));
        assert_eq!(channel_for(&layout, ends, &[-10.0]), None);
        // where the gap under the supertype's row does carry it
        assert_eq!(channel_for(&layout, ends, &[150.0]), Some(150.0));
    }

    #[test]
    fn making_room_moves_everything_the_drawing_holds() {
        // routes, swimlanes and package frames are all read in canvas
        // coordinates, so sliding the boxes clear of the corner has to
        // slide them too or the drawing comes apart
        let ws = resolved("port def Fuel;\npart def Tank { port aVeryLongPortName : Fuel; }\n");
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let style = Style::default();
        let tight = Layout {
            placed: (0..diagram.nodes.len())
                .map(|node| Placed {
                    node,
                    x: 0.0,
                    y: 0.0,
                    width: 80.0,
                    height: 40.0,
                })
                .collect(),
            width: 80.0,
            height: 40.0,
            routes: vec![vec![(1.0, 2.0), (3.0, 4.0)]],
            lanes: vec![crate::layout::Column {
                name: "lane".to_string(),
                x: 5.0,
                width: 10.0,
                top: 6.0,
                height: 10.0,
            }],
            packages: vec![crate::layout::Frame {
                name: "P".to_string(),
                x: 7.0,
                y: 8.0,
                width: 10.0,
                height: 10.0,
            }],
        };
        let grown = with_room_for_labels(&diagram, &tight, &style).expect("room was needed");
        let (across, down) = (grown.placed[0].x, grown.placed[0].y);
        assert!(across > style.text_width("aVeryLongPortName"));
        assert!(down > 0.0);
        assert_eq!(
            grown.routes[0],
            [(1.0 + across, 2.0 + down), (3.0 + across, 4.0 + down)]
        );
        assert_eq!(
            (grown.lanes[0].x, grown.lanes[0].top),
            (5.0 + across, 6.0 + down)
        );
        assert_eq!(
            (grown.packages[0].x, grown.packages[0].y),
            (7.0 + across, 8.0 + down)
        );
        assert!(grown.width > tight.width + across && grown.height > tight.height + down);

        // and a drawing with nothing written outside its boxes is left
        // exactly as it was laid out
        let plain = resolved("part def A;\n");
        let plain = definition_diagram(plain.model(), &[plain.root()]);
        assert!(with_room_for_labels(&plain, &layout(&plain, &style), &style).is_none());
    }

    #[test]
    fn two_ports_a_tenth_of_a_pixel_apart_are_one_place() {
        // 100.4 and 100.6 fall either side of a half-pixel boundary, so
        // bucketing by one used to leave these two squares drawn over
        // each other while spreading a pair that straddled no boundary
        let together = piles(&[(100.4, 5.0), (100.6, 5.0), (300.0, 5.0)]);
        assert_eq!(together, [vec![0, 1], vec![2]]);
        // and a point half a pixel off is a place of its own
        assert_eq!(piles(&[(0.0, 0.0), (0.0, 0.5)]), [vec![0], vec![1]]);
    }

    #[test]
    fn the_canvas_leaves_room_for_the_names_written_outside_the_boxes() {
        let source = "port def Fuel;\n\
             part def Tank { port aVeryLongPortNameIndeed : Fuel; }\n";
        let ws = resolved(source);
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let style = Style::default();
        let placed = layout(&diagram, &style);
        let tank = diagram
            .nodes
            .iter()
            .position(|it| it.name == "Tank")
            .unwrap();

        // the box is held clear of the canvas edge by the width of the
        // name its port writes outside it
        let room = style.text_width("aVeryLongPortNameIndeed");
        assert!(placed.placed[tank].x > room, "{:?}", placed.placed[tank]);
        assert!(placed.width > placed.placed[tank].x + placed.placed[tank].width + room);

        // and the name itself is on the canvas, whichever side it fell on
        let svg = render(&diagram, &style);
        let at = svg
            .split("aVeryLongPortNameIndeed</text>")
            .next()
            .and_then(|before| before.rsplit_once("<text class=\"aside\" x=\""))
            .map(|(_, tag)| tag)
            .expect("the port's name is drawn");
        let x: f64 = at.split('"').next().unwrap().parse().unwrap();
        assert!(x - room > 0.0 && x + room < placed.width, "{svg}");
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
        // the whole is drawn above its part, so the line leaves the
        // bottom border and arrives at the top of the other
        assert!((y1 - (whole.y + whole.height)).abs() < 0.1, "{y1}");
        assert!(x1 >= whole.x && x1 <= whole.x + whole.width);
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
    fn a_line_leaves_each_box_on_the_side_that_faces_the_other() {
        let style = Style::default();
        let ws = resolved(
            "part def LugBolt;\n\
             part def Wheel { part lb : LugBolt; }\n\
             part def Chassis {\n\
             \tpart w : Wheel;\n\
             \tpart lb : LugBolt;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let placed = layout(&diagram, &style);
        let spanning = diagram
            .edges
            .iter()
            .position(|edge| {
                let (from, to) = (&placed.placed[edge.from], &placed.placed[edge.to]);
                to.y > from.y + from.height + style.v_gap
            })
            .expect("the bolt is two rows below the chassis");
        let edge = &diagram.edges[spanning];
        let Facing {
            detour: (leaves, arrives),
            bands: (first, second),
        } = facing_sides(&placed, edge, &style, 0.0, (0.5, 0.5));
        let (whole, part) = (&placed.placed[edge.from], &placed.placed[edge.to]);

        // down out of the whole and in through the top of the part
        assert_eq!(leaves.1, whole.y + whole.height);
        assert_eq!(arrives.1, part.y);
        // and the gaps it runs along lie between them
        assert!(first > whole.y + whole.height && first < part.y);
        assert!(second > whole.y + whole.height && second < part.y);
    }

    #[test]
    fn a_line_between_peers_leaves_both_downward() {
        // neither box is above the other, so there is no facing side and
        // the line drops into the gap under the row they share
        let style = Style::default();
        let ws = resolved(
            "part def Sensor;\n\
             part def Display;\n\
             dependency from Sensor to Display;\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let placed = layout(&diagram, &style);
        let joined = diagram
            .edges
            .iter()
            .position(|edge| edge.relation == Relation::Dependency)
            .expect("one definition depends on the other");
        let edge = &diagram.edges[joined];
        let Facing {
            detour: (leaves, arrives),
            bands: (first, second),
        } = facing_sides(&placed, edge, &style, 0.0, (0.5, 0.5));
        let (one, other) = (&placed.placed[edge.from], &placed.placed[edge.to]);

        assert_eq!(leaves.1, one.y + one.height);
        assert_eq!(arrives.1, other.y + other.height);
        assert!(first > one.y + one.height && second > other.y + other.height);
    }

    #[test]
    fn a_line_that_would_run_under_a_box_steps_around_it() {
        // `Chassis` is made of a wheel and of a bolt the wheel is made
        // of too, so the line to the bolt has a row to get past
        let svg = svg_of(
            "part def LugBolt;\n\
             part def Wheel { part lb : LugBolt; }\n\
             part def Chassis {\n\
             	part w : Wheel;\n\
             	part lb : LugBolt;\n\
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
            lanes: Vec::new(),
            packages: Vec::new(),
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
            lanes: Vec::new(),
            packages: Vec::new(),
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
            lanes: Vec::new(),
            packages: Vec::new(),
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
        assert!(svg.contains(">p</text>"), "{svg}");
        assert!(svg.contains(">q</text>"), "{svg}");
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
            lanes: Vec::new(),
            packages: Vec::new(),
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
             part def Chassis {\n\
             	part w : Wheel;\n\
             	part lb : LugBolt;\n\
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
    fn a_port_sets_its_name_on_the_side_its_line_is_not_on() {
        // the line leaves the left border going down, so the name goes up
        let style = Style::default();
        let above = |out: &str| {
            let y: f64 = out
                .split("<text")
                .nth(1)
                .and_then(|text| text.split("y=\"").nth(1))
                .and_then(|value| value.split('"').next())
                .and_then(|value| value.parse().ok())
                .expect("the name states where it sits");
            y < 20.0
        };
        let beside = |peer: (f64, f64)| {
            let mut out = String::new();
            marked(
                &mut out,
                &mut Vec::new(),
                Marked {
                    at: (50.0, 20.0),
                    // the border faces left, whatever the line then does
                    toward: (42.0, 20.0),
                    peer: Some(peer),
                    name: "p",
                    direction: None,
                    rounded: false,
                },
                &style,
            );
            out
        };
        let down = beside((10.0, 90.0));
        assert!(above(&down), "{down}");

        // and the other way round when the line leaves going up
        let up = beside((10.0, -50.0));
        assert!(!above(&up), "{up}");
    }

    #[test]
    fn a_port_on_a_horizontal_border_sets_its_name_beside_it() {
        // a line leaving straight down has no room under the port for a
        // name, so the name goes to one side of it and grows that way --
        // centred, it would be drawn over its own square
        let style = Style::default();
        let mut out = String::new();
        port(
            &mut out,
            &mut Vec::new(),
            (50.0, 20.0),
            (50.0, 90.0),
            "p",
            None,
            &style,
        );

        assert!(out.contains("text-anchor=\"start\""), "{out}");
        let x: f64 = out
            .split("<text")
            .nth(1)
            .and_then(|text| text.split("x=\"").nth(1))
            .and_then(|value| value.split('"').next())
            .and_then(|value| value.parse().ok())
            .expect("the name states where it starts");
        assert!(
            x > 50.0 + port_side(&style) / 2.0,
            "the name covers its port"
        );
    }

    /// The tips a port's direction arrow draws, and which way each points:
    /// one head for `in` or `out`, one at each end for `inout`.
    fn tips(direction: &str) -> Vec<f64> {
        let mut out = String::new();
        // a port on the left border of a box that lies to the right, so
        // the way it faces is -x and `in` runs back into the box
        port(
            &mut out,
            &mut Vec::new(),
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
        // the name beside the glyph, the type in the compartment
        assert!(svg.contains(">cold</text>"), "{svg}");
        assert!(svg.contains(">hot</text>"), "{svg}");
        assert!(svg.contains(">in item cold : Temp</text>"), "{svg}");
    }

    #[test]
    fn a_name_that_reaches_through_the_border_is_a_proxy() {
        // `proxy-v`/`proxy-h` stand a circle in for what a name reaches
        // through the border, where a port it declares is a square
        let mut out = String::new();
        port(
            &mut out,
            &mut Vec::new(),
            (50.0, 20.0),
            (42.0, 20.0),
            "hub.pin",
            None,
            &Style::default(),
        );
        assert!(out.contains("<circle class=\"port\""), "{out}");
        let mut declared = String::new();
        port(
            &mut declared,
            &mut Vec::new(),
            (50.0, 20.0),
            (42.0, 20.0),
            "pin",
            None,
            &Style::default(),
        );
        assert!(declared.contains("<rect class=\"port\""), "{declared}");
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
    fn a_slide_along_a_border_stops_at_the_corner() {
        let placed = Placed {
            node: 0,
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 40.0,
        };
        // a top or bottom border slides rightwards
        assert_eq!(slid(&placed, (50.0, 60.0), (0.0, 1.0), 20.0), (70.0, 60.0));
        // and no further than the corner, past which there is no border
        // left to sit on
        assert_eq!(
            slid(&placed, (100.0, 60.0), (0.0, 1.0), 40.0),
            (110.0, 60.0)
        );
        // a left or right border slides downwards instead
        assert_eq!(slid(&placed, (10.0, 30.0), (-1.0, 0.0), 5.0), (10.0, 35.0));
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
    fn a_package_is_the_folder_the_standard_draws_round_what_it_holds() {
        let ws = resolved(
            "package Outer {\n\
             \tpart def A;\n\
             \tpackage Inner { part def C; }\n\
             }\n\
             package Other { part def E; }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let layout = crate::layout(&diagram, &Style::default());
        let svg = to_svg(&diagram, &layout, &Style::default());
        assert!(svg.contains(">Outer</text>"), "{svg}");

        // `Inner` is drawn inside `Outer`, and `Other` beside neither
        let frame = |name: &str| {
            layout
                .packages
                .iter()
                .find(|frame| frame.name == name)
                .unwrap()
        };
        let (outer, inner, other) = (frame("Outer"), frame("Inner"), frame("Other"));
        assert!(outer.x <= inner.x && inner.x + inner.width <= outer.x + outer.width);
        assert!(outer.y < inner.y && inner.y + inner.height <= outer.y + outer.height);
        assert!(other.y >= outer.y + outer.height, "two frames overlap");

        // and every definition sits inside the package that owns it
        for (group, frame) in diagram.groups.iter().zip(&layout.packages) {
            for &at in &group.nodes {
                let placed = &layout.placed[at];
                assert!(placed.x >= frame.x, "{} escaped its package", placed.node);
                assert!(placed.x + placed.width <= frame.x + frame.width);
                assert!(placed.y >= frame.y && placed.y + placed.height <= frame.y + frame.height);
            }
        }
    }

    #[test]
    fn a_package_tab_is_clear_of_what_the_package_holds() {
        // the tab is drawn inside the frame, so a box at the very top of
        // it would have its own border drawn along the tab's bottom line
        // and the two would read as one
        let style = Style::default();
        let ws = resolved(
            "package Outer {\n\
             \tpart def A;\n\
             \tpackage Inner { part def C; }\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let placed = layout(&diagram, &style);
        let tab = style.package_tab();
        for (group, frame) in diagram.groups.iter().zip(&placed.packages) {
            for &at in &group.nodes {
                let clear = frame.y + tab + style.padding;
                let named = &diagram.nodes[at].name;
                assert!(placed.placed[at].y >= clear, "{named} is under the tab");
            }
        }
    }

    #[test]
    fn a_package_that_owns_only_packages_is_drawn_round_them() {
        let ws = resolved(
            "package Top {\n\
             \tpackage One { part def A; }\n\
             \tpackage Two { part def B; }\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let layout = crate::layout(&diagram, &Style::default());
        let frame = |name: &str| {
            layout
                .packages
                .iter()
                .find(|frame| frame.name == name)
                .unwrap()
        };
        let (top, one, two) = (frame("Top"), frame("One"), frame("Two"));
        for within in [one, two] {
            assert!(top.x <= within.x && within.x + within.width <= top.x + top.width);
            assert!(top.y < within.y && within.y + within.height <= top.y + top.height);
        }
        assert!(two.y >= one.y + one.height, "two frames overlap");
        assert!(to_svg(&diagram, &layout, &Style::default()).contains(">Top</text>"));
    }

    #[test]
    fn a_swimlane_is_headed_by_its_performer_and_holds_what_it_carries_out() {
        let ws = resolved(
            "action def Generate;\n\
             action def Convert;\n\
             action providePower {\n\
             \taction generateTorque : Generate;\n\
             \taction convert : Convert;\n\
             }\n\
             part def Engine { perform providePower.generateTorque; }\n\
             part def Gearbox { perform providePower.convert; }\n",
        );
        let power = ws
            .named_elements()
            .find(|(_, name)| *name == "providePower")
            .map(|(id, _)| id)
            .unwrap();
        let diagram = interconnection_diagram(ws.model(), power);
        let layout = crate::layout(&diagram, &Style::default());
        let svg = to_svg(&diagram, &layout, &Style::default());
        assert_eq!(svg.matches("class=\"lane\"").count(), 2, "{svg}");
        assert!(svg.contains(">Engine</text>"), "{svg}");

        // the lanes touch on their vertical edges and are aligned along
        // the top and the bottom, the way the standard's note has it
        assert_eq!(layout.lanes.len(), 2);
        let (first, second) = (&layout.lanes[0], &layout.lanes[1]);
        assert_eq!(first.x + first.width, second.x);
        assert_eq!((first.top, first.height), (second.top, second.height));
        // and each box sits inside the lane that carries it out
        for (lane, column) in diagram.lanes.iter().zip(&layout.lanes) {
            for &at in &lane.nodes {
                let placed = &layout.placed[at];
                assert!(placed.x >= column.x, "a box outside its lane");
                assert!(placed.x + placed.width <= column.x + column.width);
            }
        }
    }

    #[test]
    fn an_n_ary_dependency_meets_at_a_dot() {
        // the standard's note: two or more of either end makes it n-ary,
        // and then a client link reaches the dot with no arrowhead --
        // the dot is not what the client depends on
        let ws = resolved(
            "package P {\n\
             \tpart def A;\n\
             \tpart def B;\n\
             \tpart def C;\n\
             \tdependency Wide from A, B to C;\n\
             }\n",
        );
        let svg = render(
            &definition_diagram(ws.model(), &[ws.root()]),
            &Style::default(),
        );
        assert_eq!(svg.matches("class=\"dependency\"").count(), 3, "{svg}");
        // one of the three carries the arrowhead: the supplier link
        assert_eq!(svg.matches("url(#transition)").count(), 1, "{svg}");
        assert_eq!(svg.matches("class=\"initial\"").count(), 1, "{svg}");
        assert!(svg.contains(">Wide</text>"), "{svg}");
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
        // each is named beside itself. The name alone: the compartment
        // an inch away says what it is typed by, and a second copy of
        // that only gives a line more to be drawn through
        assert!(svg.contains(">fuelIn</text>"), "{svg}");
        assert!(svg.contains(">airIn</text>"), "{svg}");
        assert!(!svg.contains(">fuelIn : Fuel</text>"), "{svg}");
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
    fn a_port_sits_where_the_route_leaves_the_box_not_where_a_straight_line_would() {
        // An engine that routes for itself bends, and leaves through
        // whichever border it chose. Placing the square by the straight
        // line to the other box instead leaves it on a border the line
        // never touches, with the diamond stranded somewhere else.
        let ws = resolved("port def Fuel;\npart def Tank { port out1 : Fuel; }\n");
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let style = Style::default();
        let mut layout = layout(&diagram, &style);
        let at = diagram
            .edges
            .iter()
            .position(|edge| edge.ends.0.as_deref() == Some("out1"))
            .unwrap();
        let tank = &layout.placed[diagram.edges[at].from];
        let leaves = (tank.x + tank.width / 3.0, tank.y + tank.height);
        layout.routes = vec![Vec::new(); diagram.edges.len()];
        layout.routes[at] = vec![leaves, (leaves.0, leaves.1 + 40.0), (0.0, leaves.1 + 40.0)];
        let svg = to_svg(&diagram, &layout, &style);
        let clear = port_side(&style) / 2.0;
        assert!(
            svg.contains(&format!(
                "<rect class=\"port\" x=\"{:.1}\" y=\"{:.1}\"",
                leaves.0 - clear,
                leaves.1 - clear
            )),
            "{svg}"
        );
        // and the line starts on the square's outer face, so what it
        // carries there is against the square rather than under it
        assert!(
            svg.contains(&format!("d=\"M {:.1} {:.1} ", leaves.0, leaves.1 + clear)),
            "{svg}"
        );
    }

    #[test]
    fn ports_of_one_type_get_a_square_each_rather_than_one_between_them() {
        // the lines are held apart by a lane each; the squares they end
        // on have to be held apart by the same amount, or three lines
        // arrive at one square and two of them look unattached
        let svg = svg_of(
            "port def Sig;\n\
             part def Box {\n\
             \tport p : Sig;\n\
             \tport q : Sig;\n\
             \tport r : Sig;\n\
             }\n",
        );
        let squares: Vec<&str> = svg
            .match_indices("<rect class=\"port\"")
            .map(|(at, _)| &svg[at..at + 46])
            .collect();
        assert_eq!(squares.len(), 3, "{svg}");
        assert_eq!(
            squares
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            3,
            "{svg}"
        );
    }

    #[test]
    fn ports_facing_two_boxes_the_same_way_are_spread_along_the_border() {
        // two features can be typed by two definitions that lie the same
        // way, and the border point each faces is then the same point
        let ws = resolved(
            "port def One;\n\
             port def Two;\n\
             part def Box {\n\
             \tport p : One;\n\
             \tport q : Two;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let style = Style::default();
        let mut layout = layout(&diagram, &style);
        let box_at = diagram.edges[0].from;
        layout.placed[box_at].x = 100.0;
        layout.placed[box_at].y = 0.0;
        layout.placed[box_at].width = 200.0;
        layout.placed[box_at].height = 100.0;
        // both types straight below, so both face the middle of the
        // bottom border
        for (nth, edge) in diagram.edges.iter().enumerate() {
            let peer = &mut layout.placed[edge.to];
            peer.x = 100.0;
            peer.y = 400.0 + 400.0 * nth as f64;
            peer.width = 200.0;
            peer.height = 100.0;
        }
        let svg = to_svg(&diagram, &layout, &style);
        let squares: Vec<&str> = svg
            .match_indices("<rect class=\"port\"")
            .map(|(at, _)| &svg[at..at + 46])
            .collect();
        assert_eq!(squares.len(), 2, "{svg}");
        assert_ne!(squares[0], squares[1], "{svg}");
    }

    #[test]
    fn a_feature_typed_by_what_declares_it_sits_on_the_loop_it_draws() {
        // there is no other box to face, so the border point works out
        // as the box's own middle -- inside it, on no border at all.
        // The loop leaves the bottom border, and the square goes there.
        let svg = svg_of("port def Chain { port next : Chain; }\n");
        let drawn = svg.split("d=\"M ").nth(1).expect("a loop is drawn");
        let start: Vec<f64> = drawn
            .split(' ')
            .take(2)
            .map(|word| word.parse().expect("two numbers after the M"))
            .collect();
        // the square straddles the border the loop sets out from, and
        // the loop starts on its outer face
        let clear = port_side(&Style::default()) / 2.0;
        assert!(
            svg.contains(&format!(
                "<rect class=\"port\" x=\"{:.1}\" y=\"{:.1}\"",
                start[0] - clear,
                start[1] - 2.0 * clear
            )),
            "loop starts at {start:?} in {svg}"
        );
    }

    #[test]
    fn a_portion_of_what_declares_it_loops_rather_than_collapsing() {
        // `timeslice t : Person` inside `item def Person` relates the
        // box to itself; run centre to centre it would be a line of no
        // length at all, with its marker adrift in the middle of the box
        let svg = svg_of("item def Person { timeslice was : Person; }\n");
        assert!(svg.contains("marker-start=\"url(#portion)\""), "{svg}");
        assert!(
            !svg.contains("<line class=\"edge\""),
            "a loop is a path, not a line of no length: {svg}"
        );
    }

    #[test]
    fn a_parameter_that_detours_is_drawn_where_the_detour_leaves() {
        // A line that would run under a box in the way goes round it
        // instead, leaving downward rather than through the border a
        // straight line would cross. The glyph an action's parameter is
        // drawn as has to go with it, or it is left on a border the
        // line never touches.
        let ws = resolved(
            "attribute def Millis;\n\
             action def Wait { in millis : Millis; }\n\
             part def Wall;\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let style = Style::default();
        let mut layout = layout(&diagram, &style);
        let at = diagram
            .edges
            .iter()
            .position(|edge| edge.ends.0.as_deref() == Some("millis"))
            .unwrap();
        let (wait, millis) = (diagram.edges[at].from, diagram.edges[at].to);
        let wall = (0..layout.placed.len())
            .find(|&at| at != wait && at != millis)
            .unwrap();
        // stack them: the wall square between the two, so a straight
        // line from one to the other runs under it
        for (at, y) in [(wait, 0.0), (wall, 200.0), (millis, 400.0)] {
            layout.placed[at].x = 100.0;
            layout.placed[at].y = y;
            layout.placed[at].width = 200.0;
            layout.placed[at].height = 100.0;
        }
        let svg = to_svg(&diagram, &layout, &style);
        // the detour leaves the middle of the border it sets out from
        let leaves = (200.0, 100.0);
        let clear = port_side(&style) / 2.0;
        assert!(
            svg.contains(&format!(
                "<rect class=\"port\" x=\"{:.1}\" y=\"{:.1}\"",
                leaves.0 - clear,
                leaves.1 - clear
            )),
            "{svg}"
        );
        assert!(
            svg.contains(&format!("d=\"M {:.1} {:.1} V", leaves.0, leaves.1 + clear)),
            "{svg}"
        );
    }

    #[test]
    fn a_name_slides_along_its_run_to_where_no_line_covers_it() {
        // which side of its run a name falls on says nothing about what
        // is drawn there: another line can run the length of the page
        // through the very spot, and the name is the thing a reader
        // came to the line for
        let style = Style::default();
        let aside = Aside {
            run: ((0.0, 100.0), (600.0, 100.0)),
            text: "connected".to_string(),
        };
        let across = across_of(&style);
        let struck = ((0.0, 100.0 + across), (600.0, 100.0 + across));

        let put = clear_spot(&aside, &[struck], &[], &style);
        assert_eq!(covered(put, &aside.text, &[struck], &[], &style), (0, 0.0));
        // it is still beside its own run, on the other side of it
        assert!((put.1 - (100.0 - across)).abs() < 0.05, "{put:?}");

        // and with nothing in the way it stays opposite the middle
        assert_eq!(
            clear_spot(&aside, &[], &[], &style),
            (300.0, 100.0 + across)
        );
    }

    #[test]
    fn a_name_gives_way_to_a_box_that_would_be_drawn_over_it() {
        // the boxes are drawn after the names, so a name under one is
        // not dimmed but gone, which is why a box costs a name its
        // whole width where a line costs it the letters it runs through
        let style = Style::default();
        let aside = Aside {
            run: ((0.0, 100.0), (600.0, 100.0)),
            text: "hidden".to_string(),
        };
        let over = Placed {
            node: 0,
            x: 200.0,
            y: 60.0,
            width: 200.0,
            height: 60.0,
        };
        let put = clear_spot(&aside, &[], &[over], &style);
        assert_eq!(covered(put, &aside.text, &[], &[over], &style), (0, 0.0));
        assert!(!overlaps(
            &text_box(put, "middle", &aside.text, &style),
            &over
        ));
    }

    #[test]
    fn a_name_would_rather_be_run_through_than_hidden_under_a_box() {
        // the two do not trade against each other: a box is drawn after
        // the names and a name under one is gone, where a line through
        // one costs it the letters it runs through and the ground it
        // stands on keeps even those
        let style = Style::default();
        let aside = Aside {
            run: ((0.0, 100.0), (600.0, 100.0)),
            text: "counted".to_string(),
        };
        let across = across_of(&style);
        // the side the name falls on anyway is clear of every line and
        // under a box for the whole length of the run; the other side
        // has a line down all of it
        let over = Placed {
            node: 0,
            x: 0.0,
            y: 100.0 + across - 10.0,
            width: 600.0,
            height: 20.0,
        };
        let struck = ((0.0, 100.0 - across), (600.0, 100.0 - across));

        let put = clear_spot(&aside, &[struck], &[over], &style);
        assert!((put.1 - (100.0 - across)).abs() < 0.05, "{put:?}");
        assert_eq!(covered(put, &aside.text, &[struck], &[over], &style).0, 0);
    }

    #[test]
    fn a_port_name_that_would_land_on_one_already_written_is_set_out_a_row() {
        // two ports on one border can sit closer together than their
        // names are long, and one name over another leaves neither
        // readable
        let style = Style::default();
        let mut written = Vec::new();
        let mut out = String::new();
        // both leave the same border downward, closer together than
        // `shaftPort_x` is wide
        for at in [(100.0, 50.0), (140.0, 50.0)] {
            port(
                &mut out,
                &mut written,
                at,
                (at.0, 90.0),
                "shaftPort_x",
                None,
                &style,
            );
        }

        let rows: Vec<f64> = out
            .lines()
            .filter(|line| line.starts_with("<text"))
            .map(|line| anchor_tests::number(line, " y=\""))
            .collect();
        assert_eq!(rows.len(), 2, "{out}");
        assert!((rows[1] - rows[0]).abs() >= style.line_height, "{out}");
        assert!(!overlaps(&written[0], &written[1]), "{written:?}");
    }

    #[test]
    fn a_name_written_over_the_drawing_stands_on_a_ground_of_its_own() {
        // a drawing gets dense enough that every spot along a run has a
        // line through it, and where the name cannot be moved clear it
        // is read against whatever the lines there are doing
        let svg = svg_of(
            "part def A;\n\
             connection def D {\n\
             \tend one : A;\n\
             }\n",
        );
        assert!(svg.contains("<text class=\"aside\""), "{svg}");
        assert!(
            svg.contains(
                ".aside { fill: var(--muted); paint-order: stroke; \
                          stroke: var(--box); stroke-width: 3; stroke-linejoin: round; }"
            ),
            "{svg}"
        );
    }

    #[test]
    fn a_name_on_a_bent_line_is_written_clear_of_the_run_it_carries() {
        // the name was set off to one side of the line's far end, which
        // for a bent line is not the run the name is written along:
        // where the line doubled back on itself, the offset put the
        // name back over the channel and it was drawn through the words
        let ws = resolved(
            "part def A;\n\
             part def Wall;\n\
             connection def D {\n\
             \tend one : A;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let style = Style::default();
        let mut layout = layout(&diagram, &style);
        // one row, with the wall standing between the two boxes the
        // line joins: a straight line would run under it, so the line
        // drops into the channel beneath the row and runs back along it
        let row: Vec<usize> = ["A", "Wall", "D"]
            .iter()
            .map(|name| {
                (0..layout.placed.len())
                    .find(|&at| diagram.nodes[layout.placed[at].node].name == *name)
                    .expect("the box is drawn")
            })
            .collect();
        for (nth, &at) in row.iter().enumerate() {
            layout.placed[at].x = 300.0 * nth as f64;
            layout.placed[at].y = 0.0;
            layout.placed[at].width = 200.0;
            layout.placed[at].height = 100.0;
        }
        let svg = to_svg(&diagram, &layout, &style);

        let route = svg
            .lines()
            .find(|line| line.starts_with("<path class=\"edge\""))
            .expect("the line went round the wall");
        let channel: f64 = route
            .split(" V ")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .and_then(|it| it.parse().ok())
            .expect("the channel it runs along");
        let name = svg
            .lines()
            .find(|line| line.contains(">one<"))
            .expect("the end is named beside the line");
        let put = anchor_tests::number(name, " y=\"");
        // half the font is what a line has to clear to miss the letters
        assert!(
            (put - channel).abs() >= 0.5 * style.font_size,
            "`one` is written at {put}, on the run at {channel}\n{svg}"
        );
    }

    #[test]
    fn two_detours_leaving_one_box_set_out_from_different_points() {
        // both would leave the middle of the border it faces them
        // through, piling their markers -- and the square of any port
        // they name -- on the one point
        let ws = resolved(
            "attribute def One;\n\
             attribute def Two;\n\
             action def Act {\n\
             \tin a : One;\n\
             \tin b : Two;\n\
             }\n\
             part def Wall;\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let style = Style::default();
        let mut layout = layout(&diagram, &style);
        let act = diagram.edges[0].from;
        let wall = (0..layout.placed.len())
            .find(|at| *at != act && diagram.edges.iter().all(|edge| edge.to != *at))
            .expect("a box nothing is typed by");
        // stack them, so a straight line to either type runs under the
        // wall and has to go round it
        layout.placed[act].x = 100.0;
        layout.placed[act].y = 0.0;
        layout.placed[wall].x = 100.0;
        layout.placed[wall].y = 200.0;
        for (nth, edge) in diagram.edges.iter().enumerate() {
            layout.placed[edge.to].x = 100.0;
            layout.placed[edge.to].y = 400.0 + 200.0 * nth as f64;
        }
        for at in [act, wall] {
            layout.placed[at].width = 300.0;
            layout.placed[at].height = 100.0;
        }
        for edge in &diagram.edges {
            layout.placed[edge.to].width = 300.0;
            layout.placed[edge.to].height = 100.0;
        }
        let svg = to_svg(&diagram, &layout, &style);
        let mark = "<path class=\"edge\" fill=\"none\" d=\"M ";
        let starts: Vec<&str> = svg
            .match_indices(mark)
            .map(|(at, _)| &svg[at + mark.len()..at + mark.len() + 11])
            .collect();
        assert_eq!(starts.len(), 2, "{svg}");
        assert_ne!(starts[0], starts[1], "{svg}");
    }

    #[test]
    fn a_route_arriving_at_a_port_stops_at_its_square_too() {
        // the far end of a route names a port as readily as the near
        // one, and is held off the square by the same half a square
        let ws = resolved(
            "port def Fuel;\n\
             part def Tank { port out1 : Fuel; }\n\
             part def Engine { port in1 : Fuel; }\n\
             part def Car {\n\
             \tpart t : Tank;\n\
             \tpart e : Engine;\n\
             \tconnect t.out1 to e.in1;\n\
             }\n",
        );
        let car = ws
            .named_elements()
            .find(|(_, name)| *name == "Car")
            .map(|(id, _)| id)
            .unwrap();
        let diagram = interconnection_diagram(ws.model(), car);
        let style = Style::default();
        let mut layout = layout(&diagram, &style);
        let at = diagram
            .edges
            .iter()
            .position(|edge| edge.ends.1.as_deref() == Some("in1"))
            .unwrap();
        let edge = &diagram.edges[at];
        let (from, to) = (&layout.placed[edge.from], &layout.placed[edge.to]);
        let leaves = (from.x + from.width / 3.0, from.y + from.height);
        let arrives = (to.x + to.width / 3.0, to.y + to.height);
        let below = leaves.1.max(arrives.1) + 40.0;
        layout.routes = vec![Vec::new(); diagram.edges.len()];
        layout.routes[at] = vec![leaves, (leaves.0, below), (arrives.0, below), arrives];
        let svg = to_svg(&diagram, &layout, &style);
        let clear = port_side(&style) / 2.0;
        assert!(
            svg.contains(&format!(
                "<rect class=\"port\" x=\"{:.1}\" y=\"{:.1}\"",
                arrives.0 - clear,
                arrives.1 - clear
            )),
            "{svg}"
        );
        assert!(
            svg.contains(&format!("L {:.1} {:.1}\"", arrives.0, arrives.1 + clear)),
            "{svg}"
        );
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

#[cfg(test)]
pub(crate) mod anchor_tests {
    use super::*;
    use crate::tests::resolved;
    use crate::{definition_diagram, interconnection_diagram, layout};

    /// One port named by two connections: the second box lies straight
    /// ahead of the first, so one line runs straight there and the other
    /// has to go round it.
    pub(crate) const TWICE: &str = "port def P;\n\
                         part def Src { port o : P; }\n\
                         part def Dst { port i : P; }\n\
                         part def Sys {\n\
                         \tpart s : Src;\n\
                         \tpart d1 : Dst;\n\
                         \tpart d2 : Dst;\n\
                         \tconnect s.o to d1.i;\n\
                         \tconnect s.o to d2.i;\n\
                         }\n";

    pub(crate) fn diagram_of(source: &str, name: &str) -> Diagram {
        let ws = resolved(source);
        let owner = ws
            .named_elements()
            .find(|(_, declared)| *declared == name)
            .map(|(id, _)| id)
            .expect("the definition is declared");
        interconnection_diagram(ws.model(), owner)
    }

    /// The value of one `name="123.4"` attribute.
    pub(crate) fn number(text: &str, key: &str) -> f64 {
        text.split(key)
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| panic!("`{key}` in `{text}`"))
    }

    /// The middle of every square a port is drawn as.
    pub(crate) fn squares(svg: &str) -> Vec<(f64, f64)> {
        svg.lines()
            .filter(|line| line.starts_with("<rect class=\"port\""))
            .map(|line| {
                let side = number(line, " width=\"");
                (
                    number(line, " x=\"") + side / 2.0,
                    number(line, " y=\"") + side / 2.0,
                )
            })
            .collect()
    }

    /// Where every edge meets a box, both ends of it: the two ends of a
    /// straight line, and the first and last point of a routed one.
    pub(crate) fn edge_ends(svg: &str) -> Vec<((f64, f64), (f64, f64))> {
        svg.lines()
            .filter_map(|line| {
                if line.starts_with("<line class=\"edge\"") {
                    return Some((
                        (number(line, " x1=\""), number(line, " y1=\"")),
                        (number(line, " x2=\""), number(line, " y2=\"")),
                    ));
                }
                let path = line
                    .starts_with("<path class=\"edge\"")
                    .then(|| line.split(" d=\"").nth(1))
                    .flatten()?;
                let mut walked = path.split('"').next()?.split_whitespace();
                let mut at = (0.0, 0.0);
                let mut first = None;
                while let Some(step) = walked.next() {
                    let mut number = || walked.next().and_then(|it| it.parse::<f64>().ok());
                    match step {
                        "H" => at.0 = number()?,
                        "V" => at.1 = number()?,
                        // `M` and `L`, the only other commands the
                        // renderer writes
                        _ => at = (number()?, number()?),
                    }
                    first = first.or(Some(at));
                }
                Some((first?, at))
            })
            .collect()
    }

    /// How many of `squares` the line touching `at` could be said to
    /// leave from: the square is drawn round the point a line sets out
    /// from, half a side away.
    pub(crate) fn leaves(squares: &[(f64, f64)], at: (f64, f64), style: &Style) -> usize {
        let clear = port_side(style) / 2.0;
        squares
            .iter()
            .filter(|square| (at.0 - square.0).hypot(at.1 - square.1) - clear < 0.05)
            .count()
    }

    #[test]
    fn ports_nothing_reaches_are_spread_evenly_down_their_side() {
        // they used to be placed at (k + 1) / (k + 2) of the height,
        // which converges on the bottom corner: the third one on a side
        // and everything after it piled up there
        // untyped, so nothing is drawn for the ports to face
        let mut source = String::from("part def Eight {\n");
        for nth in 0..8 {
            source.push_str(&format!("\tport p{nth};\n"));
        }
        source.push_str("}\n");
        let ws = resolved(&source);
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let style = Style::default();
        let placed = layout(&diagram, &style);
        let svg = to_svg(&diagram, &placed, &style);

        let squares = squares(&svg);
        assert_eq!(squares.len(), 8, "{svg}");
        for side in [
            placed.placed[0].x,
            placed.placed[0].x + placed.placed[0].width,
        ] {
            let mut down: Vec<f64> = squares
                .iter()
                .filter(|square| (square.0 - side).abs() < 0.05)
                .map(|square| square.1)
                .collect();
            down.sort_by(f64::total_cmp);
            assert_eq!(down.len(), 4, "{squares:?}");
            let step = down[1] - down[0];
            for gap in down.windows(2) {
                assert!((gap[1] - gap[0] - step).abs() < 0.05, "{down:?}");
            }
        }
    }

    #[test]
    fn both_lines_naming_one_port_set_out_from_its_square() {
        let diagram = diagram_of(TWICE, "Sys");
        let style = Style::default();
        let svg = to_svg(&diagram, &layout(&diagram, &style), &style);

        // one square for `o` and one for each `i`, not two for `o`
        let squares = squares(&svg);
        assert_eq!(squares.len(), 3, "{svg}");
        // one line runs straight and the other is routed round the box
        // in the way, and both leave `o` where its square is drawn
        let drawn = edge_ends(&svg);
        assert_eq!(drawn.len(), 2, "{svg}");
        for (start, finish) in drawn {
            assert_eq!(leaves(&squares, start, &style), 1, "{svg}");
            assert_eq!(leaves(&squares, finish, &style), 1, "{svg}");
        }
    }

    #[test]
    fn both_lines_arriving_at_one_port_meet_the_same_square() {
        let diagram = diagram_of(
            "port def P;\n\
             part def Src { port o : P; }\n\
             part def Dst { port i : P; }\n\
             part def Sys {\n\
             \tpart d : Dst;\n\
             \tpart s1 : Src;\n\
             \tpart s2 : Src;\n\
             \tconnect s1.o to d.i;\n\
             \tconnect s2.o to d.i;\n\
             }\n",
            "Sys",
        );
        let style = Style::default();
        let svg = to_svg(&diagram, &layout(&diagram, &style), &style);

        let squares = squares(&svg);
        assert_eq!(squares.len(), 3, "{svg}");
        let drawn = edge_ends(&svg);
        assert_eq!(drawn.len(), 2, "{svg}");
        for (start, finish) in drawn {
            assert_eq!(leaves(&squares, start, &style), 1, "{svg}");
            assert_eq!(leaves(&squares, finish, &style), 1, "{svg}");
        }
    }
}

//! What a drawing is made of: the size of every box, the shapes a
//! finished layout has, and the columns of a swimlane view.
//!
//! Where the boxes go is ELK's answer (`crate::elk`). A view partitioned
//! by performer is the exception: the layered algorithm's partitions run
//! along the flow and a swimlane runs across it, so those columns are
//! placed here.

use std::collections::HashSet;

use crate::graph::{Node, Shape};
use crate::{Diagram, Style};
/// A box placed on the canvas, with its top-left corner at (`x`, `y`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// Index into [`Diagram::nodes`].
    pub node: usize,
    /// Left edge, in the canvas's coordinates.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// How wide the box came out.
    pub width: f64,
    /// How tall.
    pub height: f64,
}

/// One box per size, each still at the origin.
///
/// Every arrangement starts from the sizes the boxes were measured at and
/// moves them; the entries are indexed by node, so they are made in the
/// order the sizes come in and never sorted.
fn unplaced(sizes: &[(f64, f64)]) -> Vec<Placed> {
    sizes
        .iter()
        .enumerate()
        .map(|(node, &(width, height))| Placed {
            node,
            x: 0.0,
            y: 0.0,
            width,
            height,
        })
        .collect()
}

/// Placed boxes and the canvas they need.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    /// One entry per diagram node, indexed by node.
    pub placed: Vec<Placed>,
    /// How wide the whole drawing came out.
    pub width: f64,
    /// How tall.
    pub height: f64,
    /// The path an engine chose for each edge, indexed by edge and
    /// running from the edge's `from` to its `to`. Empty where the
    /// engine chose none and the renderer is to route for itself: a
    /// layout engine picks positions and routes together, and drawing
    /// straight through a layout that expected bends puts lines where
    /// the engine left no room for them.
    pub routes: Vec<Vec<(f64, f64)>>,
    /// Where each swimlane's column sits, in the diagram's lane order.
    /// Empty where the view is not partitioned by performer.
    pub lanes: Vec<Column>,
    /// Where each package's frame sits, in the diagram's group order.
    /// Empty where nothing drawn is packaged.
    pub packages: Vec<Frame>,
}

/// One package's frame: the folder the standard draws round what it
/// holds, with its name in the tab.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// One swimlane's column: what it is headed by and the band it occupies.
/// The lanes are attached to each other on their vertical edges and
/// aligned along the top and bottom, the way the standard's note has it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Column {
    pub name: String,
    pub x: f64,
    pub width: f64,
    pub top: f64,
    pub height: f64,
}

/// Assign every node of `diagram` a position.
///
/// The canvas made here holds the names drawn outside the boxes as well
/// as the boxes themselves: a port's name reads outwards from the border
/// it sits on, and on an outermost box that is off the canvas unless the
/// room is left for it.
pub fn layout(diagram: &Diagram, style: &Style) -> Layout {
    let sizes: Vec<(f64, f64)> = diagram
        .nodes
        .iter()
        .map(|node| box_size(node, style))
        .collect();
    let laid_out = if diagram.lanes.is_empty() {
        crate::elk::elk_layout(diagram, &sizes, style)
    } else {
        swimlanes(diagram, &sizes, style)
    };
    crate::svg::with_room_for_labels(diagram, &laid_out, style).unwrap_or(laid_out)
}

/// The packages one package encloses, in document order.
pub(crate) fn enclosed(diagram: &Diagram, at: usize) -> Vec<usize> {
    let depth = diagram.groups[at].depth;
    diagram
        .groups
        .iter()
        .enumerate()
        .skip(at + 1)
        .take_while(|(_, group)| group.depth > depth)
        .filter(|(_, group)| group.depth == depth + 1)
        .map(|(below, _)| below)
        .collect()
}

/// Lay a view out by who carries each node out.
///
/// `swimlane = usage-name-compartment &action-flow-node*`: each performer
/// gets a column headed by its name, holding its nodes in the order the
/// flow declares them. What nothing says a performer for is drawn in a
/// row above the columns, which is where the standard's `action-flow-view`
/// puts the elements its swimlanes do not cover.
fn swimlanes(diagram: &Diagram, sizes: &[(f64, f64)], style: &Style) -> Layout {
    let mut placed = unplaced(sizes);

    // the loose nodes first: one row, left to right, above the columns
    let laned: HashSet<usize> = diagram
        .lanes
        .iter()
        .flat_map(|lane| lane.nodes.iter().copied())
        .collect();
    let loose: Vec<usize> = (0..sizes.len()).filter(|at| !laned.contains(at)).collect();
    let mut next = style.margin;
    for &at in &loose {
        placed[at].x = next;
        placed[at].y = style.margin;
        next += sizes[at].0 + style.h_gap;
    }
    let above = loose.iter().map(|&at| sizes[at].1).fold(0.0f64, f64::max);
    let top = style.margin
        + if loose.is_empty() {
            0.0
        } else {
            above + style.v_gap
        };

    // then the columns, attached to each other and aligned top and bottom
    let heading = 2.0 * style.padding + style.line_height;
    let tallest = diagram
        .lanes
        .iter()
        .map(|lane| {
            lane.nodes
                .iter()
                .map(|&at| sizes[at].1 + style.v_gap)
                .sum::<f64>()
        })
        .fold(0.0f64, f64::max);
    let mut columns = Vec::new();
    let mut left = style.margin;
    for lane in &diagram.lanes {
        let width = lane
            .nodes
            .iter()
            .map(|&at| sizes[at].0)
            .fold(style.text_width(&lane.name), f64::max)
            + 2.0 * style.padding;
        let mut down = top + heading + style.padding;
        for &at in &lane.nodes {
            placed[at].x = left + (width - sizes[at].0) / 2.0;
            placed[at].y = down;
            down += sizes[at].1 + style.v_gap;
        }
        columns.push(Column {
            name: lane.name.clone(),
            x: left,
            width,
            top,
            height: heading + style.padding + tallest,
        });
        left += width;
    }

    Layout {
        placed,
        width: left.max(next - style.h_gap) + style.margin,
        height: top + heading + style.padding + tallest + style.margin,
        routes: Vec::new(),
        lanes: columns,
        packages: Vec::new(),
    }
}

/// Width and height of one box: wide enough for its longest line, tall
/// enough for the keyword, the name, and every compartment with its
/// label and its lines.
pub(crate) fn box_size(node: &Node, style: &Style) -> (f64, f64) {
    match node.shape {
        // a filled circle, sized to read at the same weight as a box border
        Shape::Initial | Shape::ConnectionDot => {
            let diameter = style.line_height;
            return (diameter, diameter);
        }
        // long enough that several successions can meet along it
        Shape::Bar => return (3.0 * style.line_height, 0.3 * style.line_height),
        Shape::Diamond => return (1.6 * style.line_height, 1.6 * style.line_height),
        Shape::Cross => return (style.line_height, style.line_height),
        // a note is sized by the text it holds, like a box with one line
        Shape::Note | Shape::Box => {}
    }
    // the name is drawn bold, which the 0.6 em estimate does not account for
    let mut width = (style.text_width(&node.name) * 1.1)
        .max(style.text_width(&format!("\u{ab}{}\u{bb}", node.keyword)));
    // a note draws its prose flush with its first line; a box indents a
    // line one step in from the label of the compartment holding it
    let indent = if node.shape == Shape::Note {
        0.0
    } else {
        style.padding
    };
    for compartment in &node.compartments {
        width = width.max(style.text_width(compartment.label));
        for line in &compartment.lines {
            width = width.max(indent + style.text_width(&line.label()));
        }
    }
    if node.shape == Shape::Note {
        // and it is prose and nothing else: no name set off from a
        // keyword above it, no compartment labels, no rules between them
        let said = usize::from(!node.keyword.is_empty())
            + 1
            + node
                .compartments
                .iter()
                .map(|held| held.lines.len())
                .sum::<usize>();
        return (
            width + 2.0 * style.padding,
            2.0 * style.padding + said as f64 * style.line_height,
        );
    }
    let mut height = 2.0 * style.padding + 2.0 * style.line_height;
    for compartment in &node.compartments {
        // the label, then a line each, and a gap before the next rule
        height += style.padding + (1 + compartment.lines.len()) as f64 * style.line_height;
    }
    if !node.children.is_empty() {
        let (nested_width, nested_height) = children_block(node, style);
        width = width.max(nested_width);
        height += style.padding + nested_height;
    }
    (width + 2.0 * style.padding, height)
}

/// The block the nested parts occupy inside their parent: one row of boxes,
/// held together by the padding rather than the gap that separates the
/// top-level ones.
pub(crate) fn children_block(node: &Node, style: &Style) -> (f64, f64) {
    let sizes = child_sizes(node, style);
    let width =
        sizes.iter().map(|size| size.0).sum::<f64>() + style.padding * (sizes.len() as f64 - 1.0);
    let height = sizes.iter().map(|size| size.1).fold(0.0f64, f64::max);
    (width, height)
}

fn child_sizes(node: &Node, style: &Style) -> Vec<(f64, f64)> {
    node.children
        .iter()
        .map(|child| box_size(child, style))
        .collect()
}

/// Where each nested part sits inside the box at `parent`, centred along
/// the bottom of it.
pub(crate) fn child_boxes(
    node: &Node,
    parent: (f64, f64, f64, f64),
    style: &Style,
) -> Vec<(f64, f64, f64, f64)> {
    let (x, y, width, height) = parent;
    let (block_width, block_height) = children_block(node, style);
    let mut next = x + (width - block_width) / 2.0;
    let top = y + height - style.padding - block_height;
    child_sizes(node, style)
        .into_iter()
        .map(|(child_width, child_height)| {
            let placed = (
                next,
                top + (block_height - child_height) / 2.0,
                child_width,
                child_height,
            );
            next += child_width + style.padding;
            placed
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definition_diagram;
    use crate::tests::resolved;

    fn laid_out(source: &str) -> (Diagram, Layout) {
        let ws = resolved(source);
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let layout = layout(&diagram, &Style::default());
        (diagram, layout)
    }

    fn placed_by_name<'a>(diagram: &Diagram, layout: &'a Layout, name: &str) -> &'a Placed {
        let node = diagram.nodes.iter().position(|n| n.name == name).unwrap();
        &layout.placed[node]
    }

    #[test]
    fn the_canvas_encloses_every_box() {
        let (_, layout) = laid_out("part def A;\npart def B :> A;\n");
        let style = Style::default();
        for placed in &layout.placed {
            assert!(placed.x >= style.margin);
            assert!(placed.y >= style.margin);
            assert!(placed.x + placed.width + style.margin <= layout.width + f64::EPSILON);
            assert!(placed.y + placed.height + style.margin <= layout.height + f64::EPSILON);
        }
    }

    #[test]
    fn boxes_grow_with_their_content() {
        let (diagram, layout) = laid_out(
            "part def Bare;\n\
             part def Full {\n\
             \tattribute someRatherLongAttributeName;\n\
             \tattribute another;\n\
             }\n",
        );
        let bare = placed_by_name(&diagram, &layout, "Bare");
        let full = placed_by_name(&diagram, &layout, "Full");
        assert!(full.width > bare.width);
        assert!(full.height > bare.height);
    }

    #[test]
    fn nested_parts_fit_inside_the_box_that_holds_them() {
        let ws = resolved(
            "part def Bolt;\n\
             part def Rim;\n\
             part def Wheel { part bolt : Bolt; part rim : Rim; }\n\
             part def Car {\n\
             \tpart w : Wheel;\n\
             }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let car = diagram.nodes.iter().find(|n| n.name == "Car").unwrap().id;
        let inner = crate::interconnection_diagram(ws.model(), car);
        let style = Style::default();
        let placed = layout(&inner, &style);

        let wheel = placed.placed[0];
        let parent = (wheel.x, wheel.y, wheel.width, wheel.height);
        let boxes = child_boxes(&inner.nodes[0], parent, &style);
        assert_eq!(boxes.len(), 2);
        for (x, y, width, height) in boxes {
            assert!(x >= wheel.x && x + width <= wheel.x + wheel.width);
            assert!(y >= wheel.y && y + height <= wheel.y + wheel.height);
        }
    }

    #[test]
    fn an_empty_diagram_still_yields_a_canvas() {
        let layout = layout(&Diagram::default(), &Style::default());
        assert!(layout.placed.is_empty());
        assert!(layout.width > 0.0 && layout.height > 0.0);
    }

    /// Each performer gets a column, and what nothing says a performer
    /// for is drawn in a row above them.
    #[test]
    fn a_swimlane_view_puts_each_performer_in_a_column() {
        let ws = resolved(
            "action providePower {\n\
             \taction generate;\n\
             \taction convert;\n\
             \taction unassigned;\n\
             }\n\
             part def Engine { perform providePower.generate; }\n\
             part def Gearbox { perform providePower.convert; }\n",
        );
        let providing = ws
            .named_elements()
            .find(|(_, declared)| *declared == "providePower")
            .map(|(id, _)| id)
            .unwrap();
        let diagram = crate::interconnection_diagram(ws.model(), providing);
        let style = Style::default();
        let layout = layout(&diagram, &style);

        assert_eq!(layout.lanes.len(), diagram.lanes.len());
        // the columns are attached to each other and aligned top and bottom
        for pair in layout.lanes.windows(2) {
            assert_eq!(pair[0].x + pair[0].width, pair[1].x);
            assert_eq!(pair[0].top, pair[1].top);
            assert_eq!(pair[0].height, pair[1].height);
        }
        // each performer's node sits inside its own column, under the heading
        for (lane, column) in diagram.lanes.iter().zip(&layout.lanes) {
            for &node in &lane.nodes {
                let placed = &layout.placed[node];
                assert!(placed.x >= column.x, "{placed:?} left of {column:?}");
                assert!(
                    placed.x + placed.width <= column.x + column.width,
                    "{placed:?} right of {column:?}"
                );
                assert!(placed.y > column.top, "{placed:?} above {column:?}");
            }
        }
        // and what no lane claims is above every column
        let laned: HashSet<usize> = diagram
            .lanes
            .iter()
            .flat_map(|lane| lane.nodes.iter().copied())
            .collect();
        let loose: Vec<&Placed> = layout
            .placed
            .iter()
            .filter(|placed| !laned.contains(&placed.node))
            .collect();
        assert!(!loose.is_empty());
        for placed in loose {
            assert!(
                placed.y + placed.height <= layout.lanes[0].top,
                "{placed:?}"
            );
        }
    }
}

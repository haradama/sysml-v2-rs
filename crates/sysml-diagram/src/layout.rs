//! Layered layout: every supertype sits above the subtypes that specialize
//! it, and each layer is ordered to keep the edges between layers untangled.

use std::collections::HashMap;

use crate::graph::{Node, Relation, Shape};
use crate::{Diagram, Edge, Style};

/// How many times the crossing-reduction pass sweeps the layers. Four is the
/// point past which the orderings stop changing for diagrams of this size.
const SWEEPS: usize = 4;

/// A box placed on the canvas, with its top-left corner at (`x`, `y`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// Index into [`Diagram::nodes`].
    pub node: usize,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Placed boxes and the canvas they need.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    /// One entry per diagram node, indexed by node.
    pub placed: Vec<Placed>,
    pub width: f64,
    pub height: f64,
}

/// Assign every node of `diagram` a position.
pub fn layout(diagram: &Diagram, style: &Style) -> Layout {
    let sizes: Vec<(f64, f64)> = diagram
        .nodes
        .iter()
        .map(|node| box_size(node, style))
        .collect();
    let gaps = label_gaps(diagram, style);
    let ranks = ranks(diagram);
    let layers = order_layers(diagram, &ranks);
    let rows = wrap_layers(&layers, &sizes, &gaps, style);
    place(&sizes, &rows, &gaps, style)
}

/// How much room the names drawn between two boxes need, per pair.
///
/// A connection names the port at each end and a transition names itself,
/// and all of those sit between the boxes. At the default gap they pile up
/// on each other as soon as two connections share a pair of boxes. Only the
/// pair a name is drawn between is widened: one long transition label would
/// otherwise push every unrelated box in the row apart with it.
fn label_gaps(diagram: &Diagram, style: &Style) -> HashMap<(usize, usize), f64> {
    let mut gaps: HashMap<(usize, usize), f64> = HashMap::new();
    for edge in &diagram.edges {
        let mut needed: f64 = 0.0;
        if let Some((first, second)) = &edge.ends {
            // one name per end, each set clear of its own port
            let widest = style.text_width(first).max(style.text_width(second));
            needed = needed.max(2.0 * widest + style.line_height);
        }
        if let Some(label) = &edge.label {
            needed = needed.max(style.text_width(label) + style.line_height);
        }
        let room = gaps.entry(pair(edge.from, edge.to)).or_default();
        *room = room.max(needed);
    }
    gaps
}

/// Two nodes as one key, whichever way round the edge between them runs.
fn pair(one: usize, other: usize) -> (usize, usize) {
    (one.min(other), one.max(other))
}

/// The gap to leave between two boxes drawn side by side.
fn gap_between(
    gaps: &HashMap<(usize, usize), f64>,
    one: usize,
    other: usize,
    style: &Style,
) -> f64 {
    style
        .h_gap
        .max(gaps.get(&pair(one, other)).copied().unwrap_or_default())
}

/// Split each layer into rows no wider than [`Style::max_row_width`].
///
/// A layer holding hundreds of unrelated definitions would otherwise
/// stretch the canvas into a strip tens of thousands of pixels wide; wrapping
/// keeps it readable. A single box wider than the budget still gets its own
/// row rather than being dropped.
fn wrap_layers(
    layers: &[Vec<usize>],
    sizes: &[(f64, f64)],
    gaps: &HashMap<(usize, usize), f64>,
    style: &Style,
) -> Vec<Vec<usize>> {
    let mut rows: Vec<Vec<usize>> = Vec::new();
    for layer in layers {
        let mut row: Vec<usize> = Vec::new();
        let mut width = 0.0;
        for &node in layer {
            let grown = match row.last() {
                Some(&previous) => width + gap_between(gaps, previous, node, style) + sizes[node].0,
                None => sizes[node].0,
            };
            if !row.is_empty() && grown > style.max_row_width {
                rows.push(std::mem::take(&mut row));
                width = sizes[node].0;
            } else {
                width = grown;
            }
            row.push(node);
        }
        rows.push(row);
    }
    rows
}

/// Width and height of one box: wide enough for its longest line, tall
/// enough for the keyword, the name and one line per feature.
fn box_size(node: &Node, style: &Style) -> (f64, f64) {
    if node.shape == Shape::Initial {
        // a filled circle, sized to read at the same weight as a box border
        let diameter = style.line_height;
        return (diameter, diameter);
    }
    // the name is drawn bold, which the 0.6 em estimate does not account for
    let mut width = (style.text_width(&node.name) * 1.1)
        .max(style.text_width(&format!("\u{ab}{}\u{bb}", node.keyword)));
    for feature in &node.features {
        width = width.max(style.text_width(&feature.label()));
    }
    let mut height = 2.0 * style.padding + 2.0 * style.line_height;
    if !node.features.is_empty() {
        height += style.padding + node.features.len() as f64 * style.line_height;
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

/// Only specializations order the layers. Composition runs from a whole to
/// its parts, the opposite way round, so mixing the two would fight over
/// which box belongs above the other.
fn specializations(diagram: &Diagram) -> impl Iterator<Item = &Edge> {
    diagram
        .edges
        .iter()
        .filter(|edge| edge.relation == Relation::Specialization)
}

/// Layer index of each node: 0 when it has no supertype inside the diagram,
/// otherwise one below its deepest supertype.
///
/// Relaxation runs at most once per node, so a specialization cycle -- which
/// the parser accepts and name resolution happily reifies -- settles instead
/// of looping forever.
fn ranks(diagram: &Diagram) -> Vec<usize> {
    let mut ranks = vec![0usize; diagram.nodes.len()];
    for _ in 0..diagram.nodes.len() {
        let mut changed = false;
        for edge in specializations(diagram) {
            if ranks[edge.from] <= ranks[edge.to] {
                ranks[edge.from] = ranks[edge.to] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    ranks
}

/// Order each layer by the barycenter heuristic: repeatedly sort a layer by
/// the mean position of each node's supertypes in the layers above.
fn order_layers(diagram: &Diagram, ranks: &[usize]) -> Vec<Vec<usize>> {
    let depth = ranks.iter().copied().max().map_or(0, |deepest| deepest + 1);
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); depth];
    for (node, &rank) in ranks.iter().enumerate() {
        layers[rank].push(node);
    }

    let mut position = vec![0.0f64; ranks.len()];
    for layer in &layers {
        for (slot, &node) in layer.iter().enumerate() {
            position[node] = slot as f64;
        }
    }

    // the top layer has nothing above it to be ordered against
    for _ in 0..SWEEPS {
        for layer in layers.iter_mut().skip(1) {
            let mut keyed: Vec<(f64, usize)> = layer
                .iter()
                .map(|&node| (barycenter(diagram, node, &position), node))
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (slot, &(_, node)) in keyed.iter().enumerate() {
                position[node] = slot as f64;
            }
            *layer = keyed.into_iter().map(|(_, node)| node).collect();
        }
    }
    group_within_layers(diagram, &mut layers);
    layers
}

/// Bring boxes joined to each other within one layer together, keeping the
/// order the crossing reduction settled on otherwise.
///
/// Only specializations decide the layers, so a whole and its parts often
/// land in the same one. An unrelated definition declared between them
/// would then sit between them on the canvas as well, and the line joining
/// them would have to go the long way round something it has nothing to do
/// with.
fn group_within_layers(diagram: &Diagram, layers: &mut [Vec<usize>]) {
    for layer in layers.iter_mut() {
        let mut grouped: Vec<usize> = Vec::with_capacity(layer.len());
        let mut taken = vec![false; layer.len()];
        for start in 0..layer.len() {
            if taken[start] {
                continue;
            }
            taken[start] = true;
            let mut group = vec![start];
            let mut next = 0;
            // everything reachable from here without leaving the layer
            while next < group.len() {
                let node = layer[group[next]];
                next += 1;
                for slot in 0..layer.len() {
                    if !taken[slot] && joined(diagram, node, layer[slot]) {
                        taken[slot] = true;
                        group.push(slot);
                    }
                }
            }
            // within a group the layer's own order still stands
            group.sort_unstable();
            grouped.extend(group.into_iter().map(|slot| layer[slot]));
        }
        *layer = grouped;
    }
}

/// Is there an edge between these two boxes, either way round?
fn joined(diagram: &Diagram, one: usize, other: usize) -> bool {
    diagram.edges.iter().any(|edge| {
        (edge.from == one && edge.to == other) || (edge.from == other && edge.to == one)
    })
}

/// Mean position of a node's supertypes, or its own position when it has
/// none -- an unconnected node then keeps the slot it started in.
fn barycenter(diagram: &Diagram, node: usize, position: &[f64]) -> f64 {
    let mut sum = 0.0;
    let mut count = 0.0;
    for edge in specializations(diagram) {
        if edge.from == node {
            sum += position[edge.to];
            count += 1.0;
        }
    }
    if count == 0.0 {
        position[node]
    } else {
        sum / count
    }
}

/// Turn row orderings into coordinates, centring every row against the
/// widest one and centring each box vertically within its row.
fn place(
    sizes: &[(f64, f64)],
    layers: &[Vec<usize>],
    gaps: &HashMap<(usize, usize), f64>,
    style: &Style,
) -> Layout {
    let mut placed = vec![
        Placed {
            node: 0,
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        };
        sizes.len()
    ];
    if layers.is_empty() {
        return Layout {
            placed,
            width: 2.0 * style.margin,
            height: 2.0 * style.margin,
        };
    }

    let rows: Vec<(f64, f64)> = layers
        .iter()
        .map(|layer| {
            let width = layer.iter().map(|&node| sizes[node].0).sum::<f64>()
                + layer
                    .windows(2)
                    .map(|side_by_side| gap_between(gaps, side_by_side[0], side_by_side[1], style))
                    .sum::<f64>();
            let height = layer
                .iter()
                .map(|&node| sizes[node].1)
                .fold(0.0f64, f64::max);
            (width, height)
        })
        .collect();
    let content = rows.iter().map(|row| row.0).fold(0.0f64, f64::max);

    let mut y = style.margin;
    for (layer, &(row_width, row_height)) in layers.iter().zip(&rows) {
        let mut x = style.margin + (content - row_width) / 2.0;
        for (at, &node) in layer.iter().enumerate() {
            let (width, height) = sizes[node];
            placed[node] = Placed {
                node,
                x,
                y: y + (row_height - height) / 2.0,
                width,
                height,
            };
            x += width;
            if let Some(&next) = layer.get(at + 1) {
                x += gap_between(gaps, node, next, style);
            }
        }
        y += row_height + style.v_gap;
    }

    Layout {
        placed,
        width: content + 2.0 * style.margin,
        height: y - style.v_gap + style.margin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definition_diagram;
    use crate::tests::resolved;

    fn laid_out_internal(source: &str, owner: &str) -> (Diagram, Layout) {
        let ws = resolved(source);
        let model = ws.model();
        let id = model
            .descendants(ws.root())
            .into_iter()
            .find(|&id| model.name(id) == Some(owner))
            .expect("the owner is in the model");
        let diagram = crate::interconnection_diagram(model, id);
        let layout = layout(&diagram, &Style::default());
        (diagram, layout)
    }

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
    fn supertypes_sit_above_their_subtypes() {
        let (diagram, layout) = laid_out(
            "part def PowerSource;\n\
             part def Engine :> PowerSource;\n\
             part def Turbofan :> Engine;\n",
        );
        let source = placed_by_name(&diagram, &layout, "PowerSource");
        let engine = placed_by_name(&diagram, &layout, "Engine");
        let turbofan = placed_by_name(&diagram, &layout, "Turbofan");

        assert!(source.y + source.height < engine.y);
        assert!(engine.y + engine.height < turbofan.y);
    }

    #[test]
    fn boxes_in_one_layer_do_not_overlap() {
        let (diagram, layout) = laid_out(
            "part def A;\n\
             part def B;\n\
             part def LongerNameHere;\n",
        );
        let mut row: Vec<&Placed> = diagram
            .nodes
            .iter()
            .enumerate()
            .map(|(i, _)| &layout.placed[i])
            .collect();
        row.sort_by(|a, b| a.x.total_cmp(&b.x));

        for pair in row.windows(2) {
            assert!(pair[0].x + pair[0].width <= pair[1].x, "overlap: {pair:?}");
        }
        // all three are unconnected, so they share one layer
        assert!(row.iter().all(|p| p.y == row[0].y));
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
             	attribute someRatherLongAttributeName;\n\
             	attribute another;\n\
             }\n",
        );
        let bare = placed_by_name(&diagram, &layout, "Bare");
        let full = placed_by_name(&diagram, &layout, "Full");
        assert!(full.width > bare.width);
        assert!(full.height > bare.height);
    }

    #[test]
    fn a_specialization_cycle_terminates() {
        let (diagram, layout) = laid_out("part def A :> B;\npart def B :> A;\n");
        assert_eq!(diagram.edges.len(), 2);
        // both ranks stay finite, so every box lands on the canvas
        assert!(layout.placed.iter().all(|p| p.y.is_finite()));
        assert!(layout.height.is_finite());
    }

    #[test]
    fn a_self_specialization_terminates() {
        let mut diagram = Diagram::default();
        let ws = resolved("part def A;\n");
        diagram.nodes = definition_diagram(ws.model(), &[ws.root()]).nodes;
        diagram.edges = vec![Edge {
            from: 0,
            to: 0,
            relation: Relation::Specialization,
            ends: None,
            label: None,
        }];

        let ranks = ranks(&diagram);
        assert_eq!(ranks.len(), 1);
        assert!(ranks[0] <= diagram.nodes.len());
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
    fn labels_widen_the_gap_they_are_drawn_in() {
        let style = Style::default();
        let mut diagram = Diagram::default();
        assert!(label_gaps(&diagram, &style).is_empty());

        // a connection sets two names down, one per end
        diagram.edges = vec![Edge {
            from: 0,
            to: 1,
            relation: Relation::Connection,
            ends: Some(("aVeryLongPortName".to_string(), "short".to_string())),
            label: None,
        }];
        let widened = gap_between(&label_gaps(&diagram, &style), 0, 1, &style);
        // room for the longer name at both ends
        assert!(widened > 2.0 * style.text_width("aVeryLongPortName"));

        // a transition sets one down, in the middle
        diagram.edges = vec![Edge {
            from: 0,
            to: 1,
            relation: Relation::Transition,
            ends: None,
            label: Some("aVeryLongTransitionName".to_string()),
        }];
        let gaps = label_gaps(&diagram, &style);
        assert!(gap_between(&gaps, 0, 1, &style) > style.h_gap);
        // and the pair it is drawn between is the only one widened, whichever
        // way round the edge is read
        assert_eq!(
            gap_between(&gaps, 1, 0, &style),
            gap_between(&gaps, 0, 1, &style)
        );
        assert_eq!(gap_between(&gaps, 1, 2, &style), style.h_gap);
    }

    #[test]
    fn a_long_label_leaves_the_rest_of_the_row_alone() {
        // `a` and `b` are joined by a long transition; `sink` is joined to
        // nothing and must not be pushed away by it
        let (_, layout) = laid_out_internal(
            "state def S {\n\
             	state a;\n\
             	state b;\n\
             	state sink;\n\
             	transition aVeryLongTransitionNameIndeed first a then b;\n\
             }\n",
            "S",
        );
        let mut placed: Vec<Placed> = layout.placed.clone();
        placed.sort_by(|one, other| one.x.total_cmp(&other.x));
        let gaps: Vec<f64> = placed
            .windows(2)
            .map(|side_by_side| side_by_side[1].x - (side_by_side[0].x + side_by_side[0].width))
            .collect();
        let style = Style::default();
        // exactly one gap carries the label; the others stay at the default
        assert_eq!(
            gaps.iter().filter(|&&gap| gap > style.h_gap + 1.0).count(),
            1,
            "gaps: {gaps:?}"
        );
    }

    #[test]
    fn a_whole_is_placed_beside_its_parts() {
        // `FuelPort` is declared between `Wheel` and `Vehicle` but has
        // nothing to do with either, so it must not come between them
        let (diagram, layout) = laid_out(
            "part def Wheel;\n\
             port def FuelPort;\n\
             part def Vehicle {\n\
             	part wheels : Wheel;\n\
             }\n",
        );
        let named = |name: &str| {
            diagram
                .nodes
                .iter()
                .position(|node| node.name == name)
                .map(|node| layout.placed[node].x)
                .expect("the box is drawn")
        };
        let (wheel, port, vehicle) = (named("Wheel"), named("FuelPort"), named("Vehicle"));
        assert!(
            port < wheel.min(vehicle) || port > wheel.max(vehicle),
            "FuelPort sits between the two boxes it is unrelated to"
        );
    }

    #[test]
    fn composition_does_not_order_the_layers() {
        let (diagram, layout) = laid_out(
            "part def Engine;\n\
             part def Vehicle {\n\
             	part eng : Engine;\n\
             }\n",
        );
        assert_eq!(diagram.edges.len(), 1);
        // a whole is not a subtype of its parts, so both stay on one row --
        // boxes of unequal height are centred in it, so the centres match
        let engine = placed_by_name(&diagram, &layout, "Engine");
        let vehicle = placed_by_name(&diagram, &layout, "Vehicle");
        assert_eq!(
            engine.y + engine.height / 2.0,
            vehicle.y + vehicle.height / 2.0
        );
    }

    #[test]
    fn a_wide_layer_wraps_instead_of_stretching_the_canvas() {
        let source: String = (0..12)
            .map(|i| format!("part def Definition{i};\n"))
            .collect();
        let ws = resolved(&source);
        let diagram = definition_diagram(ws.model(), &[ws.root()]);

        let narrow = Style {
            max_row_width: 400.0,
            ..Style::default()
        };
        let wrapped = layout(&diagram, &narrow);
        let wide = layout(&diagram, &Style::default());

        assert!(wrapped.width < wide.width);
        assert!(wrapped.height > wide.height);
        assert!(wrapped.width <= narrow.max_row_width + 2.0 * narrow.margin);
        // every box still lands somewhere on the canvas
        assert_eq!(wrapped.placed.len(), 12);
        assert!(wrapped
            .placed
            .iter()
            .all(|p| p.x + p.width <= wrapped.width && p.y + p.height <= wrapped.height));
    }

    #[test]
    fn a_box_wider_than_the_budget_gets_its_own_row() {
        let ws = resolved("part def Short;\npart def AVeryLongDefinitionNameIndeed;\n");
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let style = Style {
            max_row_width: 1.0,
            ..Style::default()
        };
        let placed = layout(&diagram, &style);

        assert_eq!(placed.placed.len(), 2);
        assert_ne!(placed.placed[0].y, placed.placed[1].y);
    }

    #[test]
    fn a_node_with_no_supertype_keeps_its_slot() {
        // `ranks` only ever puts a node below the top layer because it has a
        // supertype, so this fallback is reachable only directly
        assert_eq!(barycenter(&Diagram::default(), 0, &[3.0]), 3.0);
    }

    #[test]
    fn an_empty_diagram_still_yields_a_canvas() {
        let layout = layout(&Diagram::default(), &Style::default());
        assert!(layout.placed.is_empty());
        assert!(layout.width > 0.0 && layout.height > 0.0);
    }

    #[test]
    fn the_barycenter_pass_untangles_crossing_edges() {
        // A and B are declared before their subtypes, which are declared in
        // the opposite order: without reordering, the two edges cross.
        let (diagram, layout) = laid_out(
            "part def A;\n\
             part def B;\n\
             part def SubB :> B;\n\
             part def SubA :> A;\n",
        );
        let a = placed_by_name(&diagram, &layout, "A").x;
        let b = placed_by_name(&diagram, &layout, "B").x;
        let sub_a = placed_by_name(&diagram, &layout, "SubA").x;
        let sub_b = placed_by_name(&diagram, &layout, "SubB").x;

        assert_eq!(a < b, sub_a < sub_b, "the two edges still cross");
    }

    #[test]
    fn layers_are_centred_against_the_widest_one() {
        let (diagram, layout) = laid_out(
            "part def Root;\n\
             part def One :> Root;\n\
             part def Two :> Root;\n",
        );
        let root = placed_by_name(&diagram, &layout, "Root");
        let one = placed_by_name(&diagram, &layout, "One");
        let two = placed_by_name(&diagram, &layout, "Two");

        let root_centre = root.x + root.width / 2.0;
        let row_centre = (one.x + (two.x + two.width)) / 2.0;
        assert!((root_centre - row_centre).abs() < 1.0);
    }
}

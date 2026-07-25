//! Layered layout: every supertype sits above the subtypes that specialize
//! it, and each layer is ordered to keep the edges between layers untangled.

use crate::graph::Node;
use crate::{Diagram, Style};

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
    let ranks = ranks(diagram);
    let layers = order_layers(diagram, &ranks);
    place(&sizes, &layers, style)
}

/// Width and height of one box: wide enough for its longest line, tall
/// enough for the keyword, the name and one line per feature.
fn box_size(node: &Node, style: &Style) -> (f64, f64) {
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
    (width + 2.0 * style.padding, height)
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
        for edge in &diagram.edges {
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
    layers
}

/// Mean position of a node's supertypes, or its own position when it has
/// none -- an unconnected node then keeps the slot it started in.
fn barycenter(diagram: &Diagram, node: usize, position: &[f64]) -> f64 {
    let mut sum = 0.0;
    let mut count = 0.0;
    for edge in &diagram.edges {
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

/// Turn layer orderings into coordinates, centring every layer against the
/// widest one and centring each box vertically within its layer.
fn place(sizes: &[(f64, f64)], layers: &[Vec<usize>], style: &Style) -> Layout {
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
                + style.h_gap * (layer.len() as f64 - 1.0);
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
        for &node in layer {
            let (width, height) = sizes[node];
            placed[node] = Placed {
                node,
                x,
                y: y + (row_height - height) / 2.0,
                width,
                height,
            };
            x += width + style.h_gap;
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
        diagram.edges = vec![crate::Edge { from: 0, to: 0 }];

        let ranks = ranks(&diagram);
        assert_eq!(ranks.len(), 1);
        assert!(ranks[0] <= diagram.nodes.len());
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

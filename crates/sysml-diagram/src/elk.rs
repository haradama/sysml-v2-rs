//! Where the boxes go: the Eclipse Layout Kernel.
//!
//! `elkrs` is ELK's own algorithms ported to Rust, so the engine is linked
//! in and runs here rather than being spawned and spoken to in JSON.
//! Everything visible is still drawn by this crate's renderer in the
//! standard's notation; only the positions and the routes come from ELK.
//! Both, because it picks them together: a line drawn straight across a
//! layout arranged for bends ends up where ELK left no room. Where ELK
//! routes nothing, the renderer routes for itself.

use elkrs::alg_layered::options_gen::SPACING_NODE_NODE_BETWEEN_LAYERS;
use elkrs::core::engine::RecursiveGraphLayoutEngine;
use elkrs::core::options_gen::{
    Direction, ElkPadding, HierarchyHandling, ALGORITHM, DIRECTION, HIERARCHY_HANDLING, PADDING,
    SPACING_NODE_NODE,
};
use elkrs::graph::graph::{EdgeId, ElementId, ElkGraph, NodeId, ShapeId};

use crate::graph::Relation;
use crate::layout::{enclosed, Frame, Layout, Placed};
use crate::{Diagram, Style};

/// Where ELK puts every box of `diagram`, at the sizes it was measured at.
pub(crate) fn elk_layout(diagram: &Diagram, sizes: &[(f64, f64)], style: &Style) -> Layout {
    let mut built = Built::of(diagram, sizes, style);
    let elk = elkrs::create_elk();
    RecursiveGraphLayoutEngine::new(&elk.algorithms)
        .layout(&mut built.graph)
        .expect("every node this crate builds asks for an algorithm ELK has");
    built.read(diagram, sizes, style)
}

/// The diagram as an ELK graph, with what is needed to read the answer
/// back: which ELK node each box and each package became, and which ELK
/// edge each relation did.
struct Built {
    graph: ElkGraph,
    nodes: Vec<Option<NodeId>>,
    groups: Vec<NodeId>,
    /// Indexed by relation, as the routes read back from it are.
    edges: Vec<Option<EdgeId>>,
}

impl Built {
    /// Boxes at the sizes this crate measured, packages as nodes holding
    /// what they contain, and the relations between them as edges.
    fn of(diagram: &Diagram, sizes: &[(f64, f64)], style: &Style) -> Built {
        let mut built = Built {
            graph: ElkGraph::new(),
            nodes: vec![None; diagram.nodes.len()],
            groups: Vec::new(),
            edges: vec![None; diagram.edges.len()],
        };

        let root = built.graph.root;
        let options = &built.graph.node(root).properties;
        options.set(&ALGORITHM, "layered".to_string());
        options.set(&DIRECTION, Direction::DOWN);
        options.set(&SPACING_NODE_NODE, style.h_gap);
        options.set(&SPACING_NODE_NODE_BETWEEN_LAYERS, style.v_gap);
        // a package holds what it contains, so it goes to ELK as a node
        // with children and the engine arranges each package's contents
        // inside it
        if !diagram.groups.is_empty() {
            options.set(&HIERARCHY_HANDLING, HierarchyHandling::INCLUDE_CHILDREN);
        }

        built.groups = vec![root; diagram.groups.len()];
        for at in 0..diagram.groups.len() {
            if diagram.groups[at].depth == 0 {
                built.package(diagram, sizes, style, at, root);
            }
        }
        for (at, &(width, height)) in sizes.iter().enumerate() {
            if diagram.groups.iter().any(|group| group.nodes.contains(&at)) {
                continue;
            }
            built.box_at(at, width, height, root);
        }

        let owners = edge_owners(diagram);
        built.edges_within(diagram, &owners, style, None);
        built
    }

    /// One package as a node holding its own boxes and the packages
    /// nested inside it. The padding leaves room at the top for the tab
    /// the standard draws the package's name in.
    fn package(
        &mut self,
        diagram: &Diagram,
        sizes: &[(f64, f64)],
        style: &Style,
        at: usize,
        parent: NodeId,
    ) {
        let id = self.graph.create_node(Some(parent));
        self.groups[at] = id;
        let pad = style.padding;
        // the tab and then the same clearance the other three sides get:
        // padding of exactly the tab would leave the first box's top
        // border drawn along the tab's bottom, which reads as one line
        let top = style.package_tab() + pad;
        let options = &self.graph.node(id).properties;
        options.set(&ALGORITHM, "layered".to_string());
        options.set(&DIRECTION, Direction::DOWN);
        options.set(&PADDING, ElkPadding::new(top, pad, pad, pad));

        for &node in &diagram.groups[at].nodes {
            let (width, height) = sizes[node];
            self.box_at(node, width, height, id);
        }
        for below in enclosed(diagram, at) {
            self.package(diagram, sizes, style, below, id);
        }
    }

    /// One box, at the size this crate measured it.
    fn box_at(&mut self, at: usize, width: f64, height: f64, parent: NodeId) {
        let id = self.graph.create_node(Some(parent));
        self.graph.node_mut(id).shape.set_dimensions(width, height);
        self.nodes[at] = Some(id);
    }

    /// The edges one node owns, and then those its packages own. ELK
    /// resolves an edge's ends within the node it is written in, so each
    /// edge belongs to the innermost package holding both of them.
    fn edges_within(
        &mut self,
        diagram: &Diagram,
        owners: &[Option<usize>],
        style: &Style,
        within: Option<usize>,
    ) {
        let holder = within.map_or(self.graph.root, |at| self.groups[at]);
        for (at, edge) in diagram.edges.iter().enumerate() {
            if owners[at] != within {
                continue;
            }
            let (tail, head) = if points_upward(edge.relation) {
                (edge.to, edge.from)
            } else {
                (edge.from, edge.to)
            };
            let (tail, head) = (self.at(tail), self.at(head));
            let id = self.graph.create_edge(Some(holder));
            self.graph.add_edge_source(id, ShapeId::Node(tail));
            self.graph.add_edge_target(id, ShapeId::Node(head));
            self.edges[at] = Some(id);
            // What a line says is written across the middle of it, and a
            // transition's trigger and effect can be longer than
            // everything they run between. ELK is told the words so that
            // it leaves the room, as it does for a box.
            if let Some(said) = &edge.label {
                let label = self.graph.create_label(said, ElementId::Edge(id));
                self.graph
                    .label_mut(label)
                    .shape
                    .set_dimensions(style.text_width(said), style.line_height);
            }
        }
        for at in 0..diagram.groups.len() {
            if enclosing(diagram, at) == within {
                self.edges_within(diagram, owners, style, Some(at));
            }
        }
    }

    /// The ELK node a box became.
    fn at(&self, node: usize) -> NodeId {
        self.nodes[node].expect("every box is built exactly once")
    }

    /// The positions ELK chose, on this crate's canvas.
    fn read(&self, diagram: &Diagram, sizes: &[(f64, f64)], style: &Style) -> Layout {
        let placed = sizes
            .iter()
            .enumerate()
            .map(|(at, &(width, height))| {
                let (x, y) = self.corner(self.at(at), style);
                Placed {
                    node: at,
                    x,
                    y,
                    width,
                    height,
                }
            })
            .collect();
        let mut packages: Vec<Frame> = diagram
            .groups
            .iter()
            .zip(&self.groups)
            .map(|(group, &id)| {
                let (x, y) = self.corner(id, style);
                let shape = &self.graph.node(id).shape;
                Frame {
                    name: group.name.clone(),
                    x,
                    y,
                    width: shape.width,
                    height: shape.height,
                }
            })
            .collect();
        // The tab is drawn here and not by the engine, which sizes a
        // package by what it holds: one small box under a long name comes
        // back as a frame narrower than the name written across the top of
        // it. Asking the engine for a minimum size instead is worth less
        // than it looks: it applies one to a nested node along its own
        // axis, so a frame told to be wide comes back tall.
        for frame in &mut packages {
            // room for the name and for the corner the tab is cut off
            // at, which the drawing would otherwise take out of it
            frame.width = frame
                .width
                .max(style.name_width(&frame.name) + 2.0 * style.padding + style.package_slant());
        }
        let canvas = &self.graph.node(self.graph.root).shape;
        let width = packages
            .iter()
            .fold(canvas.width + 2.0 * style.margin, |canvas: f64, frame| {
                canvas.max(frame.x + frame.width + style.margin)
            });
        Layout {
            placed,
            width,
            height: canvas.height + 2.0 * style.margin,
            routes: self.routes(diagram, style),
            // a laned view never reaches here; it is placed by this crate
            lanes: Vec::new(),
            packages,
        }
    }

    /// Where a node sits on the canvas. ELK places a child within its
    /// parent, so the corners of everything holding it add up.
    fn corner(&self, node: NodeId, style: &Style) -> (f64, f64) {
        let (mut x, mut y) = (style.margin, style.margin);
        let mut at = Some(node);
        while let Some(id) = at.filter(|&id| id != self.graph.root) {
            x += self.graph.node(id).shape.x;
            y += self.graph.node(id).shape.y;
            at = self.graph.node(id).parent;
        }
        (x, y)
    }

    /// The path ELK chose for each edge, in the diagram's own order and
    /// running from the edge's `from` to its `to`. An edge ELK said
    /// nothing about keeps an empty route, and the renderer routes it.
    ///
    /// A section is in the coordinates of the node the edge is written
    /// in, which is why that node's corner is added to every point.
    fn routes(&self, diagram: &Diagram, style: &Style) -> Vec<Vec<(f64, f64)>> {
        self.edges
            .iter()
            .enumerate()
            .map(|(at, id)| {
                let edge = self.graph.edge(id.expect("every relation becomes an edge"));
                edge.sections.first().map_or_else(Vec::new, |&section| {
                    let (left, top) = edge
                        .containing_node
                        .map_or((style.margin, style.margin), |node| {
                            self.corner(node, style)
                        });
                    let section = self.graph.section(section);
                    let mut walked = vec![(left + section.start_x, top + section.start_y)];
                    for &(x, y) in &section.bend_points {
                        walked.push((left + x, top + y));
                    }
                    walked.push((left + section.end_x, top + section.end_y));
                    // the graph points a hierarchical edge the way it
                    // should be read, which is the opposite of the way it
                    // is drawn
                    if points_upward(diagram.edges[at].relation) {
                        walked.reverse();
                    }
                    walked
                })
            })
            .collect()
    }
}

/// The packages each edge is written in: the innermost one holding both
/// of its ends, or the whole drawing where they are in different packages.
fn edge_owners(diagram: &Diagram) -> Vec<Option<usize>> {
    let holders: Vec<Vec<usize>> = (0..diagram.nodes.len())
        .map(|node| holding(diagram, node))
        .collect();
    diagram
        .edges
        .iter()
        .map(|edge| {
            holders[edge.from]
                .iter()
                .zip(&holders[edge.to])
                .take_while(|(one, other)| one == other)
                .map(|(one, _)| *one)
                .last()
        })
        .collect()
}

/// The packages a box is inside, outermost first.
fn holding(diagram: &Diagram, node: usize) -> Vec<usize> {
    let Some(innermost) = diagram
        .groups
        .iter()
        .position(|group| group.nodes.contains(&node))
    else {
        return Vec::new();
    };
    let mut chain = vec![innermost];
    let mut depth = diagram.groups[innermost].depth;
    for (at, group) in diagram.groups[..innermost].iter().enumerate().rev() {
        if depth > 0 && group.depth == depth - 1 {
            chain.push(at);
            depth -= 1;
        }
    }
    chain.reverse();
    chain
}

/// The package a package is nested in, if any.
fn enclosing(diagram: &Diagram, group: usize) -> Option<usize> {
    (0..group).find(|&at| enclosed(diagram, at).contains(&group))
}

/// Whether the box an edge is drawn towards is the one that should sit
/// higher -- the supertype, the requirement -- so that ELK is told the
/// edge the other way round from the way it is drawn.
fn points_upward(relation: Relation) -> bool {
    matches!(relation, Relation::Specialization | Relation::Satisfy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::resolved;
    use crate::{definition_diagram, render};

    /// A model with every relation kind the ELK emission distinguishes.
    const SOURCE: &str = "part def A;\n\
                          part def B :> A;\n\
                          requirement def R;\n\
                          satisfy R by B;\n";

    fn diagram() -> Diagram {
        let ws = resolved(SOURCE);
        definition_diagram(ws.model(), &[ws.root()])
    }

    /// Measure the boxes and lay them out, which is what `crate::layout`
    /// does around this module.
    fn laid_out(diagram: &Diagram, style: &Style) -> Layout {
        let sizes: Vec<(f64, f64)> = diagram
            .nodes
            .iter()
            .map(|node| crate::layout::box_size(node, style))
            .collect();
        elk_layout(diagram, &sizes, style)
    }

    /// Where the box named `name` was placed.
    fn at<'a>(diagram: &Diagram, layout: &'a Layout, name: &str) -> &'a Placed {
        let node = diagram
            .nodes
            .iter()
            .position(|node| node.name == name)
            .unwrap_or_else(|| panic!("no box named {name}"));
        &layout.placed[node]
    }

    #[test]
    fn the_graph_points_the_way_it_reads() {
        let ws = resolved(
            "part def A;\npart def B :> A;\npart def W { part b : B; }\n\
             part def P { port p : A; }\npart def Q { port q : A; }\n\
             part def Pair { part l : P; part r : Q; connect l.p to r.q; }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let style = Style::default();
        let layout = laid_out(&diagram, &style);

        // the supertype lands above its subtype, and the whole above its
        // part, whichever way round the relation is drawn
        assert!(at(&diagram, &layout, "A").y < at(&diagram, &layout, "B").y);
        assert!(at(&diagram, &layout, "W").y < at(&diagram, &layout, "B").y);
        // every box is on the canvas the layout asks for
        for placed in &layout.placed {
            assert!(placed.x >= style.margin, "{placed:?}");
            assert!(placed.x + placed.width <= layout.width, "{placed:?}");
            assert!(placed.y + placed.height <= layout.height, "{placed:?}");
        }
        // the sizes stay this crate's own: ELK is told them and echoes them
        for (placed, node) in layout.placed.iter().zip(&diagram.nodes) {
            assert_eq!(
                (placed.width, placed.height),
                crate::layout::box_size(node, &style)
            );
        }

        // a connection -- drawn in the interconnection view -- is laid
        // out like any other edge: ELK arranges a graph, not a tree
        let model = ws.model();
        let pair = model
            .descendants(ws.root())
            .into_iter()
            .find(|&id| model.name(id) == Some("Pair"))
            .unwrap();
        let inside = crate::interconnection_diagram(model, pair);
        let wired = laid_out(&inside, &style);
        assert_eq!(wired.placed.len(), inside.nodes.len());
        assert_ne!(wired.placed[0].y, wired.placed[1].y);
    }

    /// A package holding a sub-package that holds the definitions.
    const NESTED: &str = "package Outer {\n\
                          package Parts { part def Engine; }\n\
                          package Wholes { part def Car { part e : Parts::Engine; } }\n\
                          }\n";

    #[test]
    fn a_package_frames_what_it_holds() {
        let style = Style::default();
        let ws = resolved(NESTED);
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let names: Vec<&str> = diagram.groups.iter().map(|g| g.name.as_str()).collect();
        // `Outer` owns no definition of its own and is a frame all the same
        assert_eq!(names, ["Outer", "Parts", "Wholes"]);
        assert_eq!(diagram.groups[0].nodes, Vec::<usize>::new());

        let layout = laid_out(&diagram, &style);
        let frame = |name: &str| {
            layout
                .packages
                .iter()
                .find(|frame| frame.name == name)
                .unwrap()
        };
        let holds = |outer: &Frame, x: f64, y: f64, width: f64, height: f64| {
            x >= outer.x
                && y >= outer.y
                && x + width <= outer.x + outer.width
                && y + height <= outer.y + outer.height
        };
        // each frame is inside the one that encloses it
        let outer = frame("Outer");
        for inner in ["Parts", "Wholes"] {
            let inner = frame(inner);
            let (x, y, width, height) = (inner.x, inner.y, inner.width, inner.height);
            assert!(
                holds(outer, x, y, width, height),
                "{inner:?} not in {outer:?}"
            );
        }
        // and every box is inside its own frame, below the tab its name
        // is drawn in
        for (name, holder) in [("Engine", "Parts"), ("Car", "Wholes")] {
            let placed = at(&diagram, &layout, name);
            let holder = frame(holder);
            assert!(
                holds(holder, placed.x, placed.y, placed.width, placed.height),
                "{placed:?} outside {holder:?}"
            );
            assert!(placed.y >= holder.y + style.package_tab(), "{placed:?}");
        }
    }

    #[test]
    fn a_frame_comes_back_no_narrower_than_the_name_in_its_tab() {
        let style = Style::default();
        let ws = resolved("package AVeryLongPackageNameIndeed { part def A; }\n");
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let layout = laid_out(&diagram, &style);

        // the one box it holds is far narrower than the name across its
        // top, which is measured as the bold it is drawn in
        let frame = &layout.packages[0];
        assert!(
            frame.width
                >= style.name_width("AVeryLongPackageNameIndeed")
                    + 2.0 * style.padding
                    + style.package_slant(),
            "{frame:?}"
        );
        // and the canvas holds the frame it was widened to
        assert!(layout.width >= frame.x + frame.width + style.margin);
    }

    #[test]
    fn a_box_outside_every_package_is_placed_too() {
        let style = Style::default();
        let ws = resolved("package P { part def A; }\npart def Loose;\n");
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let layout = laid_out(&diagram, &style);

        let loose = at(&diagram, &layout, "Loose");
        let frame = &layout.packages[0];
        assert!(loose.x >= style.margin && loose.y >= style.margin);
        // it is drawn beside the package, not inside it
        assert!(
            loose.x >= frame.x + frame.width || loose.y >= frame.y + frame.height,
            "{loose:?} within {frame:?}"
        );
    }

    #[test]
    fn a_route_a_package_holds_is_read_in_the_canvas_it_is_drawn_on() {
        let style = Style::default();
        let ws = resolved(NESTED);
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let layout = laid_out(&diagram, &style);

        // the composition crosses between the two sub-packages, so `Outer`
        // is the node that holds it -- and its bends are `Outer`'s own,
        // which have to be read back onto the whole canvas
        assert_eq!(diagram.edges.len(), 1);
        let route = &layout.routes[0];
        assert!(route.len() >= 2, "{route:?}");
        let outer = &layout.packages[0];
        for &(x, y) in route {
            assert!(x >= outer.x && x <= outer.x + outer.width, "{route:?}");
            assert!(y >= outer.y && y <= outer.y + outer.height, "{route:?}");
        }
    }

    #[test]
    fn the_bends_elk_routes_are_the_ones_drawn() {
        let style = Style::default();
        let diagram = diagram();
        let layout = laid_out(&diagram, &style);

        // the specialization is told to ELK the other way round from the
        // way it is drawn, so its route is read back from `from` to `to`:
        // it starts at the subtype and ends at the supertype
        let specialization = diagram
            .edges
            .iter()
            .position(|edge| edge.relation == Relation::Specialization)
            .unwrap();
        let route = &layout.routes[specialization];
        assert!(route.len() >= 2, "{route:?}");
        let edge = &diagram.edges[specialization];
        let (from, to) = (&layout.placed[edge.from], &layout.placed[edge.to]);
        let (start, finish) = (route[0], route[route.len() - 1]);
        assert!(start.1 >= from.y, "{route:?}");
        assert!(finish.1 <= to.y + to.height, "{route:?}");
        assert_eq!(layout.routes.len(), diagram.edges.len());
    }

    #[test]
    fn a_port_elk_routed_twice_keeps_one_square() {
        use crate::svg::anchor_tests::{diagram_of, edge_ends, leaves, squares, TWICE};

        // ELK routes each line to a border point of its own, so without
        // an anchor the second of them would set out beside the square
        // rather than from it
        let style = Style::default();
        let diagram = diagram_of(TWICE, "Sys");
        assert_eq!((diagram.nodes.len(), diagram.edges.len()), (3, 2));
        let laid_out = laid_out(&diagram, &style);
        let svg = crate::svg::to_svg(&diagram, &laid_out, &style);

        let squares = squares(&svg);
        assert_eq!(squares.len(), 3, "{svg}");
        let drawn = edge_ends(&svg);
        assert_eq!(drawn.len(), 2, "{svg}");
        for (start, finish) in drawn {
            assert_eq!(leaves(&squares, start, &style), 1, "{start:?} in {svg}");
            assert_eq!(leaves(&squares, finish, &style), 1, "{finish:?} in {svg}");
        }
    }

    #[test]
    fn the_same_model_lays_out_the_same_way_twice() {
        // the engine is a port of ELK's own algorithms and decides
        // nothing at random, so a drawing is reproducible byte for byte
        let diagram = diagram();
        let style = Style::default();
        assert_eq!(render(&diagram, &style), render(&diagram, &style));
        assert_eq!(laid_out(&diagram, &style), laid_out(&diagram, &style));
    }
}

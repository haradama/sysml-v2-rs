//! The Eclipse Layout Kernel as the layout engine: `elkrs` decides
//! where the boxes go, and everything visible -- boxes, compartments,
//! ports, edges, labels -- is still drawn by this crate's own renderer
//! in its own style. The positions and the routes both come from ELK:
//! it picks them together, and a line drawn straight across a layout
//! that was arranged expecting bends ends up where ELK left no room for
//! it. Where ELK routes nothing, the renderer routes for itself, as it
//! does under the built-in layout.
//!
//! Nothing here requires ELK at build time: the `elkrs` binary is
//! spawned at run time (`cargo install elkrs`), and a missing or
//! failing binary surfaces as an [`ElkError`] the caller can fall back
//! from.

use std::io::Write as _;
use std::process::{Command, Stdio};

use crate::graph::Relation;
use crate::layout::{box_size, enclosed, Frame, Layout, Placed};
use crate::{Diagram, Style};

/// Why ELK produced no layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ElkError {
    /// The command could not be run at all -- most likely `elkrs` is
    /// not installed.
    Spawn { command: String, error: String },
    /// The command ran and failed.
    Failed { command: String, detail: String },
    /// The command answered something an ELK graph never says.
    Unreadable { detail: String },
}

impl std::fmt::Display for ElkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ElkError::Spawn { command, error } => write!(
                f,
                "`{command}` could not be run ({error}); try `cargo install elkrs`"
            ),
            ElkError::Failed { command, detail } => write!(f, "`{command}` failed: {detail}"),
            ElkError::Unreadable { detail } => write!(f, "unreadable ELK output: {detail}"),
        }
    }
}

impl std::error::Error for ElkError {}

/// Lay `diagram` out by running `command` (`elkrs` or anything speaking
/// ELK's JSON) and reading the positions back.
pub fn elk_layout(diagram: &Diagram, style: &Style, command: &str) -> Result<Layout, ElkError> {
    // The layered algorithm's partitions run along the flow, and a
    // swimlane runs across it, so a view partitioned by performer is laid
    // out here rather than sent away to be arranged the wrong way round.
    if !diagram.lanes.is_empty() {
        return Ok(crate::layout(diagram, style));
    }
    let sizes: Vec<(f64, f64)> = diagram
        .nodes
        .iter()
        .map(|node| box_size(node, style))
        .collect();
    let laid_out = run(command, &to_elk(diagram, &sizes, style))?;
    let placed = parse_elk(&laid_out, diagram, &sizes, style)?;
    // ELK places boxes and knows nothing of the names written outside
    // them, so the room for those is left here, as the built-in layout
    // leaves it
    Ok(crate::svg::with_room_for_labels(diagram, &placed, style).unwrap_or(placed))
}

/// The diagram as an ELK graph: boxes at their measured sizes and the
/// relations between them. An edge points from the box that should sit
/// higher -- the supertype, the whole, a transition's source, the
/// requirement -- so the layered algorithm reproduces the reading order
/// the built-in layout has.
fn to_elk(diagram: &Diagram, sizes: &[(f64, f64)], style: &Style) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    out.push_str("{\"id\":\"root\",\"layoutOptions\":{");
    out.push_str("\"elk.algorithm\":\"layered\",\"elk.direction\":\"DOWN\"");
    write!(out, ",\"elk.spacing.nodeNode\":\"{:.4}\"", style.h_gap).unwrap();
    write!(
        out,
        ",\"elk.layered.spacing.nodeNodeBetweenLayers\":\"{:.4}\"",
        style.v_gap
    )
    .unwrap();
    // a package holds what it contains, so it goes to ELK as a node with
    // children and the engine arranges each package's contents inside it
    if !diagram.groups.is_empty() {
        out.push_str(",\"elk.hierarchyHandling\":\"INCLUDE_CHILDREN\"");
    }
    out.push_str("},\"children\":[");
    let owners = edge_owners(diagram);
    let mut written = 0;
    for at in 0..diagram.groups.len() {
        if diagram.groups[at].depth == 0 {
            separate(&mut out, &mut written);
            package(&mut out, diagram, sizes, style, &owners, at);
        }
    }
    for (at, (width, height)) in sizes.iter().enumerate() {
        if diagram.groups.iter().any(|group| group.nodes.contains(&at)) {
            continue;
        }
        separate(&mut out, &mut written);
        write!(
            out,
            "{{\"id\":\"n{at}\",\"width\":{width:.4},\"height\":{height:.4}}}"
        )
        .unwrap();
    }
    out.push(']');
    edges(&mut out, diagram, &owners, None);
    out.push('}');
    out
}

/// The edges an ELK node owns, as its `edges` array. ELK resolves an
/// edge's ends within the node it is written in, so each edge is written
/// in the innermost package that holds both of them.
fn edges(out: &mut String, diagram: &Diagram, owners: &[Option<usize>], within: Option<usize>) {
    use std::fmt::Write as _;

    out.push_str(",\"edges\":[");
    let mut written = 0;
    for (at, edge) in diagram.edges.iter().enumerate() {
        if owners[at] != within {
            continue;
        }
        let (tail, head) = if points_upward(edge.relation) {
            (edge.to, edge.from)
        } else {
            (edge.from, edge.to)
        };
        separate(out, &mut written);
        write!(
            out,
            "{{\"id\":\"e{at}\",\"sources\":[\"n{tail}\"],\"targets\":[\"n{head}\"]}}"
        )
        .unwrap();
    }
    out.push(']');
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

/// Whether the box an edge is drawn towards is the one that should sit
/// higher -- the supertype, the requirement -- so that ELK is told the
/// edge the other way round from the way it is drawn.
fn points_upward(relation: Relation) -> bool {
    matches!(relation, Relation::Specialization | Relation::Satisfy)
}

/// A comma before every element of a JSON array but the first.
fn separate(out: &mut String, written: &mut usize) {
    if *written > 0 {
        out.push(',');
    }
    *written += 1;
}

/// One package as an ELK node: what it owns as children, and the packages
/// it encloses nested inside it. The padding leaves room at the top for
/// the tab the standard draws the package's name in.
fn package(
    out: &mut String,
    diagram: &Diagram,
    sizes: &[(f64, f64)],
    style: &Style,
    owners: &[Option<usize>],
    at: usize,
) {
    use std::fmt::Write as _;

    let pad = style.padding;
    // the tab and then the same clearance the other three sides get:
    // padding of exactly the tab would leave the first box's top border
    // drawn along the tab's bottom, which reads as one line, not two
    let top = 2.0 * style.padding + style.line_height + pad;
    write!(
        out,
        "{{\"id\":\"g{at}\",\"layoutOptions\":{{\"elk.algorithm\":\"layered\",\
         \"elk.direction\":\"DOWN\",\"elk.padding\":\"[top={top:.4},left={pad:.4},\
         bottom={pad:.4},right={pad:.4}]\"}},\"children\":["
    )
    .unwrap();
    let mut written = 0;
    for &node in &diagram.groups[at].nodes {
        separate(out, &mut written);
        let (width, height) = sizes[node];
        write!(
            out,
            "{{\"id\":\"n{node}\",\"width\":{width:.4},\"height\":{height:.4}}}"
        )
        .unwrap();
    }
    for below in enclosed(diagram, at) {
        separate(out, &mut written);
        package(out, diagram, sizes, style, owners, below);
    }
    out.push(']');
    edges(out, diagram, owners, Some(at));
    out.push('}');
}

/// Feed `source` to `command -` and return what it wrote.
fn run(command: &str, source: &str) -> Result<String, ElkError> {
    let mut child = Command::new(command)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| ElkError::Spawn {
            command: command.to_string(),
            error: error.to_string(),
        })?;
    // the command may exit before draining stdin; its exit status and
    // output are the signal, not the pipe
    let _ = child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(source.as_bytes());
    let output = child
        .wait_with_output()
        .expect("a spawned child can be waited on");
    if !output.status.success() {
        return Err(ElkError::Failed {
            command: command.to_string(),
            detail: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    String::from_utf8(output.stdout).map_err(|_| ElkError::Unreadable {
        detail: "not UTF-8".to_string(),
    })
}

/// Positions out of a laid-out ELK graph: each child carries the corner
/// it was placed at, and the root the canvas they all fit in.
fn parse_elk(
    laid_out: &str,
    diagram: &Diagram,
    sizes: &[(f64, f64)],
    style: &Style,
) -> Result<Layout, ElkError> {
    let unreadable = |detail: String| ElkError::Unreadable { detail };
    let graph: serde_json::Value =
        serde_json::from_str(laid_out).map_err(|why| unreadable(why.to_string()))?;

    let number = |of: &serde_json::Value, name: &str| -> Result<f64, ElkError> {
        of.get(name)
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| unreadable(format!("expected a number for `{name}`")))
    };

    let mut placed: Vec<Option<Placed>> = vec![None; sizes.len()];
    let mut packages: Vec<Option<Frame>> = vec![None; diagram.groups.len()];
    read_children(
        &graph,
        (style.margin, style.margin),
        diagram,
        sizes,
        &mut placed,
        &mut packages,
    )?;

    // a frame ELK left out would leave the drawing unable to say where a
    // package is, which is a refusal rather than a package quietly missing
    let mut packages = packages
        .into_iter()
        .enumerate()
        .map(|(at, slot)| slot.ok_or_else(|| unreadable(format!("no position for package g{at}"))))
        .collect::<Result<Vec<Frame>, ElkError>>()?;
    // The tab is drawn here and not by the engine, which sizes a package
    // by what it holds: one small box under a long name comes back as a
    // frame narrower than the name written across the top of it. Asking
    // the engine for a minimum size instead is worth less than it looks:
    // `elkrs` applies one to a nested node along its own axis, so a frame
    // told to be wide comes back tall.
    for frame in &mut packages {
        frame.width = frame
            .width
            .max(style.text_width(&frame.name) + 2.0 * style.padding);
    }
    let placed = placed
        .into_iter()
        .enumerate()
        .map(|(at, slot)| slot.ok_or_else(|| unreadable(format!("no position for node n{at}"))))
        .collect::<Result<Vec<Placed>, ElkError>>()?;
    let width = packages.iter().fold(
        number(&graph, "width")? + 2.0 * style.margin,
        |canvas: f64, frame| canvas.max(frame.x + frame.width + style.margin),
    );
    Ok(Layout {
        placed,
        width,
        height: number(&graph, "height")? + 2.0 * style.margin,
        routes: routes_of(&graph, diagram, style),
        // a laned view never reaches here; it is laid out by this crate
        lanes: Vec::new(),
        packages,
    })
}

/// Read one level of a laid-out graph and the levels below it. ELK places
/// a child within its parent, so a package's corner is carried down and
/// added to what it contains, which leaves every position on the canvas.
fn read_children(
    of: &serde_json::Value,
    (left, top): (f64, f64),
    diagram: &Diagram,
    sizes: &[(f64, f64)],
    placed: &mut [Option<Placed>],
    packages: &mut [Option<Frame>],
) -> Result<(), ElkError> {
    let unreadable = |detail: String| ElkError::Unreadable { detail };
    let number = |of: &serde_json::Value, name: &str| -> Result<f64, ElkError> {
        of.get(name)
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| unreadable(format!("expected a number for `{name}`")))
    };

    let children = of
        .get("children")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| unreadable("no children".to_string()))?;

    for child in children {
        let id = child
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if let Some(at) = index(id, 'g', diagram.groups.len()) {
            let (x, y) = (left + number(child, "x")?, top + number(child, "y")?);
            packages[at] = Some(Frame {
                name: diagram.groups[at].name.clone(),
                x,
                y,
                width: number(child, "width")?,
                height: number(child, "height")?,
            });
            read_children(child, (x, y), diagram, sizes, placed, packages)?;
            continue;
        }
        let at = index(id, 'n', sizes.len())
            .ok_or_else(|| unreadable(format!("unknown node `{id}`")))?;
        let (x, y) = (left + number(child, "x")?, top + number(child, "y")?);
        // the sizes are this crate's own; ELK only echoes them
        let (width, height) = sizes[at];
        placed[at] = Some(Placed {
            node: at,
            x,
            y,
            width,
            height,
        });
    }
    Ok(())
}

/// The number an ELK id names, if it is one of ours and within range.
fn index(id: &str, prefix: char, count: usize) -> Option<usize> {
    id.strip_prefix(prefix)?
        .parse()
        .ok()
        .filter(|&at| at < count)
}

/// The path ELK chose for each edge, in the diagram's own order and
/// running from the edge's `from` to its `to`.
///
/// ELK routes as it places -- orthogonally, around whatever is in the
/// way -- and the drawing follows it rather than cutting its own line
/// through a layout that was arranged expecting the bends. An edge ELK
/// said nothing about keeps an empty route, and the renderer routes it.
fn routes_of(graph: &serde_json::Value, diagram: &Diagram, style: &Style) -> Vec<Vec<(f64, f64)>> {
    let mut routes = vec![Vec::new(); diagram.edges.len()];
    read_routes(graph, (style.margin, style.margin), diagram, &mut routes);
    routes
}

/// Read the routes one node holds and those its children hold. An edge is
/// written in the node that can see both of its ends, so a package holds
/// the routes between the things inside it -- in its own coordinates,
/// which is why the corner it was placed at is carried down.
fn read_routes(
    of: &serde_json::Value,
    (left, top): (f64, f64),
    diagram: &Diagram,
    routes: &mut [Vec<(f64, f64)>],
) {
    let point = |at: &serde_json::Value| -> Option<(f64, f64)> {
        Some((left + at.get("x")?.as_f64()?, top + at.get("y")?.as_f64()?))
    };
    for child in of
        .get("children")
        .and_then(serde_json::Value::as_array)
        .unwrap_or(&Vec::new())
    {
        // where a child was placed has already been read and refused if
        // it was missing, so a corner that is not a number cannot be one
        let corner = |name: &str| {
            child
                .get(name)
                .and_then(serde_json::Value::as_f64)
                .unwrap_or_default()
        };
        read_routes(
            child,
            (left + corner("x"), top + corner("y")),
            diagram,
            routes,
        );
    }
    for edge in of
        .get("edges")
        .and_then(serde_json::Value::as_array)
        .unwrap_or(&Vec::new())
    {
        let Some(at) = edge
            .get("id")
            .and_then(serde_json::Value::as_str)
            .and_then(|id| id.strip_prefix('e'))
            .and_then(|digits| digits.parse::<usize>().ok())
            .filter(|&at| at < routes.len())
        else {
            continue;
        };
        let Some(section) = edge
            .get("sections")
            .and_then(serde_json::Value::as_array)
            .and_then(|sections| sections.first())
        else {
            continue;
        };
        let mut walked = Vec::new();
        walked.extend(section.get("startPoint").and_then(&point));
        for bend in section
            .get("bendPoints")
            .and_then(serde_json::Value::as_array)
            .unwrap_or(&Vec::new())
        {
            walked.extend(point(bend));
        }
        walked.extend(section.get("endPoint").and_then(&point));
        if walked.len() < 2 {
            continue;
        }
        // the emission points a hierarchical edge the way it should be
        // read, which is the opposite of the way it is drawn
        if points_upward(diagram.edges[at].relation) {
            walked.reverse();
        }
        routes[at] = walked;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::resolved;
    use crate::{definition_diagram, render_with_elk};

    /// A model with every relation kind the ELK emission distinguishes.
    const SOURCE: &str = "part def A;\n\
                          part def B :> A;\n\
                          requirement def R;\n\
                          satisfy R by B;\n";

    fn diagram() -> Diagram {
        let ws = resolved(SOURCE);
        definition_diagram(ws.model(), &[ws.root()])
    }

    /// Held while a test writes a stand-in for `elkrs` and runs it.
    ///
    /// Writing a program and then running it is a race when anything
    /// else in the process forks in between: the child inherits the
    /// still-open write handle, and Linux refuses to run a file that
    /// something holds open for writing. Two tests here write and run
    /// their own `elkrs`, and either one's fork can spoil the other's
    /// exec, which showed up as one or the other failing every few runs.
    /// Taking turns is enough -- nothing else in this process forks.
    static SPAWNING: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// An executable stand-in for `elkrs`, unique per test.
    fn fake_elk(name: &str, script: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path =
            std::env::temp_dir().join(format!("sysml-fake-elk-{name}-{}", std::process::id()));
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn the_elk_graph_points_the_way_it_reads() {
        let ws = resolved(
            "part def A;\npart def B :> A;\npart def W { part b : B; }\n\
             state def M { state x; state y; transition go first x then y; }\n\
             part def P { port p : A; }\npart def Q { port q : A; }\n\
             part def Pair { part l : P; part r : Q; connect l.p to r.q; }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let style = Style::default();
        let sizes: Vec<(f64, f64)> = diagram
            .nodes
            .iter()
            .map(|node| box_size(node, &style))
            .collect();
        let graph = to_elk(&diagram, &sizes, &style);

        // what ELK is told is a graph it can lay out, at the sizes this
        // crate measured and the spacing this crate's style asks for
        let parsed: serde_json::Value = serde_json::from_str(&graph).expect("valid ELK JSON");
        assert_eq!(parsed["layoutOptions"]["elk.algorithm"], "layered");
        assert_eq!(
            parsed["layoutOptions"]["elk.spacing.nodeNode"],
            format!("{:.4}", style.h_gap)
        );
        assert_eq!(
            parsed["children"].as_array().unwrap().len(),
            diagram.nodes.len()
        );
        assert_eq!(parsed["children"][0]["width"], sizes[0].0);

        let index = |name: &str| {
            diagram
                .nodes
                .iter()
                .position(|node| node.name == name)
                .unwrap()
        };
        let points = |from: usize, to: usize| {
            parsed["edges"].as_array().unwrap().iter().any(|edge| {
                edge["sources"][0] == format!("n{from}") && edge["targets"][0] == format!("n{to}")
            })
        };
        // the supertype is the source so it lands above its subtype
        assert!(points(index("A"), index("B")), "{graph}");
        // the whole above its part
        assert!(points(index("W"), index("B")), "{graph}");

        // a connection -- drawn in the interconnection view -- is laid
        // out like any other edge: ELK arranges a graph, not a tree
        let model = ws.model();
        let pair = model
            .descendants(ws.root())
            .into_iter()
            .find(|&id| model.name(id) == Some("Pair"))
            .unwrap();
        let inside = crate::interconnection_diagram(model, pair);
        let sizes: Vec<(f64, f64)> = inside
            .nodes
            .iter()
            .map(|node| box_size(node, &style))
            .collect();
        let wired: serde_json::Value =
            serde_json::from_str(&to_elk(&inside, &sizes, &style)).unwrap();
        assert_eq!(wired["edges"].as_array().unwrap().len(), inside.edges.len());
    }

    /// A package holding a sub-package that holds the definitions.
    const NESTED: &str = "package Outer {\n\
                          package Parts { part def Engine; }\n\
                          package Wholes { part def Car { part e : Parts::Engine; } }\n\
                          }\n";

    #[test]
    fn a_package_is_a_node_that_holds_what_it_contains() {
        let ws = resolved(NESTED);
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let names: Vec<&str> = diagram.groups.iter().map(|g| g.name.as_str()).collect();
        // `Outer` owns no definition of its own and is a frame all the same
        assert_eq!(names, ["Outer", "Parts", "Wholes"]);
        assert_eq!(diagram.groups[0].nodes, Vec::<usize>::new());

        let sizes = vec![(100.0, 40.0); diagram.nodes.len()];
        let graph: serde_json::Value =
            serde_json::from_str(&to_elk(&diagram, &sizes, &Style::default())).unwrap();
        assert_eq!(
            graph["layoutOptions"]["elk.hierarchyHandling"],
            "INCLUDE_CHILDREN"
        );

        // one node at the top, holding the two packages below it
        let outer = &graph["children"][0];
        assert_eq!(outer["id"], "g0");
        assert!(graph["children"][1].is_null());
        let within: Vec<&str> = outer["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|child| child["id"].as_str().unwrap())
            .collect();
        assert_eq!(within, ["g1", "g2"]);

        // the composition crosses from `Wholes` into `Parts`, so only
        // `Outer` can see both of its ends and it is written there
        assert!(graph["edges"].as_array().unwrap().is_empty());
        assert_eq!(outer["edges"][0]["sources"][0], "n1");
        assert_eq!(outer["edges"][0]["targets"][0], "n0");
        assert!(outer["children"][0]["edges"][0].is_null());
    }

    #[test]
    fn a_package_leaves_room_under_its_tab() {
        let style = Style::default();
        let ws = resolved(NESTED);
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let sizes = vec![(100.0, 40.0); diagram.nodes.len()];
        let graph: serde_json::Value =
            serde_json::from_str(&to_elk(&diagram, &sizes, &style)).unwrap();

        // the drawing puts the package's name in a tab inside the frame,
        // so what the frame holds has to start below it and then clear of
        // it by as much as the other three sides are cleared by
        let tab = 2.0 * style.padding + style.line_height;
        let padding = graph["children"][0]["layoutOptions"]["elk.padding"]
            .as_str()
            .expect("a package says how much room it keeps");
        let top: f64 = padding
            .trim_start_matches("[top=")
            .split(',')
            .next()
            .and_then(|room| room.parse().ok())
            .unwrap();
        assert!(top >= tab + style.padding, "{padding}");
    }

    #[test]
    fn a_frame_comes_back_no_narrower_than_the_name_in_its_tab() {
        let style = Style::default();
        let ws = resolved("package AVeryLongPackageNameIndeed { part def A; }\n");
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let sizes = vec![(60.0, 40.0); diagram.nodes.len()];
        // an engine that sized the frame by the one box it holds
        let layout = parse_elk(
            "{\"width\":80,\"height\":100,\"children\":[{\"id\":\"g0\",\"x\":0,\"y\":0,\
             \"width\":80,\"height\":100,\"children\":[{\"id\":\"n0\",\"x\":10,\"y\":47}]}]}",
            &diagram,
            &sizes,
            &style,
        )
        .unwrap();
        let frame = &layout.packages[0];
        assert!(
            frame.width >= style.text_width("AVeryLongPackageNameIndeed") + 2.0 * style.padding,
            "{frame:?}"
        );
        // and the canvas holds the frame it was widened to
        assert!(layout.width >= frame.x + frame.width + style.margin);
    }

    #[test]
    fn a_box_outside_every_package_still_goes_to_elk() {
        let ws = resolved("package P { part def A; }\npart def Loose;\n");
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let sizes = vec![(100.0, 40.0); diagram.nodes.len()];
        let graph: serde_json::Value =
            serde_json::from_str(&to_elk(&diagram, &sizes, &Style::default())).unwrap();
        let top: Vec<&str> = graph["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|child| child["id"].as_str().unwrap())
            .collect();
        assert_eq!(top, ["g0", "n1"]);
    }

    #[test]
    fn a_package_elk_placed_becomes_the_frame_drawn_round_it() {
        let style = Style::default();
        let ws = resolved(NESTED);
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let sizes = [(100.0, 40.0), (100.0, 40.0)];
        // ELK places a child within its parent, so the corners add up
        let laid_out = "{\"id\":\"root\",\"width\":400,\"height\":300,\"children\":[\
                        {\"id\":\"g0\",\"x\":10,\"y\":20,\"width\":380,\"height\":270,\
                        \"children\":[\
                        {\"id\":\"g1\",\"x\":5,\"y\":30,\"width\":120,\"height\":80,\
                        \"children\":[{\"id\":\"n0\",\"x\":6,\"y\":40}]},\
                        {\"id\":\"g2\",\"x\":5,\"y\":150,\"width\":120,\"height\":80,\
                        \"children\":[{\"id\":\"n1\",\"x\":6,\"y\":40}]}]}]}";
        let layout = parse_elk(laid_out, &diagram, &sizes, &style).unwrap();

        let frames: Vec<(&str, f64, f64)> = layout
            .packages
            .iter()
            .map(|frame| (frame.name.as_str(), frame.x, frame.y))
            .collect();
        assert_eq!(
            frames,
            [
                ("Outer", style.margin + 10.0, style.margin + 20.0),
                ("Parts", style.margin + 15.0, style.margin + 50.0),
                ("Wholes", style.margin + 15.0, style.margin + 170.0),
            ]
        );
        assert_eq!(layout.packages[0].width, 380.0);
        assert_eq!(layout.placed[0].x, style.margin + 21.0);
        assert_eq!(layout.placed[0].y, style.margin + 90.0);
    }

    #[test]
    fn a_route_a_package_holds_is_read_in_the_canvas_it_is_drawn_on() {
        let style = Style::default();
        let ws = resolved(NESTED);
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let sizes = [(100.0, 40.0), (100.0, 40.0)];
        // the composition crosses between the two sub-packages, so `Outer`
        // is the node that holds it -- and its bends are `Outer`'s own
        let laid_out = "{\"id\":\"root\",\"width\":400,\"height\":300,\"children\":[\
                        {\"id\":\"g0\",\"x\":10,\"y\":20,\"width\":380,\"height\":270,\
                        \"children\":[\
                        {\"id\":\"g1\",\"x\":5,\"y\":30,\"width\":120,\"height\":80,\
                        \"children\":[{\"id\":\"n0\",\"x\":6,\"y\":40}]},\
                        {\"id\":\"g2\",\"x\":5,\"y\":150,\"width\":120,\"height\":80,\
                        \"children\":[{\"id\":\"n1\",\"x\":6,\"y\":40}]}],\
                        \"edges\":[{\"id\":\"e0\",\"sections\":[{\
                        \"startPoint\":{\"x\":50,\"y\":230},\
                        \"bendPoints\":[{\"x\":50,\"y\":140}],\
                        \"endPoint\":{\"x\":60,\"y\":110}}]}]}]}";
        let layout = parse_elk(laid_out, &diagram, &sizes, &style).unwrap();

        // every point is `Outer`'s corner plus the margin plus its own
        let corner = style.margin + 10.0;
        assert_eq!(
            layout.routes[0],
            [
                (corner + 50.0, style.margin + 20.0 + 230.0),
                (corner + 50.0, style.margin + 20.0 + 140.0),
                (corner + 60.0, style.margin + 20.0 + 110.0),
            ]
        );
    }

    #[test]
    fn a_swimlane_view_is_laid_out_here_rather_than_by_elk() {
        let ws = resolved(
            "action providePower {\n\
             \taction generate;\n\
             \taction convert;\n\
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
        assert!(!diagram.lanes.is_empty());
        // no ELK to run: it is never reached, so the name cannot matter
        let layout = elk_layout(&diagram, &Style::default(), "elk-that-is-not-there").unwrap();
        assert_eq!(layout.lanes.len(), diagram.lanes.len());
    }

    #[test]
    fn a_laid_out_graph_maps_back_to_pixels() {
        let style = Style::default();
        let diagram = diagram();
        let sizes = [(120.0, 40.0), (150.0, 60.0)];
        let laid_out = "{\"id\":\"root\",\"width\":400,\"height\":200,\"children\":[\
                        {\"id\":\"n0\",\"x\":12,\"y\":0,\"width\":120,\"height\":40},\
                        {\"id\":\"n1\",\"x\":0,\"y\":96,\"width\":150,\"height\":60}]}";
        let layout = parse_elk(laid_out, &diagram, &sizes, &style).unwrap();

        // ELK places a box by its top-left corner already; only the
        // margin this crate draws around everything is added
        assert_eq!(layout.placed[0].x, style.margin + 12.0);
        assert_eq!(layout.placed[0].y, style.margin);
        assert_eq!(layout.placed[1].y, style.margin + 96.0);
        // the sizes stay this crate's own, whatever ELK echoes
        assert_eq!(layout.placed[1].width, 150.0);
        assert_eq!(layout.width, 400.0 + 2.0 * style.margin);
        assert_eq!(layout.height, 200.0 + 2.0 * style.margin);
    }

    #[test]
    fn what_an_elk_graph_never_says_is_refused() {
        let style = Style::default();
        let sizes = [(10.0, 10.0)];
        // every refusal is `Unreadable`; its Display carries the detail
        let diagram = diagram();
        let unreadable = |laid_out: &str| {
            parse_elk(laid_out, &diagram, &sizes, &style)
                .expect_err("output that never parses")
                .to_string()
                .replace("unreadable ELK output: ", "")
        };

        assert!(unreadable("pancakes").contains("expected value"));
        assert_eq!(unreadable("{}"), "no children");
        assert!(unreadable("{\"children\":[{\"id\":\"n7\"}]}").contains("unknown node `n7`"));
        assert!(unreadable("{\"children\":[{\"id\":\"wat\"}]}").contains("unknown node `wat`"));
        assert!(unreadable("{\"children\":[{}]}").contains("unknown node ``"));
        assert!(unreadable("{\"children\":[{\"id\":\"n0\",\"y\":0}]}").contains("a number for `x`"));
        assert_eq!(unreadable("{\"children\":[]}"), "no position for node n0");
        // and a package ELK never placed is refused the same way
        let ws = resolved(NESTED);
        let nested = definition_diagram(ws.model(), &[ws.root()]);
        assert_eq!(
            parse_elk("{\"children\":[]}", &nested, &[(1.0, 1.0); 2], &style)
                .expect_err("a graph that placed nothing")
                .to_string()
                .replace("unreadable ELK output: ", ""),
            "no position for package g0"
        );
        assert!(
            unreadable("{\"children\":[{\"id\":\"n0\",\"x\":0,\"y\":0}],\"height\":1}")
                .contains("a number for `width`")
        );
    }

    #[test]
    fn a_fake_elk_lays_the_whole_diagram_out() {
        let _turn = SPAWNING.lock().unwrap_or_else(|e| e.into_inner());
        let diagram = diagram();
        assert_eq!(diagram.nodes.len(), 3);
        let script = fake_elk(
            "ok",
            "cat >/dev/null\n\
             printf '{\"width\":600,\"height\":400,\"children\":[\
{\"id\":\"n0\",\"x\":0,\"y\":0},{\"id\":\"n1\",\"x\":0,\"y\":200},\
{\"id\":\"n2\",\"x\":300,\"y\":200}]}\\n'",
        );
        let svg = render_with_elk(&diagram, &Style::default(), script.to_str().unwrap()).unwrap();
        std::fs::remove_file(&script).ok();

        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(svg.contains(">A<") && svg.contains(">B<") && svg.contains(">R<"));
    }

    #[test]
    fn the_bends_elk_routes_are_the_ones_drawn() {
        let style = Style::default();
        let diagram = diagram();
        let sizes = [(120.0, 40.0), (150.0, 60.0), (100.0, 40.0)];
        // e0 is the specialization, told to ELK the other way round from
        // the way it is drawn, so its route comes back reversed
        let laid_out = "{\"width\":400,\"height\":200,\"children\":[\
             {\"id\":\"n0\",\"x\":0,\"y\":0},{\"id\":\"n1\",\"x\":0,\"y\":100},\
             {\"id\":\"n2\",\"x\":200,\"y\":100}],\"edges\":[\
             {\"id\":\"e0\",\"sections\":[{\"startPoint\":{\"x\":10,\"y\":40},\
             \"bendPoints\":[{\"x\":10,\"y\":70},{\"x\":60,\"y\":70}],\
             \"endPoint\":{\"x\":60,\"y\":100}}]},\
             {\"id\":\"e9\",\"sections\":[{\"startPoint\":{\"x\":0,\"y\":0},\
             \"endPoint\":{\"x\":1,\"y\":1}}]}]}";
        let layout = parse_elk(laid_out, &diagram, &sizes, &style).unwrap();

        let m = style.margin;
        assert_eq!(
            layout.routes[0],
            [
                (60.0 + m, 100.0 + m),
                (60.0 + m, 70.0 + m),
                (10.0 + m, 70.0 + m),
                (10.0 + m, 40.0 + m)
            ]
        );
        // an edge ELK said nothing about is left to the renderer, and an
        // id for an edge the diagram does not have is not one of ours
        assert!(layout.routes[1..].iter().all(Vec::is_empty));
        assert_eq!(layout.routes.len(), diagram.edges.len());
    }

    #[test]
    fn a_route_of_one_point_is_no_route() {
        // three ways for an edge to say nothing: a section with one
        // end, no sections at all, and an id for an edge this diagram
        // does not have
        let style = Style::default();
        let diagram = diagram();
        let sizes = [(1.0, 1.0), (1.0, 1.0), (1.0, 1.0)];
        let layout = parse_elk(
            "{\"width\":9,\"height\":9,\"children\":[\
             {\"id\":\"n0\",\"x\":0,\"y\":0},{\"id\":\"n1\",\"x\":0,\"y\":1},\
             {\"id\":\"n2\",\"x\":1,\"y\":1}],\"edges\":[\
             {\"id\":\"e0\",\"sections\":[{\"endPoint\":{\"x\":1,\"y\":1}}]},\
             {\"id\":\"e0\"},{\"id\":\"e9\"},{\"id\":\"nope\"}]}",
            &diagram,
            &sizes,
            &style,
        )
        .unwrap();
        assert!(
            layout.routes.iter().all(Vec::is_empty),
            "{:?}",
            layout.routes
        );
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
        let sizes = [(120.0, 40.0); 3];
        let laid_out = "{\"width\":400,\"height\":300,\"children\":[\
             {\"id\":\"n0\",\"x\":0,\"y\":0},{\"id\":\"n1\",\"x\":0,\"y\":200},\
             {\"id\":\"n2\",\"x\":200,\"y\":200}],\"edges\":[\
             {\"id\":\"e0\",\"sections\":[{\"startPoint\":{\"x\":60,\"y\":40},\
             \"bendPoints\":[{\"x\":60,\"y\":120}],\"endPoint\":{\"x\":60,\"y\":200}}]},\
             {\"id\":\"e1\",\"sections\":[{\"startPoint\":{\"x\":20,\"y\":40},\
             \"bendPoints\":[{\"x\":20,\"y\":140},{\"x\":260,\"y\":140}],\
             \"endPoint\":{\"x\":260,\"y\":200}}]}]}";
        let laid_out = parse_elk(laid_out, &diagram, &sizes, &style).unwrap();
        let svg = crate::svg::to_svg(&diagram, &laid_out, &style);

        let squares = squares(&svg);
        assert_eq!(squares.len(), 3, "{svg}");
        let drawn = edge_ends(&svg);
        assert_eq!(drawn.len(), 2, "{svg}");
        for (start, finish) in drawn {
            assert_eq!(leaves(&squares, start, &style), 1, "{start:?} in {svg}");
            assert_eq!(leaves(&squares, finish, &style), 1, "{finish:?} in {svg}");
        }
        // the square is the one ELK put the first route on
        assert!(
            squares.contains(&(60.0 + style.margin, 40.0 + style.margin)),
            "{squares:?}"
        );
    }

    #[test]
    fn a_missing_or_broken_elk_is_reported_not_papered_over() {
        let _turn = SPAWNING.lock().unwrap_or_else(|e| e.into_inner());
        let diagram = diagram();
        let style = Style::default();

        let missing = elk_layout(&diagram, &style, "/nonexistent/elk/elkrs");
        assert!(matches!(missing, Err(ElkError::Spawn { .. })));
        assert!(missing
            .unwrap_err()
            .to_string()
            .contains("cargo install elkrs"));

        let angry = fake_elk("angry", "echo 'boom' >&2\nexit 3");
        let failed = elk_layout(&diagram, &style, angry.to_str().unwrap());
        std::fs::remove_file(&angry).ok();
        let failed = failed.unwrap_err();
        assert!(matches!(&failed, ElkError::Failed { detail, .. } if detail == "boom"));
        assert!(failed.to_string().contains("failed: boom"));

        let mute = fake_elk("mute", "cat >/dev/null\necho pancakes");
        let unreadable = elk_layout(&diagram, &style, mute.to_str().unwrap());
        std::fs::remove_file(&mute).ok();
        assert!(matches!(unreadable, Err(ElkError::Unreadable { .. })));

        let binary = fake_elk("binary", "cat >/dev/null\nprintf '\\377\\376'");
        let not_text = elk_layout(&diagram, &style, binary.to_str().unwrap());
        std::fs::remove_file(&binary).ok();
        assert_eq!(
            not_text.unwrap_err().to_string(),
            "unreadable ELK output: not UTF-8"
        );
    }
}

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
use crate::layout::{box_size, Layout, Placed};
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
    let sizes: Vec<(f64, f64)> = diagram
        .nodes
        .iter()
        .map(|node| box_size(node, style))
        .collect();
    let laid_out = run(command, &to_elk(diagram, &sizes, style))?;
    parse_elk(&laid_out, diagram, &sizes, style)
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
    out.push_str("},\"children\":[");
    for (at, (width, height)) in sizes.iter().enumerate() {
        if at > 0 {
            out.push(',');
        }
        write!(
            out,
            "{{\"id\":\"n{at}\",\"width\":{width:.4},\"height\":{height:.4}}}"
        )
        .unwrap();
    }
    out.push_str("],\"edges\":[");
    for (at, edge) in diagram.edges.iter().enumerate() {
        let (tail, head) = if points_upward(edge.relation) {
            (edge.to, edge.from)
        } else {
            (edge.from, edge.to)
        };
        if at > 0 {
            out.push(',');
        }
        write!(
            out,
            "{{\"id\":\"e{at}\",\"sources\":[\"n{tail}\"],\"targets\":[\"n{head}\"]}}"
        )
        .unwrap();
    }
    out.push_str("]}");
    out
}

/// Whether the box an edge is drawn towards is the one that should sit
/// higher -- the supertype, the requirement -- so that ELK is told the
/// edge the other way round from the way it is drawn.
fn points_upward(relation: Relation) -> bool {
    matches!(relation, Relation::Specialization | Relation::Satisfy)
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

    let children = graph
        .get("children")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| unreadable("no children".to_string()))?;

    let mut placed: Vec<Option<Placed>> = vec![None; sizes.len()];
    for child in children {
        let id = child
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let at: usize = id
            .strip_prefix('n')
            .and_then(|digits| digits.parse().ok())
            .filter(|&at| at < sizes.len())
            .ok_or_else(|| unreadable(format!("unknown node `{id}`")))?;
        // the sizes are this crate's own; ELK only echoes them
        let (width, height) = sizes[at];
        placed[at] = Some(Placed {
            node: at,
            x: style.margin + number(child, "x")?,
            y: style.margin + number(child, "y")?,
            width,
            height,
        });
    }

    let placed = placed
        .into_iter()
        .enumerate()
        .map(|(at, slot)| slot.ok_or_else(|| unreadable(format!("no position for node n{at}"))))
        .collect::<Result<Vec<Placed>, ElkError>>()?;
    Ok(Layout {
        placed,
        width: number(&graph, "width")? + 2.0 * style.margin,
        height: number(&graph, "height")? + 2.0 * style.margin,
        routes: routes_of(&graph, diagram, style),
        // the engine arranged the boxes; the swimlane bands are ours
        lanes: Vec::new(),
        packages: Vec::new(),
    })
}

/// The path ELK chose for each edge, in the diagram's own order and
/// running from the edge's `from` to its `to`.
///
/// ELK routes as it places -- orthogonally, around whatever is in the
/// way -- and the drawing follows it rather than cutting its own line
/// through a layout that was arranged expecting the bends. An edge ELK
/// said nothing about keeps an empty route, and the renderer routes it.
fn routes_of(graph: &serde_json::Value, diagram: &Diagram, style: &Style) -> Vec<Vec<(f64, f64)>> {
    let point = |at: &serde_json::Value| -> Option<(f64, f64)> {
        Some((
            style.margin + at.get("x")?.as_f64()?,
            style.margin + at.get("y")?.as_f64()?,
        ))
    };
    let mut routes = vec![Vec::new(); diagram.edges.len()];
    for edge in graph
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
    routes
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

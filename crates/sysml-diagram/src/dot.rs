//! Graphviz as the layout engine, PlantUML-style: `dot` decides where
//! the boxes go, and everything visible -- boxes, compartments, ports,
//! edges, labels -- is still drawn by this crate's own renderer in its
//! own style. Only positions are read back (`dot -Tplain`); the splines
//! Graphviz routes are ignored, so a diagram keeps the same visual
//! language whichever engine laid it out.
//!
//! Nothing here requires Graphviz at build time: the `dot` binary is
//! spawned at run time, and a missing or failing binary surfaces as a
//! [`GraphvizError`] the caller can fall back from.

use std::io::Write as _;
use std::process::{Command, Stdio};

use crate::graph::Relation;
use crate::layout::{box_size, Layout, Placed};
use crate::{Diagram, Style};

/// Points per inch: `dot -Tplain` speaks inches, the SVG pixels.
const DPI: f64 = 72.0;

/// Why Graphviz produced no layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphvizError {
    /// The command could not be run at all -- most likely Graphviz is
    /// not installed.
    Spawn { command: String, error: String },
    /// The command ran and failed.
    Failed { command: String, detail: String },
    /// The command answered something `-Tplain` never says.
    Unreadable { detail: String },
}

impl std::fmt::Display for GraphvizError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphvizError::Spawn { command, error } => {
                write!(
                    f,
                    "`{command}` could not be run ({error}); is Graphviz installed?"
                )
            }
            GraphvizError::Failed { command, detail } => write!(f, "`{command}` failed: {detail}"),
            GraphvizError::Unreadable { detail } => {
                write!(f, "unreadable `dot -Tplain` output: {detail}")
            }
        }
    }
}

impl std::error::Error for GraphvizError {}

/// Lay `diagram` out by running `command` (Graphviz `dot` or anything
/// speaking its language) and reading the positions back.
pub fn graphviz_layout(
    diagram: &Diagram,
    style: &Style,
    command: &str,
) -> Result<Layout, GraphvizError> {
    let sizes: Vec<(f64, f64)> = diagram
        .nodes
        .iter()
        .map(|node| box_size(node, style))
        .collect();
    let plain = run(command, &to_dot(diagram, &sizes, style))?;
    parse_plain(&plain, &sizes, style)
}

/// The diagram as DOT source: boxes at their measured sizes and the
/// relations between them. Ranked edges point from the box that should
/// sit higher -- the supertype, the whole, a transition's source, the
/// requirement -- so `dot` reproduces the reading order the built-in
/// layout has; a connection is a peer-to-peer wire and constrains no
/// rank.
fn to_dot(diagram: &Diagram, sizes: &[(f64, f64)], style: &Style) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    writeln!(out, "digraph sysml {{").unwrap();
    writeln!(
        out,
        "  nodesep={:.4}; ranksep={:.4};",
        style.h_gap / DPI,
        style.v_gap / DPI
    )
    .unwrap();
    writeln!(out, "  node [shape=box fixedsize=true label=\"\"];").unwrap();
    for (at, (width, height)) in sizes.iter().enumerate() {
        writeln!(
            out,
            "  n{at} [width={:.4} height={:.4}];",
            width / DPI,
            height / DPI
        )
        .unwrap();
    }
    for edge in &diagram.edges {
        let (tail, head, attributes) = match edge.relation {
            Relation::Specialization | Relation::Satisfy => (edge.to, edge.from, ""),
            Relation::Composition | Relation::Transition => (edge.from, edge.to, ""),
            Relation::Connection => (edge.from, edge.to, " [constraint=false]"),
        };
        writeln!(out, "  n{tail} -> n{head}{attributes};").unwrap();
    }
    writeln!(out, "}}").unwrap();
    out
}

/// Feed `source` to `command -Tplain` and return what it wrote.
fn run(command: &str, source: &str) -> Result<String, GraphvizError> {
    let mut child = Command::new(command)
        .arg("-Tplain")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| GraphvizError::Spawn {
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
        return Err(GraphvizError::Failed {
            command: command.to_string(),
            detail: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    String::from_utf8(output.stdout).map_err(|_| GraphvizError::Unreadable {
        detail: "not UTF-8".to_string(),
    })
}

/// Positions out of `-Tplain`: one `graph` line with the canvas, one
/// `node` line per box with its centre, everything in inches with the
/// origin at the bottom left.
fn parse_plain(plain: &str, sizes: &[(f64, f64)], style: &Style) -> Result<Layout, GraphvizError> {
    let number = |part: Option<&str>| -> Result<f64, GraphvizError> {
        part.and_then(|text| text.parse().ok())
            .ok_or_else(|| GraphvizError::Unreadable {
                detail: format!("expected a number, got {part:?}"),
            })
    };

    let mut placed: Vec<Option<Placed>> = vec![None; sizes.len()];
    let mut canvas: Option<(f64, f64, f64)> = None;
    for line in plain.lines() {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("graph") => {
                let scale = number(parts.next())?;
                let width = number(parts.next())?;
                let height = number(parts.next())?;
                canvas = Some((scale, width, height));
            }
            Some("node") => {
                let (scale, _, height) = canvas.ok_or_else(|| GraphvizError::Unreadable {
                    detail: "a node before the graph line".to_string(),
                })?;
                let name = parts.next().unwrap_or("");
                let at: usize = name
                    .strip_prefix('n')
                    .and_then(|digits| digits.parse().ok())
                    .filter(|&at| at < sizes.len())
                    .ok_or_else(|| GraphvizError::Unreadable {
                        detail: format!("unknown node `{name}`"),
                    })?;
                let cx = number(parts.next())?;
                let cy = number(parts.next())?;
                // the sizes are this crate's own; dot only echoes them
                let (width, height_px) = sizes[at];
                placed[at] = Some(Placed {
                    node: at,
                    x: style.margin + scale * cx * DPI - width / 2.0,
                    y: style.margin + scale * (height - cy) * DPI - height_px / 2.0,
                    width,
                    height: height_px,
                });
            }
            _ => {}
        }
    }

    let (scale, width, height) = canvas.ok_or_else(|| GraphvizError::Unreadable {
        detail: "no graph line".to_string(),
    })?;
    let placed = placed
        .into_iter()
        .enumerate()
        .map(|(at, slot)| {
            slot.ok_or_else(|| GraphvizError::Unreadable {
                detail: format!("no position for node n{at}"),
            })
        })
        .collect::<Result<Vec<Placed>, GraphvizError>>()?;
    Ok(Layout {
        placed,
        width: scale * width * DPI + 2.0 * style.margin,
        height: scale * height * DPI + 2.0 * style.margin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::resolved;
    use crate::{definition_diagram, render_with_graphviz};

    /// A model with every relation kind the DOT emission distinguishes.
    const SOURCE: &str = "part def A;\n\
                          part def B :> A;\n\
                          requirement def R;\n\
                          satisfy R by B;\n";

    fn diagram() -> Diagram {
        let ws = resolved(SOURCE);
        definition_diagram(ws.model(), &[ws.root()])
    }

    /// Held while a test writes a stand-in for `dot` and runs it.
    ///
    /// Writing a program and then running it is a race when anything
    /// else in the process forks in between: the child inherits the
    /// still-open write handle, and Linux refuses to run a file that
    /// something holds open for writing. Two tests here write and run
    /// their own `dot`, and either one's fork can spoil the other's
    /// exec, which showed up as one or the other failing every few runs.
    /// Taking turns is enough -- nothing else in this process forks.
    static SPAWNING: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// An executable stand-in for `dot`, unique per test.
    fn fake_dot(name: &str, script: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path =
            std::env::temp_dir().join(format!("sysml-fake-dot-{name}-{}", std::process::id()));
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn the_dot_source_ranks_what_reads_downward() {
        let ws = resolved(
            "part def A;\npart def B :> A;\npart def W { part b : B; }\n\
             state def M { state x; state y; transition go first x then y; }\n\
             part def P { port p : A; }\npart def Q { port q : A; }\n\
             part def Pair { part l : P; part r : Q; connect l.p to r.q; }\n",
        );
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let sizes: Vec<(f64, f64)> = diagram
            .nodes
            .iter()
            .map(|node| box_size(node, &Style::default()))
            .collect();
        let dot = to_dot(&diagram, &sizes, &Style::default());

        assert!(dot.starts_with("digraph sysml {"));
        assert!(dot.contains("node [shape=box fixedsize=true label=\"\"];"));
        // every box appears with a size in inches
        for at in 0..diagram.nodes.len() {
            assert!(
                dot.contains(&format!("n{at} [width=")),
                "n{at} missing:\n{dot}"
            );
        }
        let index = |name: &str| {
            diagram
                .nodes
                .iter()
                .position(|node| node.name == name)
                .unwrap()
        };
        // the supertype is the tail so it lands above its subtype
        assert!(dot.contains(&format!("n{} -> n{};\n", index("A"), index("B"))));
        // the whole above its part
        assert!(dot.contains(&format!("n{} -> n{};\n", index("W"), index("B"))));

        // a connection -- drawn in the interconnection view -- is a peer
        // wire: no rank constraint
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
            .map(|node| box_size(node, &Style::default()))
            .collect();
        let wired = to_dot(&inside, &sizes, &Style::default());
        assert!(wired.contains("[constraint=false];"), "{wired}");
    }

    #[test]
    fn plain_output_maps_back_to_pixels() {
        let style = Style::default();
        let sizes = [(120.0, 40.0), (150.0, 60.0)];
        let plain = "graph 1 4 3\n\
                     node n0 1 2.5 1.6667 0.5556 x x x x x\n\
                     node n1 2 0.5 2.0833 0.8333 x x x x x\n\
                     edge n0 n1 2 1 2 2 1 x x x x\n\
                     stop\n";
        let layout = parse_plain(plain, &sizes, &style).unwrap();

        // 72 px per inch, y flipped so 2.5in from the bottom of a 3in
        // canvas is 0.5in from the top, margins added around everything
        assert_eq!(layout.placed[0].x, style.margin + 72.0 - 60.0);
        assert_eq!(layout.placed[0].y, style.margin + 0.5 * 72.0 - 20.0);
        assert_eq!(layout.placed[1].x, style.margin + 144.0 - 75.0);
        assert_eq!(layout.width, 4.0 * 72.0 + 2.0 * style.margin);
        assert_eq!(layout.height, 3.0 * 72.0 + 2.0 * style.margin);

        // a scaled drawing scales every coordinate with it
        let half = parse_plain(
            "graph 0.5 4 3\nnode n0 1 2.5 0 0\nnode n1 2 0.5 0 0\n",
            &sizes,
            &style,
        )
        .unwrap();
        assert_eq!(half.placed[0].x, style.margin + 36.0 - 60.0);
        assert_eq!(half.width, 2.0 * 72.0 + 2.0 * style.margin);
    }

    #[test]
    fn what_plain_never_says_is_refused() {
        let style = Style::default();
        let sizes = [(10.0, 10.0)];
        // every refusal is `Unreadable`; its Display carries the detail
        let unreadable = |plain: &str| {
            parse_plain(plain, &sizes, &style)
                .expect_err("plain that never parses")
                .to_string()
                .replace("unreadable `dot -Tplain` output: ", "")
        };

        assert_eq!(unreadable(""), "no graph line");
        assert_eq!(
            unreadable("node n0 1 1 1 1\ngraph 1 2 2\n"),
            "a node before the graph line"
        );
        assert!(unreadable("graph 1 2 2\nnode n7 1 1 1 1\n").contains("unknown node `n7`"));
        assert!(unreadable("graph 1 2 2\nnode wat 1 1 1 1\n").contains("unknown node `wat`"));
        assert!(unreadable("graph 1 2 2\nnode n0 1 pancake 1 1\n").contains("expected a number"));
        assert!(unreadable("graph one 2 2\n").contains("expected a number"));
        assert_eq!(unreadable("graph 1 2 2\n"), "no position for node n0");
    }

    #[test]
    fn a_fake_dot_lays_the_whole_diagram_out() {
        let _turn = SPAWNING.lock().unwrap_or_else(|e| e.into_inner());
        let diagram = diagram();
        assert_eq!(diagram.nodes.len(), 3);
        let script = fake_dot(
            "ok",
            "cat >/dev/null\n\
             printf 'graph 1 6 4\\n'\n\
             printf 'node n0 1 3 1 1\\n'\n\
             printf 'node n1 1 1 1 1\\n'\n\
             printf 'node n2 4 1 1 1\\n'\n\
             printf 'stop\\n'",
        );
        let svg =
            render_with_graphviz(&diagram, &Style::default(), script.to_str().unwrap()).unwrap();
        std::fs::remove_file(&script).ok();

        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(svg.contains(">A<") && svg.contains(">B<") && svg.contains(">R<"));
    }

    #[test]
    fn a_missing_or_broken_dot_is_reported_not_papered_over() {
        let _turn = SPAWNING.lock().unwrap_or_else(|e| e.into_inner());
        let diagram = diagram();
        let style = Style::default();

        let missing = graphviz_layout(&diagram, &style, "/nonexistent/graphviz/dot");
        assert!(matches!(missing, Err(GraphvizError::Spawn { .. })));
        assert!(missing
            .unwrap_err()
            .to_string()
            .contains("is Graphviz installed?"));

        let angry = fake_dot("angry", "echo 'boom' >&2\nexit 3");
        let failed = graphviz_layout(&diagram, &style, angry.to_str().unwrap());
        std::fs::remove_file(&angry).ok();
        let failed = failed.unwrap_err();
        assert!(matches!(&failed, GraphvizError::Failed { detail, .. } if detail == "boom"));
        assert!(failed.to_string().contains("failed: boom"));

        let mute = fake_dot("mute", "cat >/dev/null\necho pancakes");
        let unreadable = graphviz_layout(&diagram, &style, mute.to_str().unwrap());
        std::fs::remove_file(&mute).ok();
        assert!(matches!(unreadable, Err(GraphvizError::Unreadable { .. })));

        let binary = fake_dot("binary", "cat >/dev/null\nprintf '\\377\\376'");
        let not_text = graphviz_layout(&diagram, &style, binary.to_str().unwrap());
        std::fs::remove_file(&binary).ok();
        assert_eq!(
            not_text.unwrap_err().to_string(),
            "unreadable `dot -Tplain` output: not UTF-8"
        );
    }
}

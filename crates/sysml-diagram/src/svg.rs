//! Serializing a laid-out diagram as a standalone SVG document.

use std::collections::HashMap;
use std::fmt::Write;

use crate::{Diagram, Edge, Layout, Placed, Relation, Style};

/// Font stack for the drawing: the same families a browser would pick for
/// UI text, so a diagram looks native wherever it is embedded.
const FONT: &str =
    "ui-sans-serif, system-ui, -apple-system, Segoe UI, Helvetica, Arial, sans-serif";

/// Colours for both viewer themes. The document is self-contained, so the
/// palette travels with it rather than depending on the embedding page.
const CSS: &str = "\
:root { --box: #ffffff; --line: #3f4451; --text: #1b1f27; --muted: #6b7280; }\n\
@media (prefers-color-scheme: dark) {\n\
  :root { --box: #1f2430; --line: #9aa3b2; --text: #e6e9ef; --muted: #9aa3b2; }\n\
}\n\
.box { fill: var(--box); stroke: var(--line); stroke-width: 1.2; }\n\
.rule, .edge { stroke: var(--line); stroke-width: 1.2; fill: none; }\n\
.arrow { fill: var(--box); stroke: var(--line); stroke-width: 1.2; }\n\
.diamond { fill: var(--line); stroke: var(--line); stroke-width: 1.2; }\n\
.name { fill: var(--text); font-weight: 600; }\n\
.keyword, .feature { fill: var(--muted); }\n";

/// Render a laid-out diagram. The output is a complete SVG document: it can
/// be written to a `.svg` file or inlined into HTML as-is.
pub fn to_svg(diagram: &Diagram, layout: &Layout, style: &Style) -> String {
    let mut out = String::new();
    let (width, height) = (layout.width, layout.height);

    writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" \
         viewBox=\"0 0 {width:.0} {height:.0}\" font-family=\"{FONT}\" \
         font-size=\"{:.0}\">",
        style.font_size
    )
    .expect("writing to a String cannot fail");
    writeln!(out, "<style>\n{CSS}</style>").unwrap();
    writeln!(
        out,
        "<defs>\
         <marker id=\"specialization\" viewBox=\"0 0 12 10\" refX=\"12\" refY=\"5\" \
         markerWidth=\"12\" markerHeight=\"10\" orient=\"auto\">\
         <path class=\"arrow\" d=\"M0,0 L12,5 L0,10 z\"/></marker>\
         <marker id=\"composition\" viewBox=\"0 0 16 10\" refX=\"0\" refY=\"5\" \
         markerWidth=\"16\" markerHeight=\"10\" orient=\"auto\">\
         <path class=\"diamond\" d=\"M0,5 L8,0 L16,5 L8,10 z\"/></marker></defs>"
    )
    .unwrap();

    // edges first, so the boxes paint over the line ends
    let lanes = lanes(diagram);
    for (edge, lane) in diagram.edges.iter().zip(&lanes) {
        let from = &layout.placed[edge.from];
        let to = &layout.placed[edge.to];
        match edge.relation {
            // the layering already put the supertype above, so the line
            // runs from the subtype's top edge to the supertype's bottom
            Relation::Specialization => writeln!(
                out,
                "<line class=\"edge\" x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" \
                 marker-end=\"url(#specialization)\"/>",
                from.x + from.width / 2.0,
                from.y,
                to.x + to.width / 2.0,
                to.y + to.height,
            ),
            // neither of these follows the layering, so the line runs
            // centre to centre clipped to both borders. Composition puts a
            // filled diamond on the side of the whole; a connection is
            // undirected and gets no marker at all.
            Relation::Composition | Relation::Connection => {
                let (mut x1, mut y1) = border_point(from, centre_of(to));
                let (mut x2, mut y2) = border_point(to, centre_of(from));
                // shift edges sharing a pair of boxes along the normal, so
                // two connections do not collapse into one line
                let (dx, dy) = (x2 - x1, y2 - y1);
                let length = dx.hypot(dy);
                if length > 0.0 {
                    let shift = lane * style.line_height;
                    let (nx, ny) = (-dy / length * shift, dx / length * shift);
                    x1 += nx;
                    y1 += ny;
                    x2 += nx;
                    y2 += ny;
                }
                let marker = match edge.relation {
                    Relation::Composition => " marker-start=\"url(#composition)\"",
                    _ => "",
                };
                writeln!(
                    out,
                    "<line class=\"edge\" x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" \
                     y2=\"{y2:.1}\"{marker}/>"
                )
                .unwrap();
                // name the feature each end attaches to, near its own box
                if let Some((first, second)) = &edge.ends {
                    end_label(&mut out, (x1, y1), (x2, y2), 0.18, first);
                    end_label(&mut out, (x1, y1), (x2, y2), 0.82, second);
                }
                Ok(())
            }
        }
        .unwrap();
    }

    for placed in &layout.placed {
        let node = &diagram.nodes[placed.node];
        let centre = placed.x + placed.width / 2.0;
        let header = placed.y + style.padding;

        writeln!(
            out,
            "<g>\n<rect class=\"box\" x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"4\"/>",
            placed.x, placed.y, placed.width, placed.height
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
            "<text class=\"name\" x=\"{centre:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{}</text>",
            header + 1.75 * style.line_height,
            escape(&node.name)
        )
        .unwrap();

        if !node.features.is_empty() {
            let rule = header + 2.0 * style.line_height + style.padding / 2.0;
            writeln!(
                out,
                "<line class=\"rule\" x1=\"{:.1}\" y1=\"{rule:.1}\" x2=\"{:.1}\" y2=\"{rule:.1}\"/>",
                placed.x,
                placed.x + placed.width
            )
            .unwrap();
            for (row, feature) in node.features.iter().enumerate() {
                writeln!(
                    out,
                    "<text class=\"feature\" x=\"{:.1}\" y=\"{:.1}\">{}</text>",
                    placed.x + style.padding,
                    header
                        + 2.0 * style.line_height
                        + style.padding
                        + (row as f64 + 0.75) * style.line_height,
                    escape(&feature.label())
                )
                .unwrap();
            }
        }
        writeln!(out, "</g>").unwrap();
    }

    writeln!(out, "</svg>").unwrap();
    out
}

/// Perpendicular offset factor for each edge: edges sharing a pair of boxes
/// are spread symmetrically about the straight line between them.
fn lanes(diagram: &Diagram) -> Vec<f64> {
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
            let slot = taken.entry(pair(edge)).or_default();
            let index = *slot;
            *slot += 1;
            index as f64 - (total[&pair(edge)] as f64 - 1.0) / 2.0
        })
        .collect()
}

/// Write one end's feature name at `fraction` along the edge.
fn end_label(out: &mut String, start: (f64, f64), end: (f64, f64), fraction: f64, name: &str) {
    let x = start.0 + (end.0 - start.0) * fraction;
    let y = start.1 + (end.1 - start.1) * fraction;
    writeln!(
        out,
        "<text class=\"feature\" x=\"{x:.1}\" y=\"{y:.1}\" text-anchor=\"middle\">{}</text>",
        escape(name)
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
fn escape(text: &str) -> String {
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
    fn a_feature_compartment_gets_a_rule_and_one_line_each() {
        let svg = svg_of(
            "part def FuelPort;\n\
             part def Engine {\n\
             	attribute power;\n\
             	port fuelIn : FuelPort;\n\
             }\n",
        );
        assert_eq!(svg.matches("<line class=\"rule\"").count(), 1);
        assert!(svg.contains(">attribute power</text>"));
        assert!(svg.contains(">port fuelIn : FuelPort</text>"));
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

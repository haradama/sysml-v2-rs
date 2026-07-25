//! Serializing a laid-out diagram as a standalone SVG document.

use std::fmt::Write;

use crate::{Diagram, Layout, Style};

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
        "<defs><marker id=\"specialization\" viewBox=\"0 0 12 10\" refX=\"12\" refY=\"5\" \
         markerWidth=\"12\" markerHeight=\"10\" orient=\"auto\">\
         <path class=\"arrow\" d=\"M0,0 L12,5 L0,10 z\"/></marker></defs>"
    )
    .unwrap();

    // edges first, so the boxes paint over the line ends
    for edge in &diagram.edges {
        let subtype = &layout.placed[edge.from];
        let supertype = &layout.placed[edge.to];
        writeln!(
            out,
            "<line class=\"edge\" x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" \
             marker-end=\"url(#specialization)\"/>",
            subtype.x + subtype.width / 2.0,
            subtype.y,
            supertype.x + supertype.width / 2.0,
            supertype.y + supertype.height,
        )
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
    use crate::{definition_diagram, layout, render, Node};
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

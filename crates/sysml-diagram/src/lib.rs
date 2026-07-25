//! Definition diagrams for [`sysml_model::Model`], rendered as SVG.
//!
//! Collects the definitions of a resolved model together with the
//! specializations between them (`part def Engine :> PowerSource`), lays the
//! result out as a layered graph with supertypes above their subtypes, and
//! serializes it as a standalone SVG document.
//!
//! Nothing external is involved: layering, crossing reduction, text metrics
//! and the SVG itself are all produced here, so a model always renders to the
//! same bytes and the result needs no viewer beyond a browser.
//!
//! Only specializations the model reifies are drawn. `sysml-semantics`
//! reifies the ones written in the source but not the implicit library
//! supertypes every definition inherits, so a diagram does not collapse into
//! a hub of edges into `Parts::Part`.
//!
//! ```
//! use sysml_semantics::Workspace;
//!
//! let mut ws = Workspace::new();
//! ws.add_file(
//!     "vehicle.sysml",
//!     "part def PowerSource;\npart def Engine :> PowerSource;\n",
//! );
//! ws.resolve_all();
//!
//! let diagram = sysml_diagram::definition_diagram(ws.model(), &[ws.root()]);
//! assert_eq!(diagram.nodes.len(), 2);
//!
//! let svg = sysml_diagram::render(&diagram, &sysml_diagram::Style::default());
//! assert!(svg.starts_with("<svg xmlns="));
//! ```

mod graph;
mod layout;
mod svg;

pub use graph::{definition_diagram, Diagram, Edge, Feature, Node};
pub use layout::{layout, Layout, Placed};
pub use svg::to_svg;

/// Sizes and spacing shared by the layout and the renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// Font size of an element name, in pixels.
    pub font_size: f64,
    /// Baseline-to-baseline distance inside a compartment.
    pub line_height: f64,
    /// Space between a box's border and its text.
    pub padding: f64,
    /// Horizontal space between neighbouring boxes.
    pub h_gap: f64,
    /// Vertical space between layers, leaving room for the edges.
    pub v_gap: f64,
    /// Space around the whole drawing.
    pub margin: f64,
}

impl Default for Style {
    fn default() -> Style {
        Style {
            font_size: 13.0,
            line_height: 18.0,
            padding: 10.0,
            h_gap: 32.0,
            v_gap: 56.0,
            margin: 16.0,
        }
    }
}

impl Style {
    /// Rough advance width of `text`. Boxes are sized without a font engine,
    /// so this assumes the average glyph of a sans-serif face is 0.6 em --
    /// wide enough for the ASCII identifiers SysML models are written with.
    pub(crate) fn text_width(&self, text: &str) -> f64 {
        text.chars().count() as f64 * self.font_size * 0.6
    }
}

/// Lay `diagram` out and render it as a standalone SVG document.
pub fn render(diagram: &Diagram, style: &Style) -> String {
    to_svg(diagram, &layout(diagram, style), style)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_semantics::Workspace;

    /// A workspace holding one resolved file, as the CLI would build it.
    pub(crate) fn resolved(source: &str) -> Workspace {
        let mut ws = Workspace::new();
        ws.add_file("test.sysml", source);
        ws.resolve_all();
        ws
    }

    #[test]
    fn text_width_scales_with_the_font() {
        let style = Style::default();
        assert!(style.text_width("mm") > style.text_width("m"));
        assert_eq!(style.text_width(""), 0.0);

        let bigger = Style {
            font_size: 26.0,
            ..Style::default()
        };
        assert_eq!(bigger.text_width("m"), 2.0 * style.text_width("m"));
    }

    #[test]
    fn render_produces_a_standalone_document() {
        let ws = resolved("part def A;\npart def B :> A;\n");
        let diagram = definition_diagram(ws.model(), &[ws.root()]);
        let svg = render(&diagram, &Style::default());

        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(svg.ends_with("</svg>\n"));
        assert!(svg.contains(">A<") && svg.contains(">B<"));
    }

    #[test]
    fn the_default_style_is_self_consistent() {
        let style = Style::default();
        assert_eq!(style, Style::default());
        assert!(format!("{style:?}").contains("font_size"));
        // a layer has to clear a box plus the edge running into it
        assert!(style.v_gap > style.line_height);
    }
}

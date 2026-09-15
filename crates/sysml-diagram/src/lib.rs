//! Definition diagrams for [`sysml_model::Model`], rendered as SVG.
//!
//! Collects the definitions of a resolved model together with the
//! specializations between them, lays the result out as a layered graph
//! with supertypes above their subtypes, and serializes it as a standalone
//! SVG document.
//!
//! Where the boxes go is the Eclipse Layout Kernel's answer, through
//! `elkrs` -- ELK's algorithms ported to Rust and linked in, so nothing
//! is spawned and nothing has to be installed. Everything visible is
//! drawn here: no font engine is involved, and the engine decides nothing
//! at random, so a model always renders to the same bytes. Only how many
//! columns a character takes is read from Unicode's own table.
//!
//! Only specializations the model reifies are drawn -- not the implicit
//! library supertypes every definition inherits, or a diagram would
//! collapse into a hub of edges into `Parts::Part`.
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

mod browser;
mod elk;
mod graph;
mod layout;
mod sequence;
pub mod skin;
mod svg;

pub use browser::{browser_view, Browser, Row};
pub use graph::{
    definition_diagram, interconnection_diagram, lines, Compartment, Diagram, Edge, Feature, Node,
    Relation, Shape,
};
pub use layout::{layout, Layout, Placed};
pub use sequence::{sequence_view, Lifeline, Moment, Sequence};
pub use skin::{Colour, Palette, Skin, Tint};
pub use svg::to_svg;

/// Sizes and spacing shared by the layout and the renderer, and the
/// skin the drawing is painted in.
#[derive(Clone, Debug, PartialEq)]
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
    /// How far one level of a browser view is indented from the last.
    pub indent: f64,
    /// Width a layer may reach before it wraps onto another row.
    pub max_row_width: f64,
    /// What the drawing is painted in. The notation is not here: a skin
    /// says colour, face and weight, and never which marker means what.
    pub skin: Skin,
}

impl Default for Style {
    fn default() -> Style {
        Style {
            font_size: 12.0,
            line_height: 17.0,
            padding: 10.0,
            h_gap: 32.0,
            v_gap: 56.0,
            margin: 16.0,
            indent: 24.0,
            max_row_width: 1600.0,
            skin: Skin::default(),
        }
    }
}

impl Style {
    /// How tall the tab in a package's top left corner is.
    ///
    /// One line of the name with room above and below it, which is what
    /// makes the tab sit close around what it says rather than reading
    /// as a box in its own right. The drawing and both layouts have to
    /// agree on it, or what the package holds is placed under its name.
    pub(crate) fn package_tab(&self) -> f64 {
        self.padding + self.line_height
    }

    /// Rough advance width of `text`. Boxes are sized without a font engine,
    /// so this assumes the average glyph of a sans-serif face is 0.6 em --
    /// wide enough for the ASCII identifiers SysML models are written with.
    pub(crate) fn text_width(&self, text: &str) -> f64 {
        columns(text) as f64 * self.font_size * 0.6
    }
}

/// How many columns `text` takes, counting a character an em across as
/// two.
///
/// The 0.6 em an ASCII letter is estimated at is about right for Latin and
/// about half of what a CJK character takes, so a `doc` written in one of
/// them would be measured at half its width. Two columns overstates such a
/// character slightly, which leaves a box wider than it needs to be rather
/// than prose wider than its box.
///
/// Which characters those are is UAX #11's East Asian Width, read from
/// `unicode-width`: the property covers far more than the CJK blocks, and
/// a list copied into this file would be a snapshot of one Unicode release
/// with nothing to check it against.
///
/// What [`svg::escape`] drops on the way out is dropped here too.
///
/// [`svg::escape`]: crate::svg::escape
pub(crate) fn columns(text: &str) -> usize {
    if text.chars().all(svg::xml_carries) {
        return unicode_width::UnicodeWidthStr::width(text);
    }
    let drawn: String = text.chars().filter(|&ch| svg::xml_carries(ch)).collect();
    unicode_width::UnicodeWidthStr::width(drawn.as_str())
}

/// Lay `diagram` out and render it as a standalone SVG document.
pub fn render(diagram: &Diagram, style: &Style) -> String {
    to_svg(diagram, &layout(diagram, style), style)
}

/// Render a browser view as a standalone SVG document.
pub fn render_browser(browser: &Browser, style: &Style) -> String {
    browser::to_svg(browser, style)
}

/// Render a sequence view as a standalone SVG document.
pub fn render_sequence(sequence: &Sequence, style: &Style) -> String {
    sequence::to_svg(sequence, style)
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

    /// A layout with every route left to the renderer, which is how a
    /// view ELK does not lay out -- a swimlane view -- reaches it. ELK
    /// routes as it places, so where it answers, its routes are drawn.
    pub(crate) fn routed_here(diagram: &Diagram, style: &Style) -> crate::Layout {
        let mut laid_out = layout(diagram, style);
        laid_out.routes.iter_mut().for_each(Vec::clear);
        laid_out
    }

    #[test]
    fn text_width_scales_with_the_font() {
        let style = Style::default();
        assert!(style.text_width("mm") > style.text_width("m"));
        assert_eq!(style.text_width(""), 0.0);
        // a Chinese, Japanese or Korean character is about an em across,
        // twice what the estimate allows a Latin letter
        assert_eq!(style.text_width("\u{57fa}"), 2.0 * style.text_width("m"));
        assert_eq!(columns("ab\u{57fa}"), 4);

        let bigger = Style {
            font_size: 24.0,
            ..Style::default()
        };
        assert_eq!(bigger.text_width("m"), 2.0 * style.text_width("m"));
    }

    #[test]
    fn a_character_drawn_an_em_across_is_measured_as_two_columns() {
        // a Yi syllable and an emoji are as square as an ideograph is,
        // and a name or a `doc` can hold either
        assert_eq!(columns("\u{a000}"), 2);
        assert_eq!(columns("\u{1f600}"), 2);
        assert_eq!(columns("\u{ff21}"), 2);
        // where the width is Ambiguous the character is narrow unless
        // the surrounding text says otherwise, and this one is prose in
        // a Latin document as often as not
        assert_eq!(columns("\u{3248}"), 1);
        // and a combining mark is drawn over the letter before it
        assert_eq!(columns("e\u{301}"), 1);
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

//! The browser view: the model's membership hierarchy as an indented tree.
//!
//! This is the one standard view that needs nothing but ownership, so it
//! reads whatever the model holds -- no connections, no layering, no
//! resolution required beyond what built the elements.

use std::fmt::Write;

use sysml_model::{ElementId, Model};

use crate::graph::keyword;
use crate::svg::{document, escape};
use crate::Style;

/// One line of the tree: an element and how deep it sits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: ElementId,
    /// Indentation level, counting only the named elements above it.
    pub depth: usize,
    /// SysML keyword shown in guillemets, e.g. `part def`.
    pub keyword: String,
    pub name: String,
}

impl Row {
    /// The line as it appears in the drawing.
    pub fn label(&self) -> String {
        format!("\u{ab}{}\u{bb} {}", self.keyword, self.name)
    }
}

/// The rows of a browser view, in document order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Browser {
    pub rows: Vec<Row>,
}

/// Walk the ownership tree under `roots`, one row per named element.
///
/// Unnamed elements -- the reified relationships, a file's synthetic root,
/// the expression behind a guard -- carry no line of their own and do not
/// indent what they contain, so the tree reads as the source is written.
pub fn browser_view(model: &Model, roots: &[ElementId]) -> Browser {
    let mut rows = Vec::new();
    for &root in roots {
        collect(model, root, 0, &mut rows);
    }
    Browser { rows }
}

fn collect(model: &Model, element: ElementId, depth: usize, rows: &mut Vec<Row>) {
    let named = model.name(element);
    if let Some(name) = named {
        rows.push(Row {
            id: element,
            depth,
            keyword: keyword(model.kind(element)),
            name: name.to_string(),
        });
    }
    let below = depth + usize::from(named.is_some());
    for &child in model.owned(element) {
        collect(model, child, below, rows);
    }
}

/// Render a browser view as a standalone SVG document.
pub fn to_svg(browser: &Browser, style: &Style) -> String {
    let widest = browser
        .rows
        .iter()
        .map(|row| row.depth as f64 * style.indent + style.text_width(&row.label()))
        .fold(0.0f64, f64::max);
    let width = widest + 2.0 * style.margin;
    let height = browser.rows.len() as f64 * style.line_height + 2.0 * style.margin;

    let mut body = String::new();
    for (index, row) in browser.rows.iter().enumerate() {
        let x = style.margin + row.depth as f64 * style.indent;
        let y = style.margin + (index as f64 + 0.5) * style.line_height;

        // a rule down the left of everything this row contains, so a deep
        // tree still shows what belongs to what
        let last = last_descendant(&browser.rows, index);
        if last > index {
            writeln!(
                body,
                "<line class=\"guide\" x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\"/>",
                x + 0.25 * style.indent,
                y + 0.35 * style.line_height,
                x + 0.25 * style.indent,
                style.margin + (last as f64 + 0.5) * style.line_height,
            )
            .unwrap();
        }
        if row.depth > 0 {
            let parent = x - 0.75 * style.indent;
            writeln!(
                body,
                "<line class=\"guide\" x1=\"{parent:.1}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\"/>",
                x - 0.15 * style.indent,
            )
            .unwrap();
        }
        writeln!(
            body,
            "<text class=\"keyword\" x=\"{x:.1}\" y=\"{y:.1}\" dominant-baseline=\"middle\">\
             \u{ab}{}\u{bb} <tspan class=\"name\">{}</tspan></text>",
            escape(&row.keyword),
            escape(&row.name),
        )
        .unwrap();
    }
    document(width, height, style, &body)
}

/// Index of the last row nested under the one at `index`.
fn last_descendant(rows: &[Row], index: usize) -> usize {
    let depth = rows[index].depth;
    let mut last = index;
    for (offset, row) in rows.iter().enumerate().skip(index + 1) {
        if row.depth <= depth {
            break;
        }
        last = offset;
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::resolved;

    fn tree(source: &str) -> Browser {
        let ws = resolved(source);
        browser_view(ws.model(), &[ws.root()])
    }

    #[test]
    fn rows_follow_the_ownership_tree() {
        let browser = tree(
            "package P {\n\
             \tpart def Wheel { port hub; }\n\
             \tpart def Car { part w : Wheel; }\n\
             }\n",
        );
        let shape: Vec<(usize, &str)> = browser
            .rows
            .iter()
            .map(|row| (row.depth, row.name.as_str()))
            .collect();
        assert_eq!(
            shape,
            [(0, "P"), (1, "Wheel"), (2, "hub"), (1, "Car"), (2, "w"),]
        );
        assert_eq!(browser.rows[1].label(), "\u{ab}part def\u{bb} Wheel");
    }

    #[test]
    fn unnamed_elements_neither_show_nor_indent() {
        // the reified typing under `w` is unnamed, and the workspace root
        // above `P` is too
        let browser = tree("package P {\n\tpart def Wheel;\n\tpart w : Wheel;\n}\n");
        assert_eq!(browser.rows.len(), 3);
        assert!(browser.rows.iter().all(|row| !row.name.is_empty()));
        assert_eq!(browser.rows[2].depth, 1);
    }

    #[test]
    fn an_empty_model_yields_no_rows() {
        assert_eq!(tree(""), Browser::default());
    }

    #[test]
    fn the_svg_carries_one_line_of_text_per_row() {
        // `Car` follows the subtree under `Wheel`, so the rule beneath
        // `Wheel` has to stop before it
        let browser = tree(
            "package P {\n\
             \tpart def Wheel { port hub; }\n\
             \tpart def Car { part w : Wheel; }\n\
             }\n",
        );
        let svg = to_svg(&browser, &Style::default());

        assert!(svg.starts_with("<svg xmlns="));
        assert!(svg.ends_with("</svg>\n"));
        assert_eq!(svg.matches("<text class=\"keyword\"").count(), 5);
        assert!(svg.contains(">hub</tspan>"));
        // a rule under each of P, Wheel and Car, and a tick into each of
        // the four rows below the top
        assert_eq!(svg.matches("class=\"guide\"").count(), 7);
    }

    #[test]
    fn an_empty_browser_still_renders_a_document() {
        let svg = to_svg(&Browser::default(), &Style::default());
        assert!(svg.starts_with("<svg xmlns="));
        assert!(!svg.contains("<text"));
    }

    #[test]
    fn a_leaf_has_no_descendants() {
        let rows = tree("part def A;\n").rows;
        assert_eq!(last_descendant(&rows, 0), 0);
    }
}

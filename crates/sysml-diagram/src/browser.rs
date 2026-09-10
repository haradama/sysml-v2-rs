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
    /// The element this row stands for.
    pub id: ElementId,
    /// Indentation level, counting only the named elements above it.
    pub depth: usize,
    /// SysML keyword shown in guillemets, e.g. `part def`.
    pub keyword: String,
    /// What it is called, as the tree shows it.
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
    /// The rows, in the order the model declares them.
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
    let ends = last_children(&browser.rows);
    for (index, row) in browser.rows.iter().enumerate() {
        let x = style.margin + row.depth as f64 * style.indent;
        let y = style.margin + (index as f64 + 0.5) * style.line_height;

        // a rule down the left of everything this row contains, so a deep
        // tree still shows what belongs to what. It stops at the last row
        // that hangs off it: what the last of those holds is further in
        // and hangs off a rule of its own, so a rule drawn down to the
        // last of *those* would run past every tick joining it and end in
        // mid-air.
        let last = ends[index];
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

/// For each row, the index of the last row directly under it -- its own,
/// where it holds nothing.
///
/// The rows are the ownership tree flattened in order and a row is one
/// deeper than the row that holds it, so one pass down them settles every
/// row at once: the row a line hangs off is the last one above it that is
/// one shallower, and every row that hangs off it says so as it passes.
fn last_children(rows: &[Row]) -> Vec<usize> {
    let mut last: Vec<usize> = (0..rows.len()).collect();
    // the row each depth is currently inside of, shallowest first
    let mut above: Vec<usize> = Vec::new();
    for (at, row) in rows.iter().enumerate() {
        above.truncate(row.depth);
        if let Some(&holding) = above.last() {
            last[holding] = at;
        }
        above.push(at);
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
    fn a_leaf_holds_nothing() {
        let rows = tree("part def A;\n").rows;
        assert_eq!(last_children(&rows), [0]);
    }

    #[test]
    fn a_row_ends_where_what_it_holds_does() {
        // P holds Wheel, which holds hub, and then Car beside Wheel: the
        // package runs to the last row, Wheel to its own port, and the
        // rows that hold nothing end where they begin
        let rows = tree(
            "package P {\n\
             \tpart def Wheel { port hub; }\n\
             \tpart def Car;\n\
             }\n",
        )
        .rows;
        assert_eq!(rows.len(), 4);
        assert_eq!(last_children(&rows), [3, 2, 2, 3]);
    }

    #[test]
    fn a_rule_stops_at_the_last_row_that_hangs_off_it() {
        // What `Wheel` holds sits below it and is the last row of all,
        // but it hangs off `Wheel`'s rule rather than `P`'s: a rule of
        // `P`'s drawn down to it would pass every tick joining it and
        // stop under the tree with nothing there.
        let rows = tree(
            "package P {\n\
             \tpart def Car;\n\
             \tpart def Wheel { port hub; }\n\
             }\n",
        )
        .rows;
        assert_eq!(rows.len(), 4);
        assert_eq!(last_children(&rows), [2, 1, 3, 3]);

        // and the drawing says the same, in the one line that is P's
        // rule: from just below its own row to the row `Wheel` is on
        let style = Style::default();
        let svg = to_svg(&Browser { rows }, &style);
        let middle = |at: usize| style.margin + (at as f64 + 0.5) * style.line_height;
        let x = style.margin + 0.25 * style.indent;
        let rule = format!(
            "<line class=\"guide\" x1=\"{x:.1}\" y1=\"{:.1}\" x2=\"{x:.1}\" y2=\"{:.1}\"/>",
            middle(0) + 0.35 * style.line_height,
            middle(2),
        );
        assert!(svg.contains(&rule), "{svg}");
    }
}

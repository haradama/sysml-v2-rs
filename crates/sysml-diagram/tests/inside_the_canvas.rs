//! Nothing a drawing says may fall off the canvas it is drawn on.
//!
//! A name set beside a port on the outer border of an outermost box
//! reads outwards, away from the drawing, and the viewer clips whatever
//! reaches past the `viewBox`. This measures every `<text>` the way the
//! renderer sized the boxes -- 0.6 em a column, two columns for a
//! character an em across -- and holds it inside the canvas.
//!
//! Skipped when the submodule is not checked out.

use std::path::{Path, PathBuf};

use sysml_diagram::{definition_diagram, interconnection_diagram, render, render_with_elk, Style};
use sysml_semantics::Workspace;

fn corpus() -> Option<PathBuf> {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/sysml-v2-release/sysml/src");
    if root.is_dir() {
        return Some(root);
    }
    eprintln!("skipping: {} not checked out", root.display());
    None
}

fn sysml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sysml_files(&path, out);
        } else if path.extension().is_some_and(|it| it == "sysml") {
            out.push(path);
        }
    }
}

/// The value of one attribute of an opening tag.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    tag.split_once(&format!("{name}=\""))
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(value, _)| value)
}

fn number(tag: &str, name: &str) -> f64 {
    attr(tag, name)
        .unwrap_or_else(|| panic!("`{name}` in `{tag}`"))
        .parse()
        .expect("a number")
}

/// The words of a `<text>`, with the tspans and the escapes undone.
fn plain(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some((before, after)) = rest.split_once('<') {
        out.push_str(before);
        rest = after.split_once('>').map(|(_, tail)| tail).unwrap_or("");
    }
    out.push_str(rest);
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// How many columns a string takes, counting a character an em across as
/// two -- the estimate the renderer sizes its boxes with.
fn columns(text: &str) -> usize {
    text.chars()
        .map(|ch| if (ch as u32) > 0x2e7f { 2 } else { 1 })
        .sum()
}

/// Every `<text>` of a document that reaches past its canvas.
///
/// The canvas is written out rounded to whole pixels, so half a pixel of
/// overhang is the rounding rather than a finding.
fn overflows(svg: &str) -> Vec<String> {
    let head = svg.split_once('>').expect("an opening tag").0;
    let view = attr(head, "viewBox").expect("a viewBox");
    let sides: Vec<f64> = view.split(' ').map(|it| it.parse().unwrap()).collect();
    let (width, height) = (sides[2], sides[3]);
    let font = number(head, "font-size");

    let mut bad = Vec::new();
    for piece in svg.split("<text").skip(1) {
        let (tag, rest) = piece.split_once('>').expect("a closed tag");
        let text = plain(rest.split("</text>").next().expect("a closed element"));
        let span = columns(&text) as f64 * font * 0.6;
        let (x, y) = (number(tag, "x"), number(tag, "y"));
        let (left, right) = match attr(tag, "text-anchor") {
            Some("middle") => (x - span / 2.0, x + span / 2.0),
            Some("end") => (x - span, x),
            _ => (x, x + span),
        };
        if left < -0.5 || right > width + 0.5 || y < 0.0 || y > height {
            bad.push(format!(
                "`{text}` spans x {left:.0}..{right:.0} at y {y:.0}, outside {width}x{height}"
            ));
        }
    }
    bad
}

#[test]
fn every_word_of_a_corpus_drawing_is_inside_its_canvas() {
    let Some(root) = corpus() else { return };
    let mut files = Vec::new();
    sysml_files(&root, &mut files);
    files.sort();
    assert!(files.len() > 100, "the corpus is there");

    let style = Style::default();
    let mut findings = Vec::new();
    for path in &files {
        let mut ws = Workspace::new();
        ws.add_file(
            path.to_string_lossy(),
            &std::fs::read_to_string(path).unwrap(),
        );
        ws.resolve_all();
        let roots = ws.file_roots(0).to_vec();
        let svg = render(&definition_diagram(ws.model(), &roots), &style);
        for bad in overflows(&svg) {
            findings.push(format!("{}: {bad}", path.display()));
        }
    }
    assert!(
        findings.is_empty(),
        "{} of {} drawings run off the canvas:\n{}",
        findings.len(),
        files.len(),
        findings.join("\n")
    );
}

#[test]
fn every_word_of_an_internal_view_is_inside_its_canvas() {
    let Some(root) = corpus() else { return };
    let mut files = Vec::new();
    sysml_files(&root, &mut files);
    files.sort();

    let style = Style::default();
    let mut findings = Vec::new();
    for path in &files {
        let mut ws = Workspace::new();
        ws.add_file(
            path.to_string_lossy(),
            &std::fs::read_to_string(path).unwrap(),
        );
        ws.resolve_all();
        let model = ws.model();
        for root in ws.file_roots(0).to_vec() {
            for id in std::iter::once(root).chain(model.descendants(root)) {
                let view = interconnection_diagram(model, id);
                if view.nodes.is_empty() {
                    continue;
                }
                for bad in overflows(&render(&view, &style)) {
                    findings.push(format!("{}: {bad}", path.display()));
                }
            }
        }
    }
    assert!(
        findings.is_empty(),
        "{} internal views run off the canvas:\n{}",
        findings.len(),
        findings.join("\n")
    );
}

/// The same drawings with the positions ELK chose: it is told the sizes
/// of the boxes, not of the names written round them, so the room has to
/// be left after it has answered.
///
/// Skipped where `elkrs` is not installed.
#[test]
fn every_word_of_an_elk_drawing_is_inside_its_canvas() {
    let Some(root) = corpus() else { return };
    let style = Style::default();
    let mut findings = Vec::new();
    for name in [
        "validation/10-Analysis and Trades/10d-Dynamics Analysis.sysml",
        "validation/14-Language Extensions/14b-Language Extensions.sysml",
        "validation/07-Variant Configuration/7b-Variant Configurations.sysml",
        "validation/12-Dependency Relationships/12b-Allocation-1.sysml",
    ] {
        let path = root.join(name);
        let mut ws = Workspace::new();
        ws.add_file(
            path.to_string_lossy(),
            &std::fs::read_to_string(&path).unwrap(),
        );
        ws.resolve_all();
        let roots = ws.file_roots(0).to_vec();
        let diagram = definition_diagram(ws.model(), &roots);
        match render_with_elk(&diagram, &style, "elkrs") {
            Ok(svg) => findings.extend(
                overflows(&svg)
                    .into_iter()
                    .map(|bad| format!("{name}: {bad}")),
            ),
            Err(why) => {
                eprintln!("skipping: {why}");
                return;
            }
        }
    }
    assert!(
        findings.is_empty(),
        "{} ELK drawings run off the canvas:\n{}",
        findings.len(),
        findings.join("\n")
    );
}

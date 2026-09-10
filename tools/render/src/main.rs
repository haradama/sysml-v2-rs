//! Rasterize a generated diagram so it can be looked at.
//!
//! The crates render SVG, which is the right thing to ship but cannot be
//! inspected without a viewer. This turns one into a PNG, which review
//! tools -- and image-reading assistants -- can open directly.
//!
//! Deliberately outside the workspace: it exists to look at the output, not
//! to be part of it, and it drags in a rasterizer the published crates have
//! no business depending on.
//!
//! ```sh
//! cargo run --manifest-path tools/render/Cargo.toml -- in.svg out.png [scale]
//! ```

use std::path::Path;

use resvg::tiny_skia;
use resvg::usvg;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (input, output, scale) = match args.as_slice() {
        [input, output] => (input, output, 2.0),
        [input, output, scale] => (input, output, scale.parse().unwrap_or(2.0)),
        _ => {
            eprintln!("usage: render-svg <in.svg> <out.png> [scale]");
            std::process::exit(2);
        }
    };
    if let Err(err) = render(Path::new(input), Path::new(output), scale) {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
    eprintln!("wrote {output}");
}

fn render(input: &Path, output: &Path, scale: f32) -> Result<(), String> {
    let svg = std::fs::read_to_string(input).map_err(|e| format!("cannot read {input:?}: {e}"))?;

    let mut fontdb = usvg::fontdb::Database::new();
    fontdb.load_system_fonts();
    // the diagrams ask for a UI font stack that ends in `sans-serif`; the
    // generic has to point at something actually installed or every label
    // is silently dropped
    for family in ["DejaVu Sans", "Liberation Sans", "Noto Sans", "Arial"] {
        if fontdb.faces().any(|face| {
            face.families
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case(family))
        }) {
            fontdb.set_sans_serif_family(family);
            break;
        }
    }
    let options = usvg::Options {
        fontdb: std::sync::Arc::new(fontdb),
        ..usvg::Options::default()
    };

    // the drawing's own light palette is what this reads: `usvg` answers
    // no media query, so the dark half of the stylesheet passes it by
    let tree =
        usvg::Tree::from_str(&svg, &options).map_err(|e| format!("cannot parse the SVG: {e}"))?;
    let size = tree.size().to_int_size().scale_by(scale).ok_or("empty")?;
    let mut pixmap =
        tiny_skia::Pixmap::new(size.width(), size.height()).ok_or("canvas too large")?;
    // white, so the drawing reads the way a light viewer would show it
    pixmap.fill(tiny_skia::Color::WHITE);
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .save_png(output)
        .map_err(|e| format!("cannot write {output:?}: {e}"))
}


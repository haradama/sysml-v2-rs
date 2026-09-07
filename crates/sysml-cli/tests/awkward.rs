//! Input that means one thing in SysML and another downstream.
//!
//! A name in SysML is whatever the modeller quoted, and the formats this
//! toolchain writes into all have characters of their own: `<` and `&`
//! end things in SVG, `*/` ends a Rust doc comment, a quote ends a Rust
//! string. Model JSON arriving from elsewhere may be short of a field or
//! have one of the wrong type. And a model may be nested far deeper than
//! anyone would write by hand, which is where a recursive descent runs
//! out of stack.

use serde_json::Value as Json;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Names and documentation SysML lets a modeller write that the formats
/// downstream give a meaning of their own.
const AWKWARD: &str = concat!(
    "package 'P&Q' {\n",
    "\tprivate import ScalarValues::*;\n",
    "\tdoc /* an ampersand & a less-than < and a quote \" and an apostrophe ' */\n",
    "\tpart def 'A<B' {\n",
    "\t\tdoc /* ends with a star-slash lookalike * / and a backslash \\ */\n",
    "\t\tattribute 'x\"y' : Real;\n",
    "\t\tattribute 'brace{}' : Real;\n",
    "\t}\n",
    "\tpart def '\"quoted\"' :> 'A<B';\n",
    "\tpart def 'tab\there';\n",
    "\tpart a : 'A<B';\n",
    "\tconnect a to a;\n",
    "}\n",
);

const SCALARS: &str = "package ScalarValues {\n\tabstract datatype Real;\n}\n";

#[test]
fn names_the_formats_would_read_as_their_own_syntax() {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file("scalars.kerml", SCALARS);
    let file = ws.add_file("awkward.sysml", AWKWARD);
    let parse = ws.file_parse(file).clone();
    assert!(
        parse.ok(),
        "the probe model must parse: {:?}",
        parse.errors()
    );
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "{:?}", ws.unresolved());
    let roots = ws.file_roots(file).to_vec();

    let mut found: Vec<String> = Vec::new();

    // SVG: a name is text inside an element, and `<` there ends it
    let style = sysml_diagram::Style::default();
    for (what, svg) in [
        (
            "diagram",
            sysml_diagram::render(
                &sysml_diagram::definition_diagram(ws.model(), &roots),
                &style,
            ),
        ),
        (
            "browser",
            sysml_diagram::render_browser(&sysml_diagram::browser_view(ws.model(), &roots), &style),
        ),
    ] {
        match well_formed(&svg) {
            Err(e) => found.push(format!("{what}-svg\t{e}")),
            Ok(()) => {
                if !svg.contains("&amp;") && !svg.contains("&lt;") {
                    found.push(format!("{what}-svg\tnothing was escaped"));
                }
            }
        }
    }

    // Rust: names become identifiers, docs become `///`, and a doc that
    // ends its own comment or an identifier with a quote in it does not
    match sysml_rust::generate(ws.model(), &roots) {
        Err(e) => found.push(format!("rustgen\trefused: {e}")),
        Ok(generated) => {
            let rust = generated.rust;
            std::fs::write(std::env::temp_dir().join("awkward.rs"), &rust).ok();
            if let Err(e) = compiles(&rust, "awkward") {
                found.push(format!("rustgen\t{e}"));
            }
        }
    }

    // JSON and the formatter
    let json = sysml_interchange::to_json(ws.model());
    match sysml_interchange::from_json(&json) {
        Err(e) => found.push(format!("interchange\t{e:?}")),
        Ok((back, _)) => {
            if sysml_interchange::to_json(&back) != json {
                found.push("interchange\tdrifted".to_string());
            }
        }
    }
    let once = sysml_syntax::fmt::format(AWKWARD, sysml_syntax::Dialect::SysML);
    if !sysml_syntax::parse(&once).errors().is_empty() {
        found.push("fmt\tbroke the parse".to_string());
    }
    if once != sysml_syntax::fmt::format(&once, sysml_syntax::Dialect::SysML) {
        found.push("fmt\tnot idempotent".to_string());
    }

    eprintln!("--- {} findings", found.len());
    for line in &found {
        eprintln!("{line}");
    }
    assert!(found.is_empty());
}

/// Model JSON as it arrives from somewhere else: a field short, a field
/// of the wrong type, a reference to nothing.
#[test]
fn json_from_elsewhere_is_refused_not_believed() {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file("scalars.kerml", SCALARS);
    ws.add_file("awkward.sysml", AWKWARD);
    ws.resolve_all();
    let json = sysml_interchange::to_json(ws.model());
    let text = json.to_string();
    let mut found = Vec::new();

    // every prefix of it, and the whole with one byte changed
    for step in 0..64 {
        let cut = text.len() * step / 64;
        let cut = (cut..=text.len())
            .find(|&i| text.is_char_boundary(i))
            .unwrap_or(text.len());
        let piece = &text[..cut];
        if catch_unwind(AssertUnwindSafe(|| {
            serde_json::from_str::<Json>(piece)
                .ok()
                .map(|v| sysml_interchange::from_json(&v))
        }))
        .is_err()
        {
            found.push(format!("PANIC-cut\t{cut}"));
        }
    }

    // whole elements mangled
    let mangle = |how: &str, edit: &dyn Fn(&mut Json)| -> Option<String> {
        let mut copy = json.clone();
        edit(&mut copy);
        match catch_unwind(AssertUnwindSafe(|| sysml_interchange::from_json(&copy))) {
            Err(_) => Some(format!("PANIC-{how}")),
            Ok(_) => None,
        }
    };
    /// One way of mangling the JSON, by name.
    type Edit = (&'static str, Box<dyn Fn(&mut Json)>);
    let edits: Vec<Edit> = vec![
        (
            "empty-object",
            Box::new(|j: &mut Json| *j = serde_json::json!({})),
        ),
        (
            "empty-array",
            Box::new(|j: &mut Json| *j = serde_json::json!([])),
        ),
        (
            "a-number",
            Box::new(|j: &mut Json| *j = serde_json::json!(7)),
        ),
        ("null", Box::new(|j: &mut Json| *j = Json::Null)),
        (
            "no-type",
            Box::new(|j: &mut Json| {
                if let Some(first) = j.as_array_mut().and_then(|a| a.first_mut()) {
                    first.as_object_mut().map(|o| o.remove("@type"));
                }
            }),
        ),
        (
            "no-id",
            Box::new(|j: &mut Json| {
                if let Some(first) = j.as_array_mut().and_then(|a| a.first_mut()) {
                    first.as_object_mut().map(|o| o.remove("@id"));
                }
            }),
        ),
        (
            "unknown-type",
            Box::new(|j: &mut Json| {
                if let Some(first) = j.as_array_mut().and_then(|a| a.first_mut()) {
                    first["@type"] = serde_json::json!("NoSuchMetaclass");
                }
            }),
        ),
        (
            "dangling-ref",
            Box::new(|j: &mut Json| {
                if let Some(first) = j.as_array_mut().and_then(|a| a.first_mut()) {
                    first["owner"] = serde_json::json!({ "@id": "nowhere" });
                }
            }),
        ),
        (
            "self-owned",
            Box::new(|j: &mut Json| {
                if let Some(first) = j.as_array_mut().and_then(|a| a.first_mut()) {
                    let id = first["@id"].clone();
                    first["owner"] = serde_json::json!({ "@id": id });
                }
            }),
        ),
        (
            "id-is-a-number",
            Box::new(|j: &mut Json| {
                if let Some(first) = j.as_array_mut().and_then(|a| a.first_mut()) {
                    first["@id"] = serde_json::json!(3);
                }
            }),
        ),
        (
            "duplicate-ids",
            Box::new(|j: &mut Json| {
                if let Some(array) = j.as_array_mut() {
                    if array.len() > 1 {
                        let id = array[0]["@id"].clone();
                        array[1]["@id"] = id;
                    }
                }
            }),
        ),
    ];
    for (how, edit) in &edits {
        if let Some(line) = mangle(how, edit) {
            found.push(line);
        }
    }

    eprintln!("--- {} findings", found.len());
    for line in &found {
        eprintln!("{line}");
    }
    assert!(found.is_empty());
}

/// Parse, resolve, render and generate for `text`, on a thread with a
/// stack the size a normal program gives `main`, so that a depth this
/// survives here is a depth a caller survives too.
fn all_of_it(what: &str, text: String) -> Result<(), String> {
    let handle = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let mut ws = sysml_semantics::Workspace::new();
            let file = ws.add_file("deep.sysml", &text);
            let parsed = ws.file_parse(file).syntax().text().to_string() == text;
            ws.resolve_files(&[file]);
            let roots = ws.file_roots(file).to_vec();
            let style = sysml_diagram::Style::default();
            let svg = sysml_diagram::render(
                &sysml_diagram::definition_diagram(ws.model(), &roots),
                &style,
            );
            let browser = sysml_diagram::render_browser(
                &sysml_diagram::browser_view(ws.model(), &roots),
                &style,
            );
            let json = sysml_interchange::to_json(ws.model());
            let back = sysml_interchange::from_json(&json).is_ok();
            let rust = sysml_rust::generate(ws.model(), &roots).is_ok();
            let formatted = sysml_syntax::fmt::format(&text, sysml_syntax::Dialect::SysML);
            (
                parsed,
                svg.len(),
                browser.len(),
                back,
                rust,
                formatted.len(),
            )
        })
        .map_err(|e| e.to_string())?;
    match handle.join() {
        Ok((parsed, ..)) if !parsed => Err(format!("{what}: the tree did not cover the text")),
        Ok(_) => Ok(()),
        Err(_) => Err(format!("{what}: died")),
    }
}

#[test]
fn nothing_nested_deeply_takes_the_process_with_it() {
    let mut found = Vec::new();
    for depth in [10, 100, 1_000] {
        let shapes: Vec<(String, String)> = vec![
            (
                format!("packages-{depth}"),
                format!("{}{}", "package P {\n".repeat(depth), "}\n".repeat(depth)),
            ),
            (
                format!("parts-{depth}"),
                format!(
                    "part def A {{\n{}{}}}\n",
                    "part p : A {\n".repeat(depth),
                    "}\n".repeat(depth)
                ),
            ),
            (
                format!("parens-{depth}"),
                format!(
                    "package P {{ attribute x = {}1{}; }}\n",
                    "(".repeat(depth),
                    ")".repeat(depth)
                ),
            ),
            (
                format!("plus-{depth}"),
                format!("package P {{ attribute x = 1{}; }}\n", " + 1".repeat(depth)),
            ),
            (
                format!("dotted-{depth}"),
                format!(
                    "package P {{ part a : {}; }}\n",
                    vec!["N"; depth].join("::")
                ),
            ),
            (format!("unclosed-{depth}"), "package P {\n".repeat(depth)),
        ];
        for (what, text) in shapes {
            if let Err(e) = all_of_it(&what, text) {
                found.push(e);
            }
        }
    }
    eprintln!("--- {} findings", found.len());
    for line in &found {
        eprintln!("{line}");
    }
    assert!(found.is_empty());
}

fn compiles(rust: &str, name: &str) -> Result<(), String> {
    let out = std::env::temp_dir().join(format!("sysml-probe-{name}"));
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let at = out.join("lib.rs");
    std::fs::write(&at, rust).map_err(|e| e.to_string())?;
    let rustc = std::process::Command::new(std::env::var("RUSTC").unwrap_or("rustc".into()))
        .args([
            "--crate-type",
            "lib",
            "--edition",
            "2021",
            "--emit=metadata",
        ])
        .arg("-o")
        .arg(out.join("meta.rmeta"))
        .arg(&at)
        .output()
        .map_err(|e| e.to_string())?;
    if rustc.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&rustc.stderr);
    Err(said
        .lines()
        .find(|line| line.starts_with("error"))
        .unwrap_or("(no error line)")
        .to_string())
}

fn well_formed(svg: &str) -> Result<(), String> {
    let mut stack: Vec<String> = Vec::new();
    let mut at = 0;
    while let Some(open) = svg[at..].find('<') {
        let start = at + open;
        let Some(close) = svg[start..].find('>') else {
            return Err("unclosed tag".into());
        };
        let end = start + close;
        let inner = &svg[start + 1..end];
        at = end + 1;
        if inner.starts_with('?') || inner.starts_with('!') {
            continue;
        }
        if let Some(rest) = inner.strip_prefix('/') {
            let tag = rest.trim();
            match stack.pop() {
                Some(open) if open == tag => {}
                other => return Err(format!("</{tag}> closes {other:?}")),
            }
        } else if !inner.ends_with('/') {
            stack.push(
                inner
                    .split([' ', '\n', '\t'])
                    .next()
                    .unwrap_or_default()
                    .to_string(),
            );
        }
    }
    if stack.is_empty() {
        Ok(())
    } else {
        Err(format!("left open: {stack:?}"))
    }
}

//! Every tool the MCP server offers, over the whole corpus.
//!
//! An agent is not careful about arguments: it asks for a line past the
//! end of the file, a column in the middle of a character, a limit of a
//! million, a list where a string belongs. The server answers a badly
//! formed call with an error, and this holds it to that -- it may not
//! panic, because a panic takes the session with it and the agent
//! cannot tell that from the toolchain having no answer.
use serde_json::{json, Value};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

fn vendor() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/sysml-v2-release")
        .canonicalize()
        .ok()?;
    root.join("sysml.library").is_dir().then_some(root)
}

fn files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.join("sysml/src"), root.join("kerml/src")];
    while let Some(at) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&at) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("sysml" | "kerml")
            ) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[test]
#[ignore = "two minutes of tool calls; run with --ignored"]
fn every_tool_over_the_corpus() {
    let Some(root) = vendor() else { return };
    let mut server = sysml_mcp::Server::new(Some(&root.join("sysml.library")));
    let mut found: Vec<String> = Vec::new();
    let all = files(&root);
    eprintln!("{} files", all.len());

    let mut id = 0;
    let mut ask = |server: &mut sysml_mcp::Server, name: &str, arguments: Value| -> Option<Value> {
        id += 1;
        let request = json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        });
        catch_unwind(AssertUnwindSafe(|| server.handle(&request)))
            .ok()
            .flatten()
    };

    for path in &all {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let at = || path.display().to_string();
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        match ask(&mut server, "check", json!({"text": text, "name": name})) {
            None => found.push(format!("PANIC-check\t{}", at())),
            Some(answer) => {
                if answer.get("error").is_some() {
                    found.push(format!("check-errored\t{}\t{}", at(), answer["error"]));
                }
            }
        }

        // a name asked for at every line, including past the end
        let lines = text.lines().count();
        let step = (lines / 12).max(1);
        for line in (1..=lines + 2).step_by(step) {
            for column in [1, 5, 200] {
                let asked = ask(
                    &mut server,
                    "visible_names",
                    json!({"text": text, "name": name, "line": line, "column": column}),
                );
                match asked {
                    None => found.push(format!("PANIC-visible\t{}:{line}:{column}", at())),
                    Some(answer) if answer.get("error").is_some() => found.push(format!(
                        "visible-errored\t{}:{line}:{column}\t{}",
                        at(),
                        answer["error"]
                    )),
                    Some(_) => {}
                }
            }
        }
    }

    // arguments a caller can get wrong, and searches that match nothing
    for (what, arguments) in [
        ("no text", json!({})),
        ("empty text", json!({"text": ""})),
        ("wrong type", json!({"text": 7})),
        ("nul", json!({"text": "\0"})),
        ("unclosed", json!({"text": "part def A {"})),
        (
            "alongside missing",
            json!({"text": "part def A;", "alongside": ["/nope.sysml"]}),
        ),
        (
            "alongside wrong",
            json!({"text": "part def A;", "alongside": "not a list"}),
        ),
        ("path missing", json!({"path": "/no/such/file.sysml"})),
    ] {
        if ask(&mut server, "check", arguments).is_none() {
            found.push(format!("PANIC-check-arg\t{what}"));
        }
    }
    for (what, arguments) in [
        ("no position", json!({"text": "part def A;"})),
        (
            "zero",
            json!({"text": "part def A;", "line": 0, "column": 0}),
        ),
        (
            "negative",
            json!({"text": "part def A;", "line": -3, "column": -1}),
        ),
        (
            "huge",
            json!({"text": "part def A;", "line": 9_999_999, "column": 9_999_999}),
        ),
        (
            "float",
            json!({"text": "part def A;", "line": 1.5, "column": 2.5}),
        ),
        (
            "mid-character",
            json!({"text": "part def 'あいう';", "line": 1, "column": 11}),
        ),
    ] {
        if ask(&mut server, "visible_names", arguments).is_none() {
            found.push(format!("PANIC-visible-arg\t{what}"));
        }
    }
    for (what, arguments) in [
        ("no query", json!({})),
        ("empty", json!({"query": ""})),
        ("wide", json!({"query": "あ"})),
        ("regex-ish", json!({"query": ".*"})),
        ("huge limit", json!({"query": "a", "limit": 1_000_000})),
        ("negative limit", json!({"query": "a", "limit": -5})),
        ("wrong type", json!({"query": ["a"]})),
    ] {
        if ask(&mut server, "library_search", arguments).is_none() {
            found.push(format!("PANIC-search-arg\t{what}"));
        }
    }
    // protocol-level shapes
    for (what, request) in [
        ("no method", json!({"jsonrpc": "2.0", "id": 1})),
        (
            "unknown method",
            json!({"jsonrpc":"2.0","id":1,"method":"nope"}),
        ),
        (
            "unknown tool",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"nope","arguments":{}}}),
        ),
        (
            "no params",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call"}),
        ),
        ("not an object", json!([1, 2, 3])),
        (
            "notification",
            json!({"jsonrpc":"2.0","method":"initialized"}),
        ),
    ] {
        if catch_unwind(AssertUnwindSafe(|| server.handle(&request))).is_err() {
            found.push(format!("PANIC-protocol\t{what}"));
        }
    }

    eprintln!("--- {} findings", found.len());
    for line in found.iter().take(30) {
        eprintln!("{line}");
    }
    assert!(found.is_empty());
}

//! The language server asked about the corpus after it has been broken.
//!
//! `corpus_positions` asks about every kind of place in models that are
//! well-formed by construction. An editor almost never holds one of
//! those: what it holds is a model halfway through being typed -- a
//! brace not yet closed, a name cut off, a line pasted in the wrong
//! place. So each model here is broken a few ways at once, sent to the
//! server as an edit, and asked everything the server answers, at
//! places chosen to land on the damage as often as anywhere else.
//!
//! A panic anywhere under a request is what this looks for. The server
//! survives one -- it answers that request with an internal error --
//! which is what lets one run find more than the first.
//!
//! The breakage is a fixed function of a seed, so a failure says how to
//! get it back: the seed, and the broken text, written under the test's
//! target directory. `SYSML_FUZZ_ROUNDS` asks for more rounds per model
//! than the two CI can afford, and `SYSML_FUZZ_SEED` for different ones.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use lsp_server::{Message, Notification, Request, RequestId};
use serde_json::{json, Value};
use sysml_corpus::{models, vendor};
use sysml_lsp::{Files, Session};

/// SplitMix64: the whole of the randomness, so that a seed is the whole
/// of a failure's recipe.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// A number in `0..n`, or 0 where there is nothing to choose from.
    fn below(&mut self, n: usize) -> usize {
        match n {
            0 => 0,
            _ => (self.next() % n as u64) as usize,
        }
    }
    fn pick<'a>(&mut self, from: &[&'a str]) -> &'a str {
        from[self.below(from.len())]
    }
}

/// What a model is made of, and what an editor is likely to have half
/// of: the notation's punctuation and keywords, and characters that
/// are more than one byte or more than one UTF-16 unit.
const PIECES: &[&str] = &[
    "{",
    "}",
    ";",
    ":",
    ":>",
    ":>>",
    "::",
    "=",
    "[",
    "]",
    "[*]",
    "(",
    ")",
    ",",
    ".",
    "\"",
    "'",
    "/*",
    "*/",
    "//",
    "doc",
    "part",
    "def",
    "part def",
    "attribute",
    "port",
    "item",
    "action",
    "state",
    "transition",
    "then",
    "first",
    "accept",
    "import",
    "private import",
    "::*",
    "package",
    "in",
    "out",
    "inout",
    "ref",
    "abstract",
    "specializes",
    "subsets",
    "redefines",
    "connect",
    "to",
    "bind",
    "flow",
    "from",
    "if",
    "else",
    "calc",
    "return",
    "constraint",
    "require",
    "satisfy",
    "verify",
    "metadata",
    "#",
    "@",
    "~",
    "<",
    ">",
    "<'x'>",
    "1..*",
    "0",
    "-",
    "+",
    "*",
    "/",
    "**",
    "^",
    "1.5e3",
    "true",
    "null",
    "é",
    "日本",
    "😀",
    "\u{0}",
    "\r",
    "\t",
    "\n",
    "\r\n",
    "\u{feff}",
    "\u{200b}",
];

/// `text` broken one way, as characters rather than bytes so that every
/// cut lands between two of them.
fn break_once(rng: &mut Rng, text: &mut Vec<char>) {
    let at = rng.below(text.len() + 1);
    match rng.below(7) {
        // stopped typing here
        0 => text.truncate(at),
        // a selection deleted
        1 => {
            let end = (at + 1 + rng.below(40)).min(text.len());
            text.drain(at.min(end)..end);
        }
        // a selection pasted somewhere else
        2 => {
            let end = (at + 1 + rng.below(80)).min(text.len());
            let copied: Vec<char> = text[at.min(end)..end].to_vec();
            let to = rng.below(text.len() + 1);
            text.splice(to..to, copied);
        }
        // a piece typed in
        3 => {
            let piece = rng.pick(PIECES);
            text.splice(at..at, piece.chars());
        }
        // a character typed over
        4 if at < text.len() => {
            let piece = rng.pick(PIECES);
            text.splice(at..at + 1, piece.chars());
        }
        // two lines swapped
        5 => {
            let joined: String = text.iter().collect();
            let mut lines: Vec<&str> = joined.split('\n').collect();
            let (a, b) = (rng.below(lines.len()), rng.below(lines.len()));
            lines.swap(a, b);
            *text = lines.join("\n").chars().collect();
        }
        // a line duplicated, as a hasty copy does
        _ => {
            let joined: String = text.iter().collect();
            let mut lines: Vec<&str> = joined.split('\n').collect();
            let line = rng.below(lines.len());
            lines.insert(line, lines[line]);
            *text = lines.join("\n").chars().collect();
        }
    }
}

/// Where the last panic was, and what it said -- the server's own answer
/// carries only the second, and two panics with the same words in
/// different places are different faults.
fn last_panic() -> &'static Mutex<Option<String>> {
    static LAST: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(None))
}

fn request(id: i32, method: &str, params: Value) -> Message {
    Message::Request(Request {
        id: RequestId::from(id),
        method: method.into(),
        params,
    })
}

fn notification(method: &str, params: Value) -> Message {
    Message::Notification(Notification {
        method: method.into(),
        params,
    })
}

/// A failure the server owned up to, if `message` is one: an internal
/// error answering a request, or one logged for a notification.
fn failure(message: &Message) -> Option<String> {
    let said = serde_json::to_value(message).expect("a message is JSON");
    if said["error"]["code"] == lsp_server::ErrorCode::InternalError as i32 {
        return Some(said["error"]["message"].as_str()?.to_string());
    }
    let logged = said["params"]["message"].as_str()?;
    (said["method"] == "window/logMessage" && logged.contains("failed inside the server"))
        .then(|| logged.to_string())
}

#[test]
fn the_corpus_broken_while_it_is_typed() {
    let Some(root) = vendor() else { return };
    let rounds: u64 = std::env::var("SYSML_FUZZ_ROUNDS")
        .ok()
        .and_then(|it| it.parse().ok())
        .unwrap_or(2);
    let base: u64 = std::env::var("SYSML_FUZZ_SEED")
        .ok()
        .and_then(|it| it.parse().ok())
        .unwrap_or(0);

    // Recorded rather than printed: every panic would otherwise put a
    // backtrace in the log, and the report at the end says it once.
    std::panic::set_hook(Box::new(|info| {
        let place = info
            .location()
            .map(|at| format!("{}:{}", at.file(), at.line()))
            .unwrap_or_default();
        let said = info
            .payload()
            .downcast_ref::<&str>()
            .map(|it| it.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        *last_panic().lock().unwrap() = Some(format!("{place}: {said}"));
    }));

    let mut session = Session::new(Files::handed());
    let mut out = Vec::new();
    session.handle(
        request(1, "initialize", json!({ "capabilities": {} })),
        &mut out,
    );
    session.handle(notification("initialized", json!({})), &mut out);
    out.clear();

    let findings_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("mutated-corpus");
    // a fault, by where it panicked, with the first input that reached it
    let mut found: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut asked = 0usize;
    let mut id = 2;
    let all = models(&root);
    for (n, path) in all.iter().enumerate() {
        let Ok(original) = std::fs::read_to_string(path) else {
            continue;
        };
        let trace = std::env::var_os("SYSML_FUZZ_TRACE").is_some();
        if trace {
            eprintln!("{n} {}", path.display());
        }
        // one document, edited round after round: the way an editor's
        // buffer goes from one broken state to the next
        let uri = format!("file:///fuzz/{n}.sysml");
        session.handle(
            notification(
                "textDocument/didOpen",
                json!({"textDocument":{"uri":&uri,"languageId":"sysml","version":0,"text":&original}}),
            ),
            &mut out,
        );
        for round in 0..rounds {
            let seed = base
                .wrapping_mul(0x1_0000_0001)
                .wrapping_add((n as u64) << 20 | round);
            let mut rng = Rng(seed);
            let mut text: Vec<char> = original.chars().collect();
            for _ in 0..1 + rng.below(4) {
                break_once(&mut rng, &mut text);
            }
            let text: String = text.into_iter().collect();
            let lines: Vec<&str> = text.split('\n').collect();

            let mut asks = vec![
                notification(
                    "textDocument/didChange",
                    json!({"textDocument":{"uri":&uri,"version":round + 1},
                           "contentChanges":[{"text":&text}]}),
                ),
                request(
                    0,
                    "textDocument/documentSymbol",
                    json!({"textDocument":{"uri":&uri}}),
                ),
                request(
                    0,
                    "textDocument/formatting",
                    json!({"textDocument":{"uri":&uri},"options":{"tabSize":4,"insertSpaces":true}}),
                ),
                request(
                    0,
                    "workspace/symbol",
                    json!({"query": rng.pick(&["", "a", "Part", "é"])}),
                ),
            ];
            for view in ["definitions", "internal", "browser"] {
                asks.push(request(0, "sysml/diagram", json!({"uri":&uri,"view":view})));
            }
            for _ in 0..4 {
                // a line past the end as well as on one, and a column
                // past the end of the line as often as inside it
                let line = rng.below(lines.len() + 2);
                let wide = lines.get(line).map_or(0, |it| it.encode_utf16().count());
                let character = rng.below(wide + 3);
                let at = json!({"line":line,"character":character});
                let doc = json!({"uri":&uri});
                for (method, params) in [
                    (
                        "textDocument/hover",
                        json!({"textDocument":doc,"position":at}),
                    ),
                    (
                        "textDocument/definition",
                        json!({"textDocument":doc,"position":at}),
                    ),
                    (
                        "textDocument/references",
                        json!({"textDocument":doc,"position":at,"context":{"includeDeclaration":true}}),
                    ),
                    (
                        "textDocument/completion",
                        json!({"textDocument":doc,"position":at}),
                    ),
                    (
                        "textDocument/signatureHelp",
                        json!({"textDocument":doc,"position":at}),
                    ),
                    (
                        "textDocument/rename",
                        json!({"textDocument":doc,"position":at,"newName":"renamed"}),
                    ),
                    (
                        "textDocument/codeAction",
                        json!({"textDocument":doc,"range":{"start":at,"end":at},
                               "context":{"diagnostics":[]}}),
                    ),
                ] {
                    asks.push(request(0, method, params));
                }
            }

            if trace {
                std::fs::create_dir_all(&findings_dir).unwrap();
                std::fs::write(findings_dir.join(format!("seed-{seed}.sysml")), &text).unwrap();
            }
            let started = std::time::Instant::now();
            for mut ask in asks {
                if let Message::Request(req) = &mut ask {
                    req.id = RequestId::from(id);
                    id += 1;
                    asked += 1;
                }
                if trace {
                    let (Message::Request(Request { method, .. })
                    | Message::Notification(Notification { method, .. })) = &ask
                    else {
                        unreachable!()
                    };
                    let params = match &ask {
                        Message::Request(req) => req.params["position"].to_string(),
                        _ => String::new(),
                    };
                    eprintln!("  seed {seed}: {method} {params}");
                }
                session.handle(ask, &mut out);
                for answer in out.drain(..) {
                    let Some(said) = failure(&answer) else {
                        continue;
                    };
                    let place = last_panic().lock().unwrap().take().unwrap_or(said.clone());
                    found.entry(place).or_insert_with(|| {
                        std::fs::create_dir_all(&findings_dir).unwrap();
                        let kept = findings_dir.join(format!("seed-{seed}.sysml"));
                        std::fs::write(&kept, &text).unwrap();
                        (
                            format!(
                                "{} (seed {seed}), kept at {}",
                                path.display(),
                                kept.display()
                            ),
                            said,
                        )
                    });
                }
            }
            let took = started.elapsed();
            if took.as_secs_f64() > 2.0 {
                eprintln!("slow: {} (seed {seed}) took {took:?}", path.display());
            }
        }
        session.handle(
            notification(
                "textDocument/didClose",
                json!({"textDocument":{"uri":&uri}}),
            ),
            &mut out,
        );
        out.clear();
    }
    let _ = std::panic::take_hook();
    eprintln!(
        "{} models, {rounds} rounds each, {asked} requests",
        all.len()
    );

    let report: Vec<String> = found
        .iter()
        .map(|(place, (input, said))| format!("{place}\n    {said}\n    from {input}"))
        .collect();
    assert!(
        report.is_empty(),
        "{} distinct panic(s) under broken input:\n{}",
        report.len(),
        report.join("\n")
    );
}

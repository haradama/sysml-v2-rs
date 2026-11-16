//! A reader that stops reading.
//!
//! `sysml export m.sysml | head` closes the pipe after a screenful, and
//! `print!` panics on the rest -- exit 101 and a backtrace hint, for what
//! the reader asked for. Every command writes to stdout through one
//! place, which stops quietly; these run a few commands to show it.

use std::io::Write as _;
use std::process::{Command, Stdio};

/// A model whose formatted text and export are each far more than a
/// pipe holds, so the writing still has a long way to go when it finds
/// the reader gone.
fn big_model(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("big.sysml");
    let mut file = std::fs::File::create(&path).expect("a scratch file");
    writeln!(file, "package Big {{").unwrap();
    for i in 0..3000 {
        writeln!(file, "\tpart def P{i} {{ attribute a{i}; }}").unwrap();
    }
    writeln!(file, "}}").unwrap();
    path
}

/// Run `sysml args` with its stdout closed before it writes, and say
/// how it ended and what it said on stderr.
fn into_a_closed_pipe(args: &[&str]) -> (Option<i32>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sysml"))
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary runs");
    drop(child.stdout.take());
    let out = child.wait_with_output().expect("it ends");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn a_closed_pipe_ends_the_output_quietly() {
    let dir = std::env::temp_dir().join(format!("sysml-closed-pipe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let model = big_model(&dir);
    let model = model.to_str().unwrap();
    for args in [
        vec!["--no-library", "export", model],
        vec!["--no-library", "fmt", model],
        vec!["--no-library", "--format", "json", "plan", model],
        vec!["plan", "--schema"],
    ] {
        let (code, stderr) = into_a_closed_pipe(&args);
        assert!(!stderr.contains("panicked"), "{args:?} panicked: {stderr}");
        assert_eq!(code, Some(0), "{args:?}: {stderr}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

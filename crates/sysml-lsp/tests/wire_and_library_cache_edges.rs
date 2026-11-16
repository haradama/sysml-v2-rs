//! The edges of the wire (frames that end early or never say how long
//! they are, a surrogate pair written as escapes) and of the cache the
//! built-in library is written to (a cache that cannot be written).
//!
//! Each case runs the real binary, with a cache directory of its own.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

fn frame(body: &str) -> String {
    format!("Content-Length: {}\r\n\r\n{body}", body.len())
}

fn read_message(reader: &mut impl BufRead) -> String {
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length: ") {
            length = value.parse().unwrap();
        }
    }
    let mut body = vec![0u8; length];
    std::io::Read::read_exact(reader, &mut body).unwrap();
    String::from_utf8(body).unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sysml-lsp-edges-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn spawn(cache: &Path, options: &str) -> (Child, ChildStdin, BufReader<ChildStdout>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sysml-lsp"))
        .env("XDG_CACHE_HOME", cache)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let init = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"capabilities":{{}},"initializationOptions":{options}}}}}"#
    );
    stdin.write_all(frame(&init).as_bytes()).unwrap();
    assert!(read_message(&mut stdout).contains("capabilities"));
    stdin
        .write_all(frame(r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#).as_bytes())
        .unwrap();
    (child, stdin, stdout)
}

fn finish(mut child: Child, mut stdin: ChildStdin, stdout: &mut BufReader<ChildStdout>) {
    stdin
        .write_all(
            frame(r#"{"jsonrpc":"2.0","id":9,"method":"shutdown","params":null}"#).as_bytes(),
        )
        .unwrap();
    assert!(read_message(stdout).contains("\"id\":9"));
    stdin
        .write_all(frame(r#"{"jsonrpc":"2.0","method":"exit","params":null}"#).as_bytes())
        .unwrap();
    drop(stdin);
    assert!(child.wait().unwrap().success());
}

fn exits_soon(child: &mut Child) -> bool {
    let until = Instant::now() + Duration::from_secs(20);
    while Instant::now() < until {
        if child.try_wait().unwrap().is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// A part declared and typed by a name from the built-in library.
const OPEN: &str = r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"untitled:a","languageId":"sysml","version":1,"text":"part def B;\npart a : B;\nattribute n : ScalarValues::Integer;\n"}}}"#;

fn opens_cleanly(stdin: &mut ChildStdin, stdout: &mut BufReader<ChildStdout>) {
    stdin.write_all(frame(OPEN).as_bytes()).unwrap();
    let published = read_message(stdout);
    assert!(
        published.contains("publishDiagnostics") && published.contains("\"diagnostics\":[]"),
        "the library still resolves: {published}"
    );
}

/// Where the library is written: as the server computes it.
fn library_dir(cache: &Path) -> PathBuf {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (name, text) in sysml_stdlib::FILES {
        for byte in name.bytes().chain([0]).chain(text.bytes()).chain([0]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    cache
        .join("sysml-lsp")
        .join(format!("library-{}-{hash:016x}", sysml_stdlib::RELEASE))
}

#[test]
fn a_second_server_finds_the_library_already_written() {
    let cache = scratch("again");
    for _ in 0..2 {
        let (child, mut stdin, mut stdout) = spawn(&cache, "{}");
        opens_cleanly(&mut stdin, &mut stdout);
        finish(child, stdin, &mut stdout);
    }
    let (name, text) = sysml_stdlib::FILES[0];
    assert_eq!(
        std::fs::read(library_dir(&cache).join(name)).unwrap(),
        text.as_bytes()
    );
}

#[test]
fn a_cache_that_is_no_directory_costs_the_jump_but_not_the_names() {
    let cache = scratch("notdir");
    let file = cache.join("a-file");
    std::fs::write(&file, "x").unwrap();
    let (child, mut stdin, mut stdout) = spawn(&file, "{}");
    opens_cleanly(&mut stdin, &mut stdout);
    finish(child, stdin, &mut stdout);
    assert!(file.is_file());
}

#[test]
fn a_cache_directory_that_cannot_be_written_to_costs_the_jump_but_not_the_names() {
    use std::os::unix::fs::PermissionsExt;
    let cache = scratch("readonly");
    let (name, _) = sysml_stdlib::FILES[0];
    let parent = library_dir(&cache)
        .join(name)
        .parent()
        .unwrap()
        .to_path_buf();
    std::fs::create_dir_all(&parent).unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o555)).unwrap();
    let writable = std::fs::write(parent.join("probe"), "x").is_ok();
    let (child, mut stdin, mut stdout) = spawn(&cache, "{}");
    opens_cleanly(&mut stdin, &mut stdout);
    finish(child, stdin, &mut stdout);
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();
    // (running as a user the mode does not bind, there is nothing to refuse)
    if !writable {
        assert!(!parent.join(Path::new(name).file_name().unwrap()).exists());
    }
}

#[test]
fn a_library_file_that_is_a_directory_cannot_be_replaced_and_is_not_written_over() {
    let cache = scratch("isdir");
    let (name, _) = sysml_stdlib::FILES[0];
    let at = library_dir(&cache).join(name);
    std::fs::create_dir_all(&at).unwrap();
    let (child, mut stdin, mut stdout) = spawn(&cache, "{}");
    opens_cleanly(&mut stdin, &mut stdout);
    finish(child, stdin, &mut stdout);
    assert!(at.is_dir());
    // the partial file is not left behind
    let left = std::fs::read_dir(at.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|it| it.file_name().to_string_lossy().contains("partial"))
        .count();
    assert_eq!(left, 0);
}

#[test]
fn a_pair_of_surrogates_written_as_escapes_is_one_character() {
    let cache = scratch("pair");
    let (child, mut stdin, mut stdout) = spawn(&cache, r#"{"noLibrary":true}"#);
    stdin
        .write_all(
            frame(
                r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"untitled:a","languageId":"sysml","version":1,"text":"// \ud83d\ude00\npart def B;\npart a : B;\n"}}}"#,
            )
            .as_bytes(),
        )
        .unwrap();
    let published = read_message(&mut stdout);
    assert!(
        published.contains("publishDiagnostics") && published.contains("\"diagnostics\":[]"),
        "{published}"
    );
    finish(child, stdin, &mut stdout);
}

#[test]
fn input_that_ends_between_messages_ends_the_server_without_a_word() {
    let cache = scratch("eof");
    let (mut child, stdin, mut stdout) = spawn(&cache, r#"{"noLibrary":true}"#);
    drop(stdin);
    assert!(exits_soon(&mut child));
    let mut rest = String::new();
    std::io::Read::read_to_string(&mut stdout, &mut rest).unwrap();
    assert_eq!(rest, "");
}

#[test]
fn a_frame_with_no_length_ends_the_server() {
    for header in ["Content-Type: x\r\n\r\n", "Content-Length: many\r\n\r\n"] {
        let cache = scratch("nolength");
        let (mut child, mut stdin, _stdout) = spawn(&cache, r#"{"noLibrary":true}"#);
        stdin.write_all(header.as_bytes()).unwrap();
        // the input is still open, and the server has stopped reading it
        assert!(exits_soon(&mut child), "{header:?}");
    }
}

#[test]
fn a_header_line_with_no_colon_is_skipped_and_the_frame_is_still_read() {
    let cache = scratch("colonless");
    let (child, mut stdin, mut stdout) = spawn(&cache, r#"{"noLibrary":true}"#);
    let body = r#"{"jsonrpc":"2.0","id":5,"method":"shutdown","params":null}"#;
    let odd = format!(
        "not a header\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stdin.write_all(odd.as_bytes()).unwrap();
    assert!(read_message(&mut stdout).contains("\"id\":5"));
    stdin
        .write_all(frame(r#"{"jsonrpc":"2.0","method":"exit","params":null}"#).as_bytes())
        .unwrap();
    drop(stdin);
    let mut child = child;
    assert!(child.wait().unwrap().success());
}

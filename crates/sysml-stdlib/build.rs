//! Turn `library/` into a table the crate can hand out.
//!
//! The files are read at compile time with `include_str!` rather than at
//! run time, so a program that loads the standard library needs no
//! filesystem, no install step and no path to be told: it is in the
//! binary. That is what makes `cargo install sysml-cli` a tool that
//! resolves names rather than one that asks for a 2 GB checkout first.
//!
//! The table is written into `OUT_DIR` rather than checked in, so it
//! cannot drift from what is beside it. What *can* drift is `library/`
//! against the release it was copied from, and `tests/matches_vendor.rs`
//! is what holds those together.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo says where"))
        .join("library");
    println!("cargo:rerun-if-changed=library");

    let mut found = Vec::new();
    walk(&root, &mut found);
    assert!(!found.is_empty(), "{} holds no model files", root.display());

    // Sorted by the name the crate hands out rather than by the path it
    // was read from: those two differ where a directory name holds a
    // space, and it is the exposed name a caller sees an order in.
    let mut named: Vec<(String, String)> = found
        .iter()
        .map(|path| {
            let name = path
                .strip_prefix(&root)
                .expect("walked from the root")
                .to_str()
                .expect("the release names its files in UTF-8")
                .replace('\\', "/");
            let full = path.to_str().expect("a UTF-8 checkout").to_string();
            (name, full)
        })
        .collect();
    named.sort();

    let mut out = String::from(
        "/// Every file of the standard library: the path it is known by,\n\
         /// relative to the library's own root, and what it says.\n\
         ///\n\
         /// In one order however it was built, so that a finding placed in\n\
         /// the library is placed the same way twice.\n\
         pub const FILES: &[(&str, &str)] = &[\n",
    );
    for (name, full) in &named {
        // `include_str!` takes a path relative to the file it is written
        // in, and that file lives in OUT_DIR, so the absolute one is what
        // reaches back into the crate.
        writeln!(out, "    ({name:?}, include_str!({full:?})),").expect("a String cannot fail");
    }
    out.push_str("];\n");

    let to = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo says where")).join("files.rs");
    std::fs::write(&to, out).expect("OUT_DIR is writable");
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Only a directory that is one is entered: a link to a directory
        // is where a walk goes round in circles.
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            walk(&path, out);
        } else if matches!(
            path.extension().and_then(|it| it.to_str()),
            Some("sysml" | "kerml")
        ) {
            out.push(path);
        }
    }
}

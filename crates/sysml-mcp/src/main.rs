//! `sysml-mcp`: the MCP server over stdin and stdout.
//!
//! The standard library comes from `--library <dir>` or the
//! `SYSML_LIBRARY_PATH` environment variable, the same way the language
//! server finds it. Without one every reference into the library reads
//! as unresolved, so a client that has it should say where it is.

use std::io::{BufReader, Write};

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut library = std::env::var("SYSML_LIBRARY_PATH").ok();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--library" => library = args.next(),
            "--help" | "-h" => {
                let mut out = std::io::stdout();
                writeln!(out, "usage: sysml-mcp [--library <sysml.library>]")?;
                writeln!(out, "speaks the Model Context Protocol over stdio")?;
                return Ok(());
            }
            other => {
                eprintln!("sysml-mcp: unexpected argument `{other}`");
                std::process::exit(2);
            }
        }
    }

    let mut server = sysml_mcp::Server::new(library.as_deref().map(std::path::Path::new));
    sysml_mcp::serve(
        &mut server,
        BufReader::new(std::io::stdin()),
        std::io::stdout(),
    )
}

//! What the `sysml` tool does that something other than a person drives.
//!
//! The Model Context Protocol server is here rather than in a crate of
//! its own because it is a front end like the command line itself: both
//! load a workspace and report what is wrong with it, and everything
//! they have in common already lives in `sysml-semantics`. It is a
//! library as well as a subcommand so that its tests can drive it
//! directly, which is how a protocol is worth testing.

pub mod mcp;

/// A finding with its place in the text: the byte offset the model sees,
/// and the line and column an editor counts, both from one.
///
/// The command line and the MCP server place their findings the same
/// way, so a program that reads one reads the other. An offset past the
/// end of the text -- a parser complaining that the file stopped -- is
/// placed at the end rather than off it.
pub fn at(text: &str, offset: usize, mut value: serde_json::Value) -> serde_json::Value {
    let offset = offset.min(text.len());
    let (line, column) = sysml_syntax::line_col(text, offset);
    let map = value.as_object_mut().expect("built as an object");
    map.insert("offset".into(), offset.into());
    map.insert("line".into(), line.into());
    map.insert("column".into(), column.into());
    value
}

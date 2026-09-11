//! What the `sysml` tool does that something other than a person drives.
//!
//! The MCP server is here rather than in a crate of its own because it is
//! a front end like the command line: both load a workspace and report
//! what is wrong with it, and what they share already lives in
//! `sysml-semantics`. A library as well as a subcommand, so its tests can
//! drive it directly.

// Nothing here needs `unsafe`, and saying so is what keeps it that way.
#![forbid(unsafe_code)]
// Every public item carries a line saying what it is for. The two
// crates that do not turn this on are `sysml-syntax`, whose public
// surface is two hundred and seventy-nine syntax kinds whose names are
// the documentation, and `sysml-model`, whose is generated from the
// metamodel and would want the generator to write it.
#![warn(missing_docs)]
pub mod mcp;
pub mod notation;
pub mod plan;
pub mod report;

/// A finding with its place in the text: where it starts and where it
/// ends, as the byte offset the model sees and as the line and column an
/// editor counts, both from one.
///
/// The command line and the MCP server place findings the same way, so a
/// program that reads one reads the other. An offset past the end of the
/// text is placed at the end rather than off it.
///
/// The end is said because the finding has always known it: a reader given
/// a point has to lex the line again to see what the finding is about, and
/// gets it wrong wherever its idea of a name differs from this one's.
pub fn at(
    text: &str,
    range: sysml_syntax::TextRange,
    mut value: serde_json::Value,
) -> serde_json::Value {
    let offset = usize::from(range.start()).min(text.len());
    let end = usize::from(range.end()).clamp(offset, text.len());
    let (line, column) = sysml_syntax::line_col(text, offset);
    let (end_line, end_column) = sysml_syntax::line_col(text, end);
    let map = value.as_object_mut().expect("built as an object");
    map.insert("offset".into(), offset.into());
    map.insert("line".into(), line.into());
    map.insert("column".into(), column.into());
    map.insert("endOffset".into(), end.into());
    map.insert("endLine".into(), end_line.into());
    map.insert("endColumn".into(), end_column.into());
    value
}

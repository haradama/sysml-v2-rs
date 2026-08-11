//! What the `sysml` tool does that something other than a person drives.
//!
//! The Model Context Protocol server is here rather than in a crate of
//! its own because it is a front end like the command line itself: both
//! load a workspace and report what is wrong with it, and everything
//! they have in common already lives in `sysml-semantics`. It is a
//! library as well as a subcommand so that its tests can drive it
//! directly, which is how a protocol is worth testing.

pub mod mcp;

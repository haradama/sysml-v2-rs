//! The KerML and SysML v2 standard model libraries, as text a program can
//! load.
//!
//! Almost nothing in a SysML model resolves without these. `part def
//! Vehicle;` specializes `Parts::Part`, every feature subsets
//! `Base::things`, and a `calc` reaches into the Kernel Function Library
//! -- so a toolchain that cannot find the standard library reports every
//! name in every model as unresolved, which is a lot of noise for a
//! missing path.
//!
//! Until this crate the path had to be given, and getting one meant
//! cloning a repository whose history is two gigabytes to obtain one and
//! a third megabytes of model. Here they are instead, in the binary:
//!
//! ```
//! let mut ws = sysml_semantics::Workspace::new();
//! for (name, text) in sysml_stdlib::FILES {
//!     ws.add_file(*name, text);
//! }
//! ```
//!
//! # What this crate is, and is not
//!
//! It is data. It depends on nothing, parses nothing and knows nothing
//! about the rest of sysml-v2-rs -- which is what lets it be a
//! dependency of whatever wants it without dragging a toolchain along,
//! and what lets it carry its own licence.
//!
//! The files under `library/` are the OMG release's, unchanged, under
//! the Eclipse Public License 2.0. The rest of sysml-v2-rs is MIT or
//! Apache-2.0. They are separate crates so that which files are under
//! which licence is a question with a one-word answer. See `NOTICE`.

// Nothing here needs `unsafe`, and saying so is what keeps it that way.
#![forbid(unsafe_code)]
// Every public item carries a line saying what it is for.
#![warn(missing_docs)]

include!(concat!(env!("OUT_DIR"), "/files.rs"));

/// The release of the OMG SysML v2 pilot implementation these came from.
///
/// The standard library moves with the specification, so a model that
/// resolves against one release may not against the next. A tool that
/// says which it is holding is a tool whose answer can be reproduced --
/// which is why `sysml --version` prints this.
pub const RELEASE: &str = "2026-05";

#[cfg(test)]
mod tests {
    use super::*;

    /// The counts the release states, which is what tells a build that
    /// quietly read half the tree from one that read all of it.
    #[test]
    fn the_whole_library_is_here() {
        assert_eq!(FILES.len(), 94);
        assert!(FILES.iter().all(|(_, text)| !text.is_empty()));
    }

    /// The two files everything else leans on, so that a table built
    /// from the wrong directory fails here rather than as ten thousand
    /// unresolved names somewhere downstream.
    #[test]
    fn the_bases_every_model_reaches_are_among_them() {
        let named = |want: &str| FILES.iter().any(|(name, _)| name.ends_with(want));
        let first: Vec<&str> = FILES.iter().map(|it| it.0).take(5).collect();
        assert!(named("Base.kerml"), "{first:?}");
        assert!(named("Parts.sysml"));
    }

    /// Names are relative to the library's own root and separated the
    /// one way, so that what a finding is reported under reads the same
    /// on every platform.
    #[test]
    fn a_file_is_named_by_where_it_sits_in_the_library() {
        for (name, _) in FILES {
            assert!(!name.contains('\\'), "{name}");
            assert!(!name.starts_with('/'), "{name}");
            assert!(
                name.ends_with(".sysml") || name.ends_with(".kerml"),
                "{name}"
            );
        }
    }

    /// And they are in one order, whoever built them.
    #[test]
    fn they_come_in_a_stable_order() {
        let mut sorted: Vec<&str> = FILES.iter().map(|(name, _)| *name).collect();
        let given = sorted.clone();
        sorted.sort();
        assert_eq!(given, sorted);
    }
}

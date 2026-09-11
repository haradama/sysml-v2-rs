//! Every corpus file resolved on its own, without the standard library.
//!
//! This is what an editor does before anyone points it at the library, and
//! what `sysml check <file>` does. Almost nothing resolves, so it
//! exercises the path the other corpus tests never take: the one where the
//! answer is no. A search that does not remember its failures re-runs
//! itself once per reference, and a file with ten unresolved wildcard
//! imports took thirty-three seconds before it did.

use sysml_corpus::{models, vendor};

/// What the resolver may do per reference before something is wrong.
///
/// Answering one reference may cost a look at each of the file's imports,
/// so this tracks how many imports a file has, not how many names it uses:
/// the worst of the 309 is thirteen. What must not happen is the cost
/// growing with the references as well, which is what an unremembered
/// failure does -- fifty is the alarm going off, not a budget to spend.
///
/// Counting the work rather than the seconds is deliberate: under coverage
/// instrumentation everything here runs fourteen times slower, so a
/// stopwatch would sleep through the next regression.
const PER_REFERENCE: u64 = 50;

#[test]
fn answering_no_does_not_make_the_resolver_start_over() {
    let Some(root) = vendor() else { return };
    let mut wasteful = Vec::new();
    let mut worst = 0.0f64;
    for path in models(&root) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut ws = sysml_semantics::Workspace::new();
        let file = ws.add_file(path.to_string_lossy(), &text);
        let stats = ws.resolve_files(&[file]);
        let references = (stats.resolved + stats.unresolved).max(1) as u64;
        let per = stats.lookups as f64 / references as f64;
        worst = worst.max(per);
        if stats.lookups > references * PER_REFERENCE {
            wasteful.push(format!(
                "{}: {} lookups for {references} references",
                path.display(),
                stats.lookups
            ));
        }
    }
    eprintln!("worst: {worst:.2} lookups per reference");
    assert!(
        wasteful.is_empty(),
        "the resolver stopped remembering:\n{}",
        wasteful.join("\n")
    );
}

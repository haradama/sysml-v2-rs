//! Regenerates `crates/sysml-model/src/generated.rs`.

fn main() {
    let path = sysml_codegen::run();
    println!("generated {}", path.display());
}

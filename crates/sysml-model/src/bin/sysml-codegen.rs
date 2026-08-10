//! Regenerates `crates/sysml-model/src/generated.rs`.

fn main() {
    let path = sysml_model::codegen::run();
    println!("generated {}", path.display());
}

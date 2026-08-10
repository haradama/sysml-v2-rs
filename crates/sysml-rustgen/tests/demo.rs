//! The whole loop, in process: the fixture crate's API imported as SysML,
//! the demo model resolved against it, Rust generated -- and held equal
//! to what `examples/order-system` has checked in, so drift in any stage
//! fails here first. A second test compiles and runs the demo crate.

use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|_| panic!("cannot read {}", path.display()))
}

/// The pipeline `make demo` runs, without touching the filesystem.
fn regenerate() -> (String, String) {
    let json =
        read(&repo().join("crates/sysml-import-api/tests/fixtures/inventory_store.rustdoc.json"));
    let api = sysml_import_api::rustdoc_to_sysml(&json, None).unwrap();

    let mut ws = sysml_semantics::Workspace::new();
    let model_dir = repo().join("examples/order-system/model");
    ws.add_file("scalars.kerml", &read(&model_dir.join("scalars.kerml")));
    ws.add_file("InventoryStoreApi.sysml", &api);
    let system = ws.add_file(
        "order_system.sysml",
        &read(&model_dir.join("order_system.sysml")),
    );
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "the demo model must resolve fully");

    let roots = ws.file_roots(system).to_vec();
    let rust = sysml_rustgen::generate(ws.model(), &roots).unwrap();
    (api, rust)
}

#[test]
fn the_checked_in_demo_matches_what_the_pipeline_generates() {
    let (api, rust) = regenerate();
    assert_eq!(
        api,
        read(&repo().join("examples/order-system/model/InventoryStoreApi.sysml")),
        "the checked-in API package drifted; run `make demo`"
    );
    assert_eq!(
        rust,
        read(&repo().join("examples/order-system/src/generated.rs")),
        "the checked-in generated code drifted; run `make demo`"
    );

    // the shape a caller relies on
    assert!(rust.contains("pub struct OrderPlanner<Store: inventory_store::InventoryStore>"));
    assert!(rust.contains(
        "pub fn check(&self, query: inventory_store::StockQuery) \
         -> Result<inventory_store::StockLevel, inventory_store::StoreError>"
    ));
    assert!(rust.contains("pub async fn observe(&self, sku: String)"));
    assert!(rust.contains("self.store.watch(sku).await"));
    assert!(rust.contains("pub fn reset(&mut self)"));
    // data: enum with default, struct with declared values, multiplicity
    assert!(rust.contains("pub enum Urgency {"));
    // an enum member that is no `enum` value is noted, not dropped
    assert!(
        rust.contains("// not generated: `replenishment` -- only `enum` values become variants")
    );
    // the first value is the default, said by an attribute on the
    // variant rather than an `impl` clippy would ask to be derived
    assert!(rust.contains("    #[default]\n    Routine,"), "{rust}");
    assert!(rust.contains("amount: 1,"));
    assert!(
        rust.contains("batch: 6 * 4,"),
        "a closed expression default"
    );
    assert!(rust.contains("pub lines: Vec<OrderLine>,"));
    // calculations: a calc def as a function (bool inferred from the
    // comparison), a calc usage as a method over the struct's fields
    assert!(rust.contains("pub fn needs_restock(level: u64, threshold: u64) -> bool {"));
    assert!(rust.contains("    level < threshold\n}"));
    assert!(rust.contains("pub fn bulk(&self) -> bool {"));
    assert!(rust.contains("self.amount >= self.batch"));
    // composition of a generic part carries its parameter upward
    assert!(rust.contains("pub struct Warehouse<PlannerStore: inventory_store::InventoryStore>"));
    assert!(rust.contains("pub planner: OrderPlanner<PlannerStore>,"));
    // requirements become ignored verification stubs, satisfiers named
    assert!(rust.contains("mod requirements {"));
    assert!(rust.contains("/// Satisfied by `OrderPlanner`."));
    assert!(rust.contains("fn stock_visibility() {}"));
    // the state machine: states, events, guarded step
    assert!(rust.contains("pub enum OrderPhaseState {"));
    assert!(rust.contains("Submit(OrderLine),"));
    assert!(rust.contains("/// SysML guard: `[line.amount > 0]`"));
    // the guard's expression became the hook's default body
    assert!(rust.contains(
        "fn guard_submit(&self, line: &OrderLine) -> bool {\n        line.amount > 0\n    }"
    ));
    assert!(rust.contains("if hooks.guard_submit(line)"));
    assert!(rust.contains("hooks.on_entry_ordering();"));
    assert!(rust.contains("(state, _) => state,"));
}

#[test]
fn the_demo_crate_compiles_and_runs() {
    let output = std::process::Command::new(env!("CARGO"))
        .args(["run", "--quiet"])
        .current_dir(repo().join("examples/order-system"))
        .output()
        .expect("cargo runs");
    assert!(
        output.status.success(),
        "the demo did not build:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "SKU-042: 7 in stock (threshold 10) -> replenish\n\
         phase: ordering\n\
         phase settled: Idle"
    );
}

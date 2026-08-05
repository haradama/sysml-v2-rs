//! The hand-written half of the demo: a stub implementation of the
//! in-house API and a main that drives the generated part.
//!
//! Everything in `generated.rs` came out of `model/order_system.sysml`
//! via `sysml rustgen` (see `make demo` at the repository root).

#[allow(dead_code)]
mod generated;

use generated::{OrderPlanner, Warehouse};
use inventory_store::{InventoryStore, StockLevel, StockQuery, StoreError};

/// An in-memory stand-in for the real inventory service.
struct FakeStore {
    refreshed: u32,
}

impl InventoryStore for FakeStore {
    fn get_stock(&self, query: StockQuery) -> Result<StockLevel, StoreError> {
        if query.sku.is_empty() {
            return Err(StoreError {
                message: "empty sku".to_string(),
            });
        }
        Ok(StockLevel {
            sku: query.sku,
            quantity: 7,
            locations: vec!["tokyo-1".to_string()],
            fill_ratio: 0.35,
            pinned: false,
        })
    }

    async fn watch(&self, sku: String) -> StockLevel {
        StockLevel {
            sku,
            quantity: 9,
            locations: Vec::new(),
            fill_ratio: 0.4,
            pinned: false,
        }
    }

    fn refresh(&mut self) {
        self.refreshed += 1;
    }
}

/// The machine's open decisions. The submit guard needs no code here:
/// its SysML expression translated into the generated default body.
struct PhaseRules;

impl generated::OrderPhaseHooks for PhaseRules {
    fn on_entry_ordering(&mut self) {
        println!("phase: ordering");
    }
}

fn main() {
    // the generated composition: a site holding one planner, the API
    // implementation flowing through the propagated generic parameter
    let mut warehouse = Warehouse {
        site: "tokyo-1".to_string(),
        planner: OrderPlanner {
            store: FakeStore { refreshed: 0 },
            threshold: 10,
            lines: vec![generated::OrderLine {
                sku: "SKU-042".to_string(),
                ..Default::default()
            }],
        },
    };
    let planner = &mut warehouse.planner;
    planner.reset();
    let level = planner
        .check(StockQuery {
            sku: "SKU-042".to_string(),
            warehouses: vec![1, 2],
            note: None,
            filter: true,
        })
        .expect("the fake store answers");
    // the decision rule is the model's own `calc def NeedsRestock`
    let verdict = if generated::needs_restock(level.quantity, planner.threshold) {
        "replenish"
    } else {
        "enough"
    };
    println!(
        "{}: {} in stock (threshold {}) -> {verdict}",
        level.sku, level.quantity, planner.threshold
    );

    // drive the generated state machine through one round
    let mut rules = PhaseRules;
    let mut phase = generated::OrderPhaseState::initial();
    phase = phase.step(
        &generated::OrderPhaseEvent::Submit(planner.lines[0].clone()),
        &mut rules,
    );
    phase = phase.step(&generated::OrderPhaseEvent::Settle, &mut rules);
    println!("phase settled: {phase:?}");
}

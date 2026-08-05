// The demo's own model: a new system calling the in-house inventory API.
// `InventoryStoreApi.sysml` next to this file is generated from the API
// crate by `sysml import-rust`; this file is written by hand.
package OrderSystem {
	private import InventoryStoreApi::*;
	private import ScalarValues::*;

	enum def Urgency {
		enum routine;
		enum rush;
	}

	enum def OrderType {
		attribute replenishment;
	}

	part def OrderLine {
		doc /* One article of a replenishment order. */
		attribute sku : String;
		attribute amount : Natural = 1;
		attribute batch : Natural = 6 * 4;
		attribute urgency : Urgency;
		calc bulk : Boolean = amount >= batch;
	}

	calc def NeedsRestock {
		doc /* Replenish when the level falls below the threshold. */
		in level : Natural;
		in threshold : Natural;
		level < threshold
	}

	part def OrderPlanner {
		doc /* Plans replenishment against the in-house inventory API. */
		port store : InventoryStore;
		attribute threshold : Natural;
		part lines : OrderLine[*];
		perform action check : GetStock;
		perform action observe : Watch;
		perform action reset : Refresh;
	}

	part def Warehouse {
		doc /* A site running one planner against the shared inventory. */
		attribute site : String = "tokyo-1";
		part planner : OrderPlanner;
	}

	requirement def StockVisibility {
		doc /* Every article's stock level must be answerable on demand. */
	}
	satisfy StockVisibility by OrderPlanner;

	state def OrderPhase {
		doc /* How one replenishment round proceeds. */
		state idle;
		state ordering;
		transition submit first idle accept line : OrderLine if line.amount > 0 then ordering;
		transition settle first ordering then idle;
	}
}

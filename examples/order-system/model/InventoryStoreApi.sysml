// generated from crate `inventory_store` by `sysml import-rust` -- DO NOT EDIT
package InventoryStoreApi {
	private import ScalarValues::*;

	// how each definition binds to the Rust it came from
	metadata def rust {
		attribute path : String;
		attribute crateName : String;
		attribute takesSelf : String;
		attribute isAsync : Boolean;
		attribute isFallible : Boolean;
	}
	doc /* What to look up. */
	item def StockQuery {
		@rust { :>> path = "inventory_store::StockQuery"; :>> crateName = "inventory_store"; }
		doc /* The article to check. */
		attribute sku : String;
		attribute warehouses : Natural[*];
		attribute note : String[0..1];
		doc /* `filter` is a SysML keyword: the importer must quote it. */
		attribute 'filter' : Boolean;
	}
	doc /* What a lookup answers. */
	item def StockLevel {
		@rust { :>> path = "inventory_store::StockLevel"; :>> crateName = "inventory_store"; }
		attribute sku : String;
		attribute quantity : Natural;
		attribute locations : String[*];
		attribute fill_ratio : Real;
		attribute pinned : Boolean;
	}
	doc /* Where a warehouse is. */
	enum def Region {
		@rust { :>> path = "inventory_store::Region"; :>> crateName = "inventory_store"; }
		enum Tokyo;
		enum Osaka;
		enum Fukuoka;
	}
	doc /* Why a lookup failed. */
	item def StoreError {
		@rust { :>> path = "inventory_store::StoreError"; :>> crateName = "inventory_store"; }
		attribute 'message' : String;
	}
	doc /* The inventory service every deployment provides. */
	port def InventoryStore {
		@rust { :>> path = "inventory_store::InventoryStore"; :>> crateName = "inventory_store"; }
	}
	doc /* Look one article up. */
	action def GetStock {
		@rust { :>> path = "inventory_store::InventoryStore::get_stock"; :>> crateName = "inventory_store"; :>> takesSelf = "&self"; :>> isAsync = false; :>> isFallible = true; }
		in query : StockQuery;
		out result : StockLevel;
		out error : StoreError[0..1];
	}
	doc /* Wait for the level of one article to change. */
	action def Watch {
		@rust { :>> path = "inventory_store::InventoryStore::watch"; :>> crateName = "inventory_store"; :>> takesSelf = "&self"; :>> isAsync = true; :>> isFallible = false; }
		in sku : String;
		out result : StockLevel;
	}
	doc /* Drop caches. */
	action def Refresh {
		@rust { :>> path = "inventory_store::InventoryStore::refresh"; :>> crateName = "inventory_store"; :>> takesSelf = "&mut self"; :>> isAsync = false; :>> isFallible = false; }
	}
	doc /* The region a query defaults to. */
	action def DefaultRegion {
		@rust { :>> path = "inventory_store::default_region"; :>> crateName = "inventory_store"; :>> takesSelf = ""; :>> isAsync = false; :>> isFallible = false; }
		out result : Region;
	}
	doc /* Signed and character-ish scalars; careful doc with *\/ inside. */
	item def Adjustment {
		@rust { :>> path = "inventory_store::Adjustment"; :>> crateName = "inventory_store"; }
		attribute delta : Integer;
		attribute grade : String;
		attribute cause : Adjustment[0..1];
	}
	doc /* A connection that is consumed when closed. */
	port def Session {
		@rust { :>> path = "inventory_store::Session"; :>> crateName = "inventory_store"; }
	}
	doc /* Takes `self` by value. */
	action def Close {
		@rust { :>> path = "inventory_store::Session::close"; :>> crateName = "inventory_store"; :>> takesSelf = "self"; :>> isAsync = false; :>> isFallible = false; }
	}
	doc /* A borrowed parameter reads as its owned value. */
	action def Label {
		@rust { :>> path = "inventory_store::Session::label"; :>> crateName = "inventory_store"; :>> takesSelf = "&self"; :>> isAsync = false; :>> isFallible = false; }
		in name : String;
		out result : Boolean;
	}

	// not imported (no monomorphic SysML shape):
	//   pick -- unsupported signature
	//   Pair -- not a plain struct
	//   Event -- not a plain enum
	//   Adjustment.bounds -- unmappable type
	//   Session::tally -- unsupported signature
	//   Session::dump -- unsupported signature
	//   Session::fetch -- unsupported signature
}

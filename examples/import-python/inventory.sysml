// generated from module `inventory` by `sysml-import-python` -- DO NOT EDIT
package InventoryApi {
	private import ScalarValues::*;

	// how each definition binds to the code it came from
	metadata def code {
		attribute writtenIn : String;
		attribute path : String;
		attribute module : String;
		attribute takesSelf : String;
		attribute isAsync : Boolean;
		attribute isFallible : Boolean;
		attribute capabilities : String;
	}

	// the language's own scalars, each a kind of the SysML one it is nearest
	attribute def PyBytes :> String { @code { :>> writtenIn = "python"; :>> path = "bytes"; :>> capabilities = "Hashable"; } }
	enum def Region {
		doc /* Where a warehouse is. */
		@code { :>> writtenIn = "python"; :>> path = "inventory.Region"; :>> module = "inventory"; }
		enum TOKYO;
		enum OSAKA;
		enum FUKUOKA;
	}
	item def StockQuery {
		doc /* What to look up. */
		@code { :>> writtenIn = "python"; :>> path = "inventory.StockQuery"; :>> module = "inventory"; }
		attribute sku : String;
		attribute warehouses : Integer[*];
		attribute note : String[0..1];
		attribute 'filter' : Boolean;
		attribute raw : PyBytes;
	}
	item def StockLevel {
		doc /* What a lookup answers. */
		@code { :>> writtenIn = "python"; :>> path = "inventory.StockLevel"; :>> module = "inventory"; }
		attribute sku : String;
		attribute quantity : Integer;
		attribute locations : String[*];
		attribute fill_ratio : Real;
		attribute pinned : Boolean;
		attribute region : Region[0..1];
	}
	port def InventoryStore {
		doc /* The inventory service every deployment provides. */
		@code { :>> writtenIn = "python"; :>> path = "inventory.InventoryStore"; :>> module = "inventory"; }
	}
	action def GetStock {
		doc /* Look one article up. */
		@code { :>> writtenIn = "python"; :>> path = "inventory.InventoryStore.get_stock"; :>> module = "inventory"; :>> takesSelf = "self"; :>> isAsync = false; :>> isFallible = true; }
		in query : StockQuery;
		out result : StockLevel;
	}
	action def Watch {
		doc /* Wait for the level of one article to change. */
		@code { :>> writtenIn = "python"; :>> path = "inventory.InventoryStore.watch"; :>> module = "inventory"; :>> takesSelf = "self"; :>> isAsync = true; :>> isFallible = false; }
		in sku : String;
		out result : StockLevel;
	}
	action def Refresh {
		doc /* Drop caches. */
		@code { :>> writtenIn = "python"; :>> path = "inventory.InventoryStore.refresh"; :>> module = "inventory"; :>> takesSelf = "self"; :>> isAsync = false; :>> isFallible = false; }
	}
	action def DefaultRegion {
		doc /* The region a query defaults to. */
		@code { :>> writtenIn = "python"; :>> path = "inventory.default_region"; :>> module = "inventory"; :>> takesSelf = ""; :>> isAsync = false; :>> isFallible = false; }
		out result : Region;
	}

	// not imported (no monomorphic SysML shape):
	//   StockQuery.history -- no SysML shape for `list[list[int]]`
	//   StoreError -- a class with no annotated attributes
	//   inventory.InventoryStore.tally -- no SysML shape for `dict[str, int]`
	//   inventory.pick -- `index` has no annotation
	//   Pair -- an assignment, not a declaration
}

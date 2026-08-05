//! An in-house inventory API, as a fixture for the SysML importer.

/// What to look up.
pub struct StockQuery {
    /// The article to check.
    pub sku: String,
    pub warehouses: Vec<u32>,
    pub note: Option<String>,
    /// `filter` is a SysML keyword: the importer must quote it.
    pub filter: bool,
}

/// What a lookup answers.
pub struct StockLevel {
    pub sku: String,
    pub quantity: u64,
    pub locations: Vec<String>,
    pub fill_ratio: f64,
    pub pinned: bool,
}

/// Where a warehouse is.
pub enum Region {
    Tokyo,
    Osaka,
    Fukuoka,
}

/// Why a lookup failed.
#[derive(Debug)]
pub struct StoreError {
    pub message: String,
}

/// The inventory service every deployment provides.
pub trait InventoryStore {
    /// Look one article up.
    fn get_stock(&self, query: StockQuery) -> Result<StockLevel, StoreError>;
    /// Wait for the level of one article to change.
    async fn watch(&self, sku: String) -> StockLevel;
    /// Drop caches.
    fn refresh(&mut self);
}

/// The region a query defaults to.
pub fn default_region() -> Region {
    Region::Tokyo
}

/// Not importable (generic): listed so the importer's skip path is honest.
pub fn pick<T>(candidates: Vec<T>) -> Option<T> {
    candidates.into_iter().next()
}

/// A tuple struct has no field names to become attributes: skipped.
pub struct Pair(pub u32, pub u32);

/// A variant with data has no plain SysML enum shape: skipped.
pub enum Event {
    Restocked(u64),
    Emptied,
}

/// Signed and character-ish scalars; careful doc with */ inside.
pub struct Adjustment {
    pub delta: i64,
    pub grade: char,
    pub cause: Option<Box<Adjustment>>,
    /// A tuple type has no monomorphic SysML shape: skipped.
    pub bounds: (u32, u32),
}

/// A connection that is consumed when closed.
pub trait Session {
    /// Takes `self` by value.
    fn close(self);
    /// A map has no monomorphic SysML shape: skipped.
    fn tally(&self, counts: std::collections::HashMap<String, u64>) -> bool;
    /// A borrowed parameter reads as its owned value.
    fn label(&self, name: &str) -> bool;
    /// An unmappable return type: skipped.
    fn dump(&self) -> std::collections::HashMap<String, u64>;
    /// An unmappable error type: skipped.
    fn fetch(&self) -> Result<String, std::collections::HashMap<String, u64>>;
}

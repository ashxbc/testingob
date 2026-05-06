use serde::{Deserialize, Serialize};

pub const CH_BOOK: &str = "lv:book";
pub const CH_TRADE: &str = "lv:trade";
pub const CH_VACUUM: &str = "lv:vacuum";
pub const CH_WALL: &str = "lv:wall";
pub const CH_PREDICT: &str = "lv:predict";
pub const CH_LIQ: &str = "lv:liq";
pub const CH_CLUSTER: &str = "lv:cluster";

pub const KEY_STATE: &str = "lv:state";
pub const KEY_WALLS: &str = "lv:walls";
pub const KEY_VACUUMS: &str = "lv:vacuums";
pub const KEY_PREDICT: &str = "lv:predict";
pub const KEY_HISTORY: &str = "lv:thesis_history";
pub const KEY_CLUSTERS: &str = "lv:clusters";
pub const KEY_LIQ_RECENT: &str = "lv:liq_recent";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Bid,
    Ask,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookSnapshot {
    pub ts: i64,
    pub mid: f64,
    pub best_bid: f64,
    pub best_ask: f64,
    pub spread_bps: f64,
    pub bid_depth_1pct: f64,
    pub ask_depth_1pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeEvent {
    pub ts: i64,
    pub price: f64,
    pub qty: f64,
    pub is_buyer_maker: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wall {
    pub id: String,
    pub side: Side,
    pub price: f64,
    pub qty: f64,
    pub notional: f64,
    pub distance_bps: f64,
    pub first_seen: i64,
    pub last_seen: i64,
    #[serde(default)]
    pub touches: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VacuumEvent {
    pub ts: i64,
    pub side: Side,
    pub price: f64,
    pub qty_pulled: f64,
    pub notional_pulled: f64,
    pub mid_at_pull: f64,
    pub distance_bps: f64,
    pub age_ms: i64,
    pub reason: VacuumReason,
    #[serde(default)]
    pub wall_id: String,
    #[serde(default)]
    pub defense_count: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VacuumReason {
    Cancelled,
    Filled,
    Mixed,
}

// ============ Thesis-based predictions ============

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PredictPayload {
    /// Active prediction with a real thesis.
    Thesis(Thesis),
    /// No qualifying trigger yet — show conditions being watched.
    Watching(WatchState),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThesisStatus {
    Active,
    Filled,
    Invalidated,
    Expired,
    Reversed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thesis {
    pub id: String,
    pub created_ts: i64,
    pub direction: i8,
    pub mid_at_creation: f64,
    pub current_mid: f64,
    pub target_price: f64,
    pub target_reason: String,
    pub stop_price: f64,
    pub expires_at: i64,
    pub status: ThesisStatus,
    pub trigger: TriggerInfo,
    pub checklist: Vec<CheckItem>,
    pub confidence: f64,
    pub progress: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerInfo {
    pub event: String,
    pub wall_id: String,
    pub wall_side: Side,
    pub wall_price: f64,
    pub wall_notional: f64,
    pub wall_age_s: i64,
    pub defense_count: u32,
    pub pull_reason: VacuumReason,
    pub quality_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckItem {
    pub label: String,
    pub passed: bool,
}

// ============ Liquidations ============

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Liquidation {
    pub ts: i64,
    pub exchange: String,
    /// Side of the liquidated position. Long liq = forced sell; Short liq = forced buy.
    pub side: LiqSide,
    pub price: f64,
    pub qty: f64,
    pub notional: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum LiqSide {
    Long,
    Short,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cluster {
    pub bucket: f64,
    pub long_notional: f64,
    pub short_notional: f64,
    pub total_notional: f64,
    pub event_count: u32,
    pub last_event_ts: i64,
    pub exchanges: Vec<String>,
    pub strength: f64,
    pub distance_bps: f64,
    pub side: LiqSide,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterSnapshot {
    pub ts: i64,
    pub mid: f64,
    pub bucket_size: f64,
    pub clusters: Vec<Cluster>,
    pub long_total: f64,
    pub short_total: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchState {
    pub ts: i64,
    pub mid: f64,
    pub watching: Vec<String>,
    pub last_thesis: Option<Thesis>,
}

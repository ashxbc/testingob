use serde::{Deserialize, Serialize};

pub const CH_BOOK: &str = "lv:book";
pub const CH_TRADE: &str = "lv:trade";
pub const CH_VACUUM: &str = "lv:vacuum";
pub const CH_WALL: &str = "lv:wall";
pub const CH_PREDICT: &str = "lv:predict";

pub const KEY_STATE: &str = "lv:state";
pub const KEY_WALLS: &str = "lv:walls";
pub const KEY_VACUUMS: &str = "lv:vacuums";
pub const KEY_PREDICT: &str = "lv:predict";
pub const KEY_HISTORY: &str = "lv:thesis_history";

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchState {
    pub ts: i64,
    pub mid: f64,
    pub watching: Vec<String>,
    pub last_thesis: Option<Thesis>,
}

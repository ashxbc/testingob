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
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VacuumReason {
    Cancelled,
    Filled,
    Mixed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prediction {
    pub ts: i64,
    pub mid: f64,
    pub direction: i8,
    pub target_price: f64,
    pub target_bps: f64,
    pub confidence: f64,
    pub horizon_seconds: i64,
    pub features: PredictFeatures,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictFeatures {
    pub ofi_5m: f64,
    pub cvd_slope_5m: f64,
    pub vacuum_imbalance_5m: f64,
    pub wall_pressure: f64,
    pub atr_15m_bps: f64,
    pub thinness_up: f64,
    pub thinness_down: f64,
    pub direction_score: f64,
}

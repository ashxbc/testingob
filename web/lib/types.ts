export type Side = 'bid' | 'ask';

export interface BookSnapshot {
  ts: number;
  mid: number;
  best_bid: number;
  best_ask: number;
  spread_bps: number;
  bid_depth_1pct: number;
  ask_depth_1pct: number;
}

export interface Wall {
  id: string;
  side: Side;
  price: number;
  qty: number;
  notional: number;
  distance_bps: number;
  first_seen: number;
  last_seen: number;
}

export interface VacuumEvent {
  ts: number;
  side: Side;
  price: number;
  qty_pulled: number;
  notional_pulled: number;
  mid_at_pull: number;
  distance_bps: number;
  age_ms: number;
  reason: 'cancelled' | 'filled' | 'mixed';
}

export interface PredictFeatures {
  ofi_5m: number;
  cvd_slope_5m: number;
  vacuum_imbalance_5m: number;
  wall_pressure: number;
  atr_15m_bps: number;
  thinness_up: number;
  thinness_down: number;
  direction_score: number;
}

export interface Prediction {
  ts: number;
  mid: number;
  direction: number;
  target_price: number;
  target_bps: number;
  confidence: number;
  horizon_seconds: number;
  features: PredictFeatures;
  label: string;
}

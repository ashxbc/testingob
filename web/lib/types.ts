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
  touches: number;
}

export type VacuumReason = 'cancelled' | 'filled' | 'mixed';

export interface VacuumEvent {
  ts: number;
  side: Side;
  price: number;
  qty_pulled: number;
  notional_pulled: number;
  mid_at_pull: number;
  distance_bps: number;
  age_ms: number;
  reason: VacuumReason;
  wall_id: string;
  defense_count: number;
}

export type ThesisStatus =
  | 'active'
  | 'filled'
  | 'invalidated'
  | 'expired'
  | 'reversed';

export interface CheckItem {
  label: string;
  passed: boolean;
}

export interface TriggerInfo {
  event: string;
  wall_id: string;
  wall_side: Side;
  wall_price: number;
  wall_notional: number;
  wall_age_s: number;
  defense_count: number;
  pull_reason: VacuumReason;
  quality_score: number;
}

export interface Thesis {
  id: string;
  created_ts: number;
  direction: number;
  mid_at_creation: number;
  current_mid: number;
  target_price: number;
  target_reason: string;
  stop_price: number;
  expires_at: number;
  status: ThesisStatus;
  trigger: TriggerInfo;
  checklist: CheckItem[];
  confidence: number;
  progress: number;
}

export interface WatchState {
  ts: number;
  mid: number;
  watching: string[];
  last_thesis: Thesis | null;
}

export type PredictPayload =
  | ({ kind: 'thesis' } & Thesis)
  | ({ kind: 'watching' } & WatchState);

export type LiqSide = 'long' | 'short';

export interface Liquidation {
  ts: number;
  exchange: string;
  side: LiqSide;
  price: number;
  qty: number;
  notional: number;
}

export interface Cluster {
  bucket: number;
  long_notional: number;
  short_notional: number;
  total_notional: number;
  event_count: number;
  last_event_ts: number;
  exchanges: string[];
  strength: number;
  distance_bps: number;
  side: LiqSide;
}

export interface ClusterSnapshot {
  ts: number;
  mid: number;
  bucket_size: number;
  clusters: Cluster[];
  long_total: number;
  short_total: number;
}

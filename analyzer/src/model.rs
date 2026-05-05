use chrono::Utc;
use common::{
    BookSnapshot, CheckItem, PredictPayload, Side, Thesis, ThesisStatus, TriggerInfo,
    VacuumEvent, VacuumReason, WatchState,
};

use crate::features::{Features, TrackedWall};

const HORIZON_MS: i64 = 15 * 60 * 1000;

// ---- Quality gates ----
const MIN_WALL_AGE_S: i64 = 30;
const MIN_WALL_NOTIONAL_USD: f64 = 1_000_000.0;
const MIN_OFI_FOR_CONFIRM: f64 = 0.05;
const MIN_CVD_FOR_CONFIRM: f64 = 0.5; // BTC/min
const PATH_THIN_LOOKAHEAD_PCT: f64 = 0.005;
const REVERSAL_QUALITY_DELTA: f64 = 0.10;

pub struct ThesisManager {
    pub active: Option<Thesis>,
    pub last_archived: Option<Thesis>,
    next_id: u64,
}

impl ThesisManager {
    pub fn new() -> Self {
        Self {
            active: None,
            last_archived: None,
            next_id: 0,
        }
    }

    fn alloc_id(&mut self) -> String {
        self.next_id += 1;
        format!("t{}", self.next_id)
    }

    /// Run on each tick. Mutates the active thesis status (filled/expired/invalidated)
    /// and processes any new vacuum events that may trigger or reverse a thesis.
    pub fn tick(
        &mut self,
        book: &BookSnapshot,
        feats: &Features,
        walls: &[TrackedWall],
        new_vacuums: Vec<VacuumEvent>,
    ) {
        let now = Utc::now().timestamp_millis();

        // 1. Update existing thesis status against current price.
        if let Some(t) = self.active.as_mut() {
            t.current_mid = book.mid;
            let dist_total = (t.target_price - t.mid_at_creation).abs().max(1.0);
            let dist_now = match t.direction {
                1 => book.mid - t.mid_at_creation,
                -1 => t.mid_at_creation - book.mid,
                _ => 0.0,
            };
            t.progress = (dist_now / dist_total).clamp(-0.5, 1.5);

            let hit_target = match t.direction {
                1 => book.mid >= t.target_price,
                -1 => book.mid <= t.target_price,
                _ => false,
            };
            let hit_stop = match t.direction {
                1 => book.mid <= t.stop_price,
                -1 => book.mid >= t.stop_price,
                _ => false,
            };
            let expired = now >= t.expires_at;

            if hit_target {
                t.status = ThesisStatus::Filled;
            } else if hit_stop {
                t.status = ThesisStatus::Invalidated;
            } else if expired {
                t.status = ThesisStatus::Expired;
            }
        }

        // 2. Archive closed thesis.
        if let Some(t) = self.active.as_ref() {
            if t.status != ThesisStatus::Active {
                self.last_archived = Some(t.clone());
                self.active = None;
            }
        }

        // 3. Evaluate new vacuum events as potential triggers.
        for v in new_vacuums {
            if let Some(new_thesis) = self.evaluate_trigger(&v, book, feats, walls) {
                // If a thesis is active in the OPPOSITE direction with lower quality, reverse it.
                if let Some(active) = self.active.as_mut() {
                    if active.direction != new_thesis.direction
                        && new_thesis.confidence
                            > active.trigger.quality_score + REVERSAL_QUALITY_DELTA
                    {
                        active.status = ThesisStatus::Reversed;
                        self.last_archived = Some(active.clone());
                        self.active = None;
                    } else {
                        // Either same direction (already covered) or weaker — ignore.
                        continue;
                    }
                }
                self.active = Some(new_thesis);
            }
        }
    }

    fn evaluate_trigger(
        &mut self,
        v: &VacuumEvent,
        book: &BookSnapshot,
        feats: &Features,
        walls: &[TrackedWall],
    ) -> Option<Thesis> {
        // Direction implied by the pull side: ask pulled => up, bid pulled => down.
        let direction: i8 = match v.side {
            Side::Ask => 1,
            Side::Bid => -1,
        };

        let wall_age_s = v.age_ms / 1000;
        let cancelled = matches!(v.reason, VacuumReason::Cancelled | VacuumReason::Mixed);

        // Hard gates
        let g_age = wall_age_s >= MIN_WALL_AGE_S;
        let g_size = v.notional_pulled >= MIN_WALL_NOTIONAL_USD;
        let g_cancelled = cancelled;

        // Soft confirms
        let c_ofi = match direction {
            1 => feats.ofi_2m > MIN_OFI_FOR_CONFIRM,
            -1 => feats.ofi_2m < -MIN_OFI_FOR_CONFIRM,
            _ => false,
        };
        let c_cvd = match direction {
            1 => feats.cvd_slope_2m > MIN_CVD_FOR_CONFIRM,
            -1 => feats.cvd_slope_2m < -MIN_CVD_FOR_CONFIRM,
            _ => false,
        };
        let c_defended = v.defense_count >= 1;
        let c_clustered = match direction {
            1 => feats.ask_pulls_60s >= 2,
            -1 => feats.bid_pulls_60s >= 2,
            _ => false,
        };
        let c_path_thin = path_is_thin(walls, book.mid, direction);
        let c_no_opposite_fresh = !has_fresh_opposite_wall(walls, direction, v.ts);

        let confirms = [c_ofi, c_cvd, c_defended, c_clustered]
            .iter()
            .filter(|&&x| x)
            .count();

        // Decision: all 3 hard gates + path thin + no opposite + at least 2 confirms
        if !(g_age && g_size && g_cancelled && c_path_thin && c_no_opposite_fresh) {
            return None;
        }
        if confirms < 2 {
            return None;
        }

        // Build target
        let (target_price, target_reason) = find_target(walls, book.mid, direction);
        if target_price <= 0.0 {
            return None;
        }

        // Stop = 50% of target distance opposite, with a 30 bps floor
        let target_dist_bps = ((target_price - book.mid) / book.mid).abs() * 10_000.0;
        let stop_bps = (target_dist_bps * 0.5).max(30.0);
        let stop_price = book.mid * (1.0 - direction as f64 * stop_bps / 10_000.0);

        // Quality / confidence
        let age_score = ((wall_age_s as f64 / 300.0).min(1.0)) * 0.25;
        let size_score = ((v.notional_pulled / 5_000_000.0).min(1.0)) * 0.20;
        let defended_score = if c_defended { 0.15 } else { 0.0 };
        let confirm_score = (confirms as f64 / 4.0) * 0.30;
        let cluster_score = if c_clustered { 0.10 } else { 0.0 };
        let confidence = (age_score + size_score + defended_score + confirm_score + cluster_score)
            .min(0.99);

        let event_str = format!(
            "{} wall pulled at ${:.0} · ${:.1}M · age {}m{}s{}",
            match v.side {
                Side::Ask => "Ask",
                Side::Bid => "Bid",
            },
            v.price,
            v.notional_pulled / 1e6,
            wall_age_s / 60,
            wall_age_s % 60,
            if c_defended {
                format!(" · defended ×{}", v.defense_count)
            } else {
                String::new()
            },
        );

        let checklist = vec![
            CheckItem {
                label: format!("Wall age ≥ {}s", MIN_WALL_AGE_S),
                passed: g_age,
            },
            CheckItem {
                label: format!("Notional ≥ ${:.1}M", MIN_WALL_NOTIONAL_USD / 1e6),
                passed: g_size,
            },
            CheckItem {
                label: "Cancelled, not filled".into(),
                passed: g_cancelled,
            },
            CheckItem {
                label: "Defended level".into(),
                passed: c_defended,
            },
            CheckItem {
                label: "OFI confirms direction".into(),
                passed: c_ofi,
            },
            CheckItem {
                label: "CVD confirms direction".into(),
                passed: c_cvd,
            },
            CheckItem {
                label: "Path to target is thin".into(),
                passed: c_path_thin,
            },
            CheckItem {
                label: "No fresh opposing wall".into(),
                passed: c_no_opposite_fresh,
            },
            CheckItem {
                label: "Cluster of same-side pulls".into(),
                passed: c_clustered,
            },
        ];

        let now = Utc::now().timestamp_millis();
        let id = self.alloc_id();

        Some(Thesis {
            id,
            created_ts: now,
            direction,
            mid_at_creation: book.mid,
            current_mid: book.mid,
            target_price,
            target_reason,
            stop_price,
            expires_at: now + HORIZON_MS,
            status: ThesisStatus::Active,
            trigger: TriggerInfo {
                event: event_str,
                wall_id: v.wall_id.clone(),
                wall_side: v.side,
                wall_price: v.price,
                wall_notional: v.notional_pulled,
                wall_age_s,
                defense_count: v.defense_count,
                pull_reason: v.reason,
                quality_score: confidence,
            },
            checklist,
            confidence,
            progress: 0.0,
        })
    }
}

fn path_is_thin(walls: &[TrackedWall], mid: f64, direction: i8) -> bool {
    // Path is "thin" if there is no wall ≥ $2M within the next 0.5% in the direction of travel.
    let limit = mid * (1.0 + direction as f64 * PATH_THIN_LOOKAHEAD_PCT);
    let (lo, hi) = if direction > 0 {
        (mid, limit)
    } else {
        (limit, mid)
    };
    let blocking_side = if direction > 0 { Side::Ask } else { Side::Bid };
    !walls.iter().any(|w| {
        w.side == blocking_side && w.price >= lo && w.price <= hi && w.notional >= 2_000_000.0
    })
}

fn has_fresh_opposite_wall(walls: &[TrackedWall], direction: i8, now_ts: i64) -> bool {
    // A "fresh" opposing wall = appeared in last 30s and is large.
    let blocking_side = if direction > 0 { Side::Ask } else { Side::Bid };
    walls.iter().any(|w| {
        w.side == blocking_side
            && w.notional >= 2_000_000.0
            && (now_ts - w.first_seen) < 30_000
    })
}

fn find_target(walls: &[TrackedWall], mid: f64, direction: i8) -> (f64, String) {
    // Target = next thick wall in direction of travel within 1.5%, biased to the largest.
    let target_side = if direction > 0 { Side::Ask } else { Side::Bid };
    let max_dist = mid * 0.015;

    let mut candidates: Vec<&TrackedWall> = walls
        .iter()
        .filter(|w| w.side == target_side && w.notional >= 1_500_000.0)
        .filter(|w| match direction {
            1 => w.price > mid && w.price - mid <= max_dist,
            -1 => w.price < mid && mid - w.price <= max_dist,
            _ => false,
        })
        .collect();

    candidates.sort_by(|a, b| {
        // Prefer larger walls. Among similar sizes, prefer closer ones.
        b.notional
            .partial_cmp(&a.notional)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    if let Some(w) = candidates.first() {
        return (
            w.price,
            format!(
                "Next {} wall · ${:.1}M at ${:.0}",
                if direction > 0 { "ask" } else { "bid" },
                w.notional / 1e6,
                w.price
            ),
        );
    }

    // Fallback: nearest round $1k away in the direction of travel.
    let round = if direction > 0 {
        ((mid / 1000.0).ceil() * 1000.0) + 1000.0
    } else {
        ((mid / 1000.0).floor() * 1000.0) - 1000.0
    };
    (round, format!("Round number · ${:.0}", round))
}

pub fn build_payload(mgr: &ThesisManager, book: &BookSnapshot, feats: &Features) -> PredictPayload {
    if let Some(t) = mgr.active.as_ref() {
        return PredictPayload::Thesis(t.clone());
    }
    let mut watching = Vec::new();
    watching.push(format!(
        "OFI 2m: {:+.2} (need ≥ ±{:.2} to confirm)",
        feats.ofi_2m, MIN_OFI_FOR_CONFIRM
    ));
    watching.push(format!(
        "CVD slope: {:+.2} BTC/m (need ≥ ±{:.2})",
        feats.cvd_slope_2m, MIN_CVD_FOR_CONFIRM
    ));
    watching.push(format!(
        "Recent pulls 60s: {} ask · {} bid",
        feats.ask_pulls_60s, feats.bid_pulls_60s
    ));
    watching.push(format!(
        "ATR 15m: {:.0} bps",
        feats.atr_15m_bps
    ));
    watching.push("Awaiting a high-quality wall pull (≥ 30s old, ≥ $1M, cancelled)".into());

    PredictPayload::Watching(WatchState {
        ts: Utc::now().timestamp_millis(),
        mid: book.mid,
        watching,
        last_thesis: mgr.last_archived.clone(),
    })
}

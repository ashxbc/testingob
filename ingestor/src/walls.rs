use ahash::{AHashMap, AHashSet};
use common::{AppConfig, Side, VacuumEvent, VacuumReason, Wall};
use ordered_float::OrderedFloat;

use crate::orderbook::OrderBook;

#[derive(Clone)]
struct ActiveWall {
    id: String,
    price: f64,
    first_seen: i64,
    last_seen: i64,
    last_qty: f64,
    touches: u32,
    last_touch_ts: i64,
}

pub struct WallTracker {
    bids: AHashMap<OrderedFloat<f64>, ActiveWall>,
    asks: AHashMap<OrderedFloat<f64>, ActiveWall>,
    min_notional_usd: f64,
    relative_multiplier: f64,
    next_id: u64,
    /// Recent trade volume bucketed by integer price (cancel-vs-fill).
    recent_trades: AHashMap<i64, (f64, i64)>,
}

const TOUCH_BPS_THRESHOLD: f64 = 5.0;
const TOUCH_DEBOUNCE_MS: i64 = 30_000;

impl WallTracker {
    pub fn new(cfg: &AppConfig) -> Self {
        Self {
            bids: AHashMap::new(),
            asks: AHashMap::new(),
            min_notional_usd: cfg.wall_min_notional_usd,
            relative_multiplier: cfg.wall_relative_multiplier,
            next_id: 0,
            recent_trades: AHashMap::new(),
        }
    }

    fn alloc_id(&mut self) -> String {
        self.next_id += 1;
        format!("w{}", self.next_id)
    }

    pub fn record_trade(&mut self, price: f64, qty: f64, ts: i64) {
        let bucket = price.round() as i64;
        let entry = self.recent_trades.entry(bucket).or_insert((0.0, ts));
        entry.0 += qty;
        entry.1 = ts;
        let cutoff = ts - 5_000;
        self.recent_trades.retain(|_, (_, t)| *t >= cutoff);
    }

    fn traded_qty_near(&self, price: f64) -> f64 {
        let bucket = price.round() as i64;
        let mut sum = 0.0;
        for offset in -1..=1 {
            if let Some((q, _)) = self.recent_trades.get(&(bucket + offset)) {
                sum += *q;
            }
        }
        sum
    }

    pub fn reconcile(
        &mut self,
        book: &OrderBook,
        ts: i64,
    ) -> (Vec<Wall>, Vec<VacuumEvent>) {
        let Some(mid) = book.mid() else {
            return (vec![], vec![]);
        };

        let mut active = Vec::new();
        let mut vacuums = Vec::new();

        // Update touches on existing walls (price tested the level).
        update_touches(&mut self.bids, mid, ts);
        update_touches(&mut self.asks, mid, ts);

        // ---- BIDS ----
        let bid_median = book.median_qty_band(true, mid, 0.01).max(0.001);
        let bid_threshold_qty = bid_median * self.relative_multiplier;
        let bid_lo = mid * 0.97;

        let mut seen_bids: Vec<(OrderedFloat<f64>, f64)> = Vec::new();
        for (price, qty) in book.bids.range(OrderedFloat(bid_lo)..=OrderedFloat(mid)) {
            let notional = price.0 * qty;
            if notional >= self.min_notional_usd && *qty >= bid_threshold_qty {
                seen_bids.push((*price, *qty));
            }
        }
        let seen_bid_set: AHashSet<OrderedFloat<f64>> =
            seen_bids.iter().map(|(k, _)| *k).collect();

        let prev_bid_keys: Vec<_> = self.bids.keys().copied().collect();
        for key in prev_bid_keys {
            if !seen_bid_set.contains(&key) {
                let w = self.bids.remove(&key).unwrap();
                let traded = self.traded_qty_near(w.price);
                let pulled_qty = (w.last_qty - traded).max(0.0);
                let reason = classify(w.last_qty, traded);
                {
                    vacuums.push(VacuumEvent {
                        ts,
                        side: Side::Bid,
                        price: w.price,
                        qty_pulled: pulled_qty,
                        notional_pulled: pulled_qty * w.price,
                        mid_at_pull: mid,
                        distance_bps: ((mid - w.price) / mid) * 10_000.0,
                        age_ms: ts - w.first_seen,
                        reason,
                        wall_id: w.id.clone(),
                        defense_count: w.touches,
                    });
                }
            }
        }

        for (key, qty) in seen_bids {
            if !self.bids.contains_key(&key) {
                let id = self.alloc_id();
                self.bids.insert(
                    key,
                    ActiveWall {
                        id,
                        price: key.0,
                        first_seen: ts,
                        last_seen: ts,
                        last_qty: qty,
                        touches: 0,
                        last_touch_ts: 0,
                    },
                );
            }
            let entry = self.bids.get_mut(&key).unwrap();
            entry.last_seen = ts;
            entry.last_qty = qty;
            active.push(Wall {
                id: entry.id.clone(),
                side: Side::Bid,
                price: entry.price,
                qty: entry.last_qty,
                notional: entry.price * entry.last_qty,
                distance_bps: ((mid - entry.price) / mid) * 10_000.0,
                first_seen: entry.first_seen,
                last_seen: entry.last_seen,
                touches: entry.touches,
            });
        }

        // ---- ASKS ----
        let ask_median = book.median_qty_band(false, mid, 0.01).max(0.001);
        let ask_threshold_qty = ask_median * self.relative_multiplier;
        let ask_hi = mid * 1.03;

        let mut seen_asks: Vec<(OrderedFloat<f64>, f64)> = Vec::new();
        for (price, qty) in book.asks.range(OrderedFloat(mid)..=OrderedFloat(ask_hi)) {
            let notional = price.0 * qty;
            if notional >= self.min_notional_usd && *qty >= ask_threshold_qty {
                seen_asks.push((*price, *qty));
            }
        }
        let seen_ask_set: AHashSet<OrderedFloat<f64>> =
            seen_asks.iter().map(|(k, _)| *k).collect();

        let prev_ask_keys: Vec<_> = self.asks.keys().copied().collect();
        for key in prev_ask_keys {
            if !seen_ask_set.contains(&key) {
                let w = self.asks.remove(&key).unwrap();
                let traded = self.traded_qty_near(w.price);
                let pulled_qty = (w.last_qty - traded).max(0.0);
                let reason = classify(w.last_qty, traded);
                {
                    vacuums.push(VacuumEvent {
                        ts,
                        side: Side::Ask,
                        price: w.price,
                        qty_pulled: pulled_qty,
                        notional_pulled: pulled_qty * w.price,
                        mid_at_pull: mid,
                        distance_bps: ((w.price - mid) / mid) * 10_000.0,
                        age_ms: ts - w.first_seen,
                        reason,
                        wall_id: w.id.clone(),
                        defense_count: w.touches,
                    });
                }
            }
        }

        for (key, qty) in seen_asks {
            if !self.asks.contains_key(&key) {
                let id = self.alloc_id();
                self.asks.insert(
                    key,
                    ActiveWall {
                        id,
                        price: key.0,
                        first_seen: ts,
                        last_seen: ts,
                        last_qty: qty,
                        touches: 0,
                        last_touch_ts: 0,
                    },
                );
            }
            let entry = self.asks.get_mut(&key).unwrap();
            entry.last_seen = ts;
            entry.last_qty = qty;
            active.push(Wall {
                id: entry.id.clone(),
                side: Side::Ask,
                price: entry.price,
                qty: entry.last_qty,
                notional: entry.price * entry.last_qty,
                distance_bps: ((entry.price - mid) / mid) * 10_000.0,
                first_seen: entry.first_seen,
                last_seen: entry.last_seen,
                touches: entry.touches,
            });
        }

        (active, vacuums)
    }
}

fn update_touches(map: &mut AHashMap<OrderedFloat<f64>, ActiveWall>, mid: f64, ts: i64) {
    for w in map.values_mut() {
        let dist_bps = ((mid - w.price) / mid).abs() * 10_000.0;
        if dist_bps <= TOUCH_BPS_THRESHOLD && ts - w.last_touch_ts > TOUCH_DEBOUNCE_MS {
            w.touches += 1;
            w.last_touch_ts = ts;
        }
    }
}

fn classify(wall_qty: f64, traded_qty: f64) -> VacuumReason {
    if traded_qty < wall_qty * 0.1 {
        VacuumReason::Cancelled
    } else if traded_qty >= wall_qty * 0.9 {
        VacuumReason::Filled
    } else {
        VacuumReason::Mixed
    }
}

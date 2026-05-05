use ahash::AHashMap;
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
}

pub struct WallTracker {
    bids: AHashMap<OrderedFloat<f64>, ActiveWall>,
    asks: AHashMap<OrderedFloat<f64>, ActiveWall>,
    min_notional_usd: f64,
    relative_multiplier: f64,
    next_id: u64,
    /// Recent trade volume bucketed by integer price (for cancel-vs-fill disambiguation).
    recent_trades: AHashMap<i64, (f64, i64)>,
}

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
        let seen_bid_set: ahash::AHashSet<OrderedFloat<f64>> =
            seen_bids.iter().map(|(k, _)| *k).collect();

        // Disappearances
        let prev_bid_keys: Vec<_> = self.bids.keys().copied().collect();
        for key in prev_bid_keys {
            if !seen_bid_set.contains(&key) {
                let w = self.bids.remove(&key).unwrap();
                let traded = self.traded_qty_near(w.price);
                let pulled_qty = (w.last_qty - traded).max(0.0);
                let reason = classify(w.last_qty, traded);
                if pulled_qty * w.price >= self.min_notional_usd * 0.5 {
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
                    });
                }
            }
        }

        // Insert / update
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
        let seen_ask_set: ahash::AHashSet<OrderedFloat<f64>> =
            seen_asks.iter().map(|(k, _)| *k).collect();

        let prev_ask_keys: Vec<_> = self.asks.keys().copied().collect();
        for key in prev_ask_keys {
            if !seen_ask_set.contains(&key) {
                let w = self.asks.remove(&key).unwrap();
                let traded = self.traded_qty_near(w.price);
                let pulled_qty = (w.last_qty - traded).max(0.0);
                let reason = classify(w.last_qty, traded);
                if pulled_qty * w.price >= self.min_notional_usd * 0.5 {
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
            });
        }

        (active, vacuums)
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

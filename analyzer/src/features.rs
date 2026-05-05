use chrono::Utc;
use common::{BookSnapshot, Side, TradeEvent, VacuumEvent};
use std::collections::VecDeque;

const FIVE_MIN_MS: i64 = 5 * 60 * 1000;
const FIFTEEN_MIN_MS: i64 = 15 * 60 * 1000;

#[derive(Default)]
pub struct FeatureStore {
    pub last_book: Option<BookSnapshot>,
    pub books: VecDeque<BookSnapshot>,
    pub trades: VecDeque<TradeEvent>,
    pub vacuums: VecDeque<VacuumEvent>,
}

impl FeatureStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_book(&mut self, b: BookSnapshot) {
        self.last_book = Some(b.clone());
        self.books.push_back(b);
        self.evict();
    }
    pub fn on_trade(&mut self, t: TradeEvent) {
        self.trades.push_back(t);
        self.evict();
    }
    pub fn on_vacuum(&mut self, v: VacuumEvent) {
        self.vacuums.push_back(v);
        self.evict();
    }

    fn evict(&mut self) {
        let now = Utc::now().timestamp_millis();
        let cutoff = now - FIFTEEN_MIN_MS - 60_000;
        while self.books.front().map_or(false, |b| b.ts < cutoff) {
            self.books.pop_front();
        }
        while self.trades.front().map_or(false, |t| t.ts < cutoff) {
            self.trades.pop_front();
        }
        while self.vacuums.front().map_or(false, |v| v.ts < cutoff) {
            self.vacuums.pop_front();
        }
    }

    pub fn compute_features(&self) -> Features {
        let now = Utc::now().timestamp_millis();
        let cut5 = now - FIVE_MIN_MS;
        let cut15 = now - FIFTEEN_MIN_MS;

        // Order Flow Imbalance: change in (bid_depth - ask_depth) over 5m
        let book_now = self.last_book.as_ref();
        let book_5m_ago = self
            .books
            .iter()
            .find(|b| b.ts >= cut5)
            .or_else(|| self.books.front());

        let ofi_5m = match (book_now, book_5m_ago) {
            (Some(n), Some(p)) => {
                let now_imb = n.bid_depth_1pct - n.ask_depth_1pct;
                let prev_imb = p.bid_depth_1pct - p.ask_depth_1pct;
                (now_imb - prev_imb)
                    / (n.bid_depth_1pct + n.ask_depth_1pct).max(1.0)
            }
            _ => 0.0,
        };

        // CVD slope: signed trade volume per minute
        let cvd_now: f64 = self
            .trades
            .iter()
            .filter(|t| t.ts >= cut5)
            .map(|t| if t.is_buyer_maker { -t.qty } else { t.qty })
            .sum();
        let cvd_slope_5m = cvd_now / 5.0; // per-minute

        // Vacuum imbalance: ask-side pulls (bullish) minus bid-side pulls (bearish), in $
        let mut bull = 0.0;
        let mut bear = 0.0;
        for v in self.vacuums.iter().filter(|v| v.ts >= cut5) {
            // Cancelled walls are signal; filled walls are noise
            let weight = match v.reason {
                common::VacuumReason::Cancelled => 1.0,
                common::VacuumReason::Mixed => 0.5,
                common::VacuumReason::Filled => 0.1,
            };
            // Decay by recency (newer = stronger)
            let age = (now - v.ts) as f64 / FIVE_MIN_MS as f64;
            let recency = (1.0 - age).max(0.0);
            let w = weight * recency;
            match v.side {
                Side::Ask => bull += v.notional_pulled * w,
                Side::Bid => bear += v.notional_pulled * w,
            }
        }
        let total = (bull + bear).max(1.0);
        let vacuum_imbalance_5m = (bull - bear) / total;

        // Wall pressure: snapshot of remaining bid vs ask depth imbalance
        let wall_pressure = match book_now {
            Some(n) => {
                let denom = (n.bid_depth_1pct + n.ask_depth_1pct).max(1.0);
                (n.bid_depth_1pct - n.ask_depth_1pct) / denom
            }
            None => 0.0,
        };

        // ATR over 15m using book mid samples
        let recent: Vec<f64> = self
            .books
            .iter()
            .filter(|b| b.ts >= cut15)
            .map(|b| b.mid)
            .collect();
        let atr_15m_bps = if recent.len() > 1 {
            let max = recent.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let min = recent.iter().cloned().fold(f64::INFINITY, f64::min);
            let mid = book_now.map(|b| b.mid).unwrap_or(1.0);
            ((max - min) / mid) * 10_000.0
        } else {
            0.0
        };

        Features {
            ofi_5m,
            cvd_slope_5m,
            vacuum_imbalance_5m,
            wall_pressure,
            atr_15m_bps,
        }
    }
}

pub struct Features {
    pub ofi_5m: f64,
    pub cvd_slope_5m: f64,
    pub vacuum_imbalance_5m: f64,
    pub wall_pressure: f64,
    pub atr_15m_bps: f64,
}

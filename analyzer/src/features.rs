use ahash::AHashMap;
use chrono::Utc;
use common::{BookSnapshot, Side, TradeEvent, VacuumEvent, Wall};
use std::collections::VecDeque;

const TWO_MIN_MS: i64 = 2 * 60 * 1000;
const FIFTEEN_MIN_MS: i64 = 15 * 60 * 1000;

#[derive(Clone)]
pub struct TrackedWall {
    pub id: String,
    pub side: Side,
    pub price: f64,
    pub notional: f64,
    pub qty: f64,
    pub first_seen: i64,
    pub last_seen: i64,
    pub touches: u32,
    pub distance_bps: f64,
}

pub struct FeatureStore {
    pub last_book: Option<BookSnapshot>,
    pub books: VecDeque<BookSnapshot>,
    pub trades: VecDeque<TradeEvent>,
    pub vacuums: VecDeque<VacuumEvent>,
    pub recent_vacuums_unprocessed: VecDeque<VacuumEvent>,
    pub walls: AHashMap<String, TrackedWall>,
}

impl Default for FeatureStore {
    fn default() -> Self {
        Self {
            last_book: None,
            books: VecDeque::new(),
            trades: VecDeque::new(),
            vacuums: VecDeque::new(),
            recent_vacuums_unprocessed: VecDeque::new(),
            walls: AHashMap::new(),
        }
    }
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
        self.vacuums.push_back(v.clone());
        self.recent_vacuums_unprocessed.push_back(v);
        self.evict();
    }
    pub fn on_walls(&mut self, walls: Vec<Wall>) {
        let mut next: AHashMap<String, TrackedWall> = AHashMap::new();
        for w in walls {
            next.insert(
                w.id.clone(),
                TrackedWall {
                    id: w.id,
                    side: w.side,
                    price: w.price,
                    notional: w.notional,
                    qty: w.qty,
                    first_seen: w.first_seen,
                    last_seen: w.last_seen,
                    touches: w.touches,
                    distance_bps: w.distance_bps,
                },
            );
        }
        self.walls = next;
    }

    pub fn drain_unprocessed_vacuums(&mut self) -> Vec<VacuumEvent> {
        self.recent_vacuums_unprocessed.drain(..).collect()
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

    pub fn compute(&self) -> Features {
        let now = Utc::now().timestamp_millis();
        let cut2 = now - TWO_MIN_MS;
        let cut15 = now - FIFTEEN_MIN_MS;

        // OFI over 2m
        let book_now = self.last_book.as_ref();
        let book_2m_ago = self
            .books
            .iter()
            .find(|b| b.ts >= cut2)
            .or_else(|| self.books.front());

        let ofi_2m = match (book_now, book_2m_ago) {
            (Some(n), Some(p)) => {
                let now_imb = n.bid_depth_1pct - n.ask_depth_1pct;
                let prev_imb = p.bid_depth_1pct - p.ask_depth_1pct;
                let denom = (n.bid_depth_1pct + n.ask_depth_1pct).max(1.0);
                (now_imb - prev_imb) / denom
            }
            _ => 0.0,
        };

        // CVD (BTC) over 2m
        let cvd_2m: f64 = self
            .trades
            .iter()
            .filter(|t| t.ts >= cut2)
            .map(|t| if t.is_buyer_maker { -t.qty } else { t.qty })
            .sum();
        let cvd_slope_2m = cvd_2m / 2.0;

        // ATR over 15m
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

        // Recent same-side pulls (last 60s) — clustering signal
        let cluster_cut = now - 60_000;
        let mut bid_pulls = 0u32;
        let mut ask_pulls = 0u32;
        for v in self.vacuums.iter().rev() {
            if v.ts < cluster_cut {
                break;
            }
            if matches!(v.reason, common::VacuumReason::Filled) {
                continue;
            }
            match v.side {
                Side::Bid => bid_pulls += 1,
                Side::Ask => ask_pulls += 1,
            }
        }

        Features {
            ofi_2m,
            cvd_slope_2m,
            atr_15m_bps,
            ask_pulls_60s: ask_pulls,
            bid_pulls_60s: bid_pulls,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Features {
    pub ofi_2m: f64,
    pub cvd_slope_2m: f64,
    pub atr_15m_bps: f64,
    pub ask_pulls_60s: u32,
    pub bid_pulls_60s: u32,
}

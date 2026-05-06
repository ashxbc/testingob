use ahash::{AHashMap, AHashSet};
use chrono::Utc;
use common::{Cluster, ClusterSnapshot, LiqSide, Liquidation};

const BUCKET_SIZE: f64 = 50.0; // $50 buckets for BTC
const HALF_LIFE_MS: f64 = 30.0 * 60.0 * 1000.0; // 30-min half-life
const PRUNE_TTL_MS: i64 = 4 * 60 * 60 * 1000; // 4h max retention
const VIEW_BAND_PCT: f64 = 0.02; // show ±2% of mid

fn exchange_weight(name: &str) -> f64 {
    match name {
        "bybit" => 0.40,
        "binance" => 0.30,
        "okx" => 0.30,
        _ => 0.20,
    }
}

pub struct ClusterAggregator {
    by_bucket: AHashMap<i64, BucketState>,
    last_decay_ts: i64,
}

struct BucketState {
    long_notional: f64,
    short_notional: f64,
    event_count: u32,
    last_event_ts: i64,
    exchanges: AHashSet<String>,
}

impl ClusterAggregator {
    pub fn new() -> Self {
        Self {
            by_bucket: AHashMap::new(),
            last_decay_ts: Utc::now().timestamp_millis(),
        }
    }

    pub fn ingest(&mut self, liq: Liquidation) {
        let bucket_idx = (liq.price / BUCKET_SIZE).round() as i64;
        let entry = self.by_bucket.entry(bucket_idx).or_insert_with(|| BucketState {
            long_notional: 0.0,
            short_notional: 0.0,
            event_count: 0,
            last_event_ts: liq.ts,
            exchanges: AHashSet::new(),
        });
        let weighted = liq.notional * exchange_weight(&liq.exchange);
        match liq.side {
            LiqSide::Long => entry.long_notional += weighted,
            LiqSide::Short => entry.short_notional += weighted,
        }
        entry.event_count += 1;
        entry.last_event_ts = liq.ts;
        entry.exchanges.insert(liq.exchange);
    }

    pub fn snapshot(&mut self, mid: f64) -> ClusterSnapshot {
        let now = Utc::now().timestamp_millis();
        // Decay since last snapshot
        let dt = (now - self.last_decay_ts) as f64;
        let decay = (-dt * std::f64::consts::LN_2 / HALF_LIFE_MS).exp();
        self.last_decay_ts = now;

        self.by_bucket.retain(|_, b| {
            b.long_notional *= decay;
            b.short_notional *= decay;
            let total = b.long_notional + b.short_notional;
            if total < 1000.0 {
                return false;
            }
            (now - b.last_event_ts) < PRUNE_TTL_MS
        });

        let lo = mid * (1.0 - VIEW_BAND_PCT);
        let hi = mid * (1.0 + VIEW_BAND_PCT);
        let mut clusters: Vec<Cluster> = self
            .by_bucket
            .iter()
            .filter_map(|(idx, b)| {
                let bucket_price = *idx as f64 * BUCKET_SIZE;
                if bucket_price < lo || bucket_price > hi {
                    return None;
                }
                let total = b.long_notional + b.short_notional;
                let side = if b.long_notional >= b.short_notional {
                    LiqSide::Long
                } else {
                    LiqSide::Short
                };
                let mut exs: Vec<String> = b.exchanges.iter().cloned().collect();
                exs.sort();
                Some(Cluster {
                    bucket: bucket_price,
                    long_notional: b.long_notional,
                    short_notional: b.short_notional,
                    total_notional: total,
                    event_count: b.event_count,
                    last_event_ts: b.last_event_ts,
                    exchanges: exs,
                    strength: 0.0,
                    distance_bps: ((bucket_price - mid) / mid) * 10_000.0,
                    side,
                })
            })
            .collect();

        let max = clusters
            .iter()
            .map(|c| c.total_notional)
            .fold(0.0_f64, f64::max);
        if max > 0.0 {
            for c in &mut clusters {
                c.strength = (c.total_notional / max).clamp(0.0, 1.0);
            }
        }

        let long_total: f64 = clusters
            .iter()
            .filter(|c| c.bucket < mid)
            .map(|c| c.long_notional)
            .sum();
        let short_total: f64 = clusters
            .iter()
            .filter(|c| c.bucket > mid)
            .map(|c| c.short_notional)
            .sum();

        clusters.sort_by(|a, b| b.bucket.partial_cmp(&a.bucket).unwrap_or(std::cmp::Ordering::Equal));

        ClusterSnapshot {
            ts: now,
            mid,
            bucket_size: BUCKET_SIZE,
            clusters,
            long_total,
            short_total,
        }
    }
}

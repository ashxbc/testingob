use ordered_float::OrderedFloat;
use std::collections::BTreeMap;

/// In-memory L2 orderbook mirror keyed by price.
/// Bids stored as max-first via reverse iteration; asks min-first.
pub struct OrderBook {
    pub bids: BTreeMap<OrderedFloat<f64>, f64>,
    pub asks: BTreeMap<OrderedFloat<f64>, f64>,
    pub last_update_id: u64,
    pub initialized: bool,
}

impl OrderBook {
    pub fn new() -> Self {
        Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            last_update_id: 0,
            initialized: false,
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: BinanceSnapshot) {
        self.bids.clear();
        self.asks.clear();
        for [p, q] in snapshot.bids {
            if q > 0.0 {
                self.bids.insert(OrderedFloat(p), q);
            }
        }
        for [p, q] in snapshot.asks {
            if q > 0.0 {
                self.asks.insert(OrderedFloat(p), q);
            }
        }
        self.last_update_id = snapshot.last_update_id;
        self.initialized = true;
    }

    pub fn apply_delta(&mut self, delta: &BinanceDepthUpdate) {
        for [p, q] in &delta.bids {
            let key = OrderedFloat(*p);
            if *q == 0.0 {
                self.bids.remove(&key);
            } else {
                self.bids.insert(key, *q);
            }
        }
        for [p, q] in &delta.asks {
            let key = OrderedFloat(*p);
            if *q == 0.0 {
                self.asks.remove(&key);
            } else {
                self.asks.insert(key, *q);
            }
        }
        self.last_update_id = delta.final_update_id;
    }

    pub fn best_bid(&self) -> Option<f64> {
        self.bids.keys().next_back().map(|k| k.0)
    }
    pub fn best_ask(&self) -> Option<f64> {
        self.asks.keys().next().map(|k| k.0)
    }
    pub fn mid(&self) -> Option<f64> {
        match (self.best_bid(), self.best_ask()) {
            (Some(b), Some(a)) => Some((a + b) / 2.0),
            _ => None,
        }
    }

    /// Cumulative bid notional within `pct` (e.g. 0.01 = 1%) below mid.
    pub fn bid_depth_within(&self, mid: f64, pct: f64) -> f64 {
        let lo = mid * (1.0 - pct);
        self.bids
            .range(OrderedFloat(lo)..=OrderedFloat(mid))
            .map(|(p, q)| p.0 * q)
            .sum()
    }
    pub fn ask_depth_within(&self, mid: f64, pct: f64) -> f64 {
        let hi = mid * (1.0 + pct);
        self.asks
            .range(OrderedFloat(mid)..=OrderedFloat(hi))
            .map(|(p, q)| p.0 * q)
            .sum()
    }

    /// Median qty of orders within a band — used to flag walls relative to neighborhood.
    pub fn median_qty_band(&self, side_is_bid: bool, mid: f64, pct: f64) -> f64 {
        let mut qs: Vec<f64> = if side_is_bid {
            let lo = mid * (1.0 - pct);
            self.bids
                .range(OrderedFloat(lo)..=OrderedFloat(mid))
                .map(|(_, q)| *q)
                .collect()
        } else {
            let hi = mid * (1.0 + pct);
            self.asks
                .range(OrderedFloat(mid)..=OrderedFloat(hi))
                .map(|(_, q)| *q)
                .collect()
        };
        if qs.is_empty() {
            return 0.0;
        }
        qs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        qs[qs.len() / 2]
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct BinanceSnapshot {
    #[serde(rename = "lastUpdateId")]
    pub last_update_id: u64,
    #[serde(deserialize_with = "deser_levels")]
    pub bids: Vec<[f64; 2]>,
    #[serde(deserialize_with = "deser_levels")]
    pub asks: Vec<[f64; 2]>,
}

#[derive(Debug, serde::Deserialize)]
pub struct BinanceDepthUpdate {
    #[serde(rename = "U")]
    pub first_update_id: u64,
    #[serde(rename = "u")]
    pub final_update_id: u64,
    #[serde(rename = "b", deserialize_with = "deser_levels")]
    pub bids: Vec<[f64; 2]>,
    #[serde(rename = "a", deserialize_with = "deser_levels")]
    pub asks: Vec<[f64; 2]>,
}

fn deser_levels<'de, D>(d: D) -> Result<Vec<[f64; 2]>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let raw: Vec<[String; 2]> = Vec::deserialize(d)?;
    raw.into_iter()
        .map(|[p, q]| {
            Ok([
                p.parse::<f64>().map_err(serde::de::Error::custom)?,
                q.parse::<f64>().map_err(serde::de::Error::custom)?,
            ])
        })
        .collect()
}

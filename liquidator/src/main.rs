mod aggregator;
mod streams;

use anyhow::Result;
use common::{
    telemetry, AppConfig, BookSnapshot, Liquidation, RedisBus, CH_BOOK, CH_CLUSTER, CH_LIQ,
    KEY_CLUSTERS, KEY_LIQ_RECENT,
};
use futures_util::StreamExt;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::aggregator::ClusterAggregator;

#[tokio::main]
async fn main() -> Result<()> {
    telemetry::init("liquidator");
    let cfg = AppConfig::load()?;
    tracing::info!("starting liquidator");

    let bus = RedisBus::connect(&cfg.redis_url).await?;
    let agg = Arc::new(RwLock::new(ClusterAggregator::new()));

    // Latest mid price (kept fresh by subscribing to CH_BOOK)
    let mid = Arc::new(RwLock::new(0.0_f64));

    // Channel for liquidations from all 3 streams
    let (tx, mut rx) = mpsc::unbounded_channel::<Liquidation>();

    // Spawn 3 stream tasks
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = streams::run_binance(tx.clone()).await {
                    tracing::error!(?e, "binance liq stream crashed");
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = streams::run_bybit(tx.clone()).await {
                    tracing::error!(?e, "bybit liq stream crashed");
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = streams::run_okx(tx.clone()).await {
                    tracing::error!(?e, "okx liq stream crashed");
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }

    // Subscribe to mid price from book channel
    {
        let mid = mid.clone();
        let url = cfg.redis_url.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = book_subscribe_loop(&url, mid.clone()).await {
                    tracing::error!(?e, "book sub crashed");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        });
    }

    // Ingest liquidations + publish them individually
    let mut bus_pub = bus.clone();
    let agg_ingest = agg.clone();
    tokio::spawn(async move {
        while let Some(liq) = rx.recv().await {
            tracing::info!(
                ex = %liq.exchange,
                side = ?liq.side,
                price = liq.price,
                notional = liq.notional,
                "LIQ"
            );
            agg_ingest.write().ingest(liq.clone());
            let _ = bus_pub.publish(CH_LIQ, &liq).await;
            let _ = bus_pub.lpush_capped(KEY_LIQ_RECENT, &liq, 200).await;
        }
    });

    // Snapshot publisher — every 1s
    let mut bus_snap = bus.clone();
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let m = *mid.read();
        if m <= 0.0 {
            continue;
        }
        let snapshot = agg.write().snapshot(m);
        let _ = bus_snap.set_json(KEY_CLUSTERS, &snapshot).await;
        let _ = bus_snap.publish(CH_CLUSTER, &snapshot).await;
    }
}

async fn book_subscribe_loop(url: &str, mid: Arc<RwLock<f64>>) -> Result<()> {
    let client = redis::Client::open(url)?;
    let mut pubsub = client.get_async_pubsub().await?;
    pubsub.subscribe(&[CH_BOOK]).await?;
    let mut msgs = pubsub.on_message();
    while let Some(msg) = msgs.next().await {
        let payload: String = msg.get_payload()?;
        if let Ok(b) = serde_json::from_str::<BookSnapshot>(&payload) {
            *mid.write() = b.mid;
        }
    }
    Ok(())
}

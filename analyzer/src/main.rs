mod features;
mod model;

use anyhow::Result;
use chrono::Utc;
use common::{
    telemetry, AppConfig, BookSnapshot, Prediction, RedisBus, TradeEvent, VacuumEvent, CH_BOOK,
    CH_PREDICT, CH_TRADE, CH_VACUUM, KEY_PREDICT,
};
use futures_util::StreamExt;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Duration;

use crate::features::FeatureStore;

#[tokio::main]
async fn main() -> Result<()> {
    telemetry::init("analyzer");
    let cfg = AppConfig::load()?;
    tracing::info!("starting analyzer");

    let bus = RedisBus::connect(&cfg.redis_url).await?;
    let store = Arc::new(RwLock::new(FeatureStore::new()));

    // Subscriber task — single connection multi-channel pubsub
    let store_sub = store.clone();
    let url_sub = cfg.redis_url.clone();
    tokio::spawn(async move {
        loop {
            if let Err(e) = subscribe_loop(&url_sub, store_sub.clone()).await {
                tracing::error!(?e, "subscribe loop error; reconnecting");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    });

    // Prediction publisher — runs every second
    let mut bus2 = bus.clone();
    let store_pred = store.clone();
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let pred_opt = {
            let s = store_pred.read();
            s.last_book.as_ref().map(|book| {
                let feats = s.compute_features();
                model::predict(book, &feats)
            })
        };
        if let Some(pred) = pred_opt {
            let _ = bus2.set_json(KEY_PREDICT, &pred).await;
            let _ = bus2.publish(CH_PREDICT, &pred).await;
        }
        let _ = Utc::now();
    }
}

async fn subscribe_loop(url: &str, store: Arc<RwLock<FeatureStore>>) -> Result<()> {
    let client = redis::Client::open(url)?;
    let mut pubsub = client.get_async_pubsub().await?;
    pubsub.subscribe(&[CH_BOOK, CH_TRADE, CH_VACUUM]).await?;
    let mut msgs = pubsub.on_message();
    while let Some(msg) = msgs.next().await {
        let chan: String = msg.get_channel_name().to_string();
        let payload: String = msg.get_payload()?;
        match chan.as_str() {
            CH_BOOK => {
                if let Ok(b) = serde_json::from_str::<BookSnapshot>(&payload) {
                    store.write().on_book(b);
                }
            }
            CH_TRADE => {
                if let Ok(t) = serde_json::from_str::<TradeEvent>(&payload) {
                    store.write().on_trade(t);
                }
            }
            CH_VACUUM => {
                if let Ok(v) = serde_json::from_str::<VacuumEvent>(&payload) {
                    store.write().on_vacuum(v);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

mod features;
mod model;

use anyhow::Result;
use common::{
    telemetry, AppConfig, BookSnapshot, RedisBus, ThesisStatus, TradeEvent, VacuumEvent, Wall,
    CH_BOOK, CH_PREDICT, CH_TRADE, CH_VACUUM, CH_WALL, KEY_HISTORY, KEY_PREDICT,
};
use futures_util::StreamExt;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Duration;

use crate::features::FeatureStore;
use crate::model::{build_payload, ThesisManager};

#[tokio::main]
async fn main() -> Result<()> {
    telemetry::init("analyzer");
    let cfg = AppConfig::load()?;
    tracing::info!("starting analyzer");

    let bus = RedisBus::connect(&cfg.redis_url).await?;
    let store = Arc::new(RwLock::new(FeatureStore::new()));
    let mgr = Arc::new(RwLock::new(ThesisManager::new()));

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

    let mut bus2 = bus.clone();
    let mut last_archived_id: Option<String> = None;

    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let book;
        let walls_vec;
        let feats;
        let new_vacs;
        {
            let mut s = store.write();
            let Some(b) = s.last_book.clone() else {
                continue;
            };
            book = b;
            walls_vec = s.walls.values().cloned().collect::<Vec<_>>();
            feats = s.compute();
            new_vacs = s.drain_unprocessed_vacuums();
        }

        let payload;
        let archived_to_record;
        {
            let mut m = mgr.write();
            m.tick(&book, &feats, &walls_vec, new_vacs);
            payload = build_payload(&m, &book, &feats);
            archived_to_record = m
                .last_archived
                .as_ref()
                .filter(|t| {
                    t.status != ThesisStatus::Active
                        && Some(&t.id) != last_archived_id.as_ref()
                })
                .cloned();
        }

        if let Some(arch) = archived_to_record {
            last_archived_id = Some(arch.id.clone());
            let _ = bus2.lpush_capped(KEY_HISTORY, &arch, 50).await;
            tracing::info!(
                id = %arch.id,
                status = ?arch.status,
                target = arch.target_price,
                "thesis closed"
            );
        }

        let _ = bus2.set_json(KEY_PREDICT, &payload).await;
        let _ = bus2.publish(CH_PREDICT, &payload).await;
    }
}

async fn subscribe_loop(url: &str, store: Arc<RwLock<FeatureStore>>) -> Result<()> {
    let client = redis::Client::open(url)?;
    let mut pubsub = client.get_async_pubsub().await?;
    pubsub
        .subscribe(&[CH_BOOK, CH_TRADE, CH_VACUUM, CH_WALL])
        .await?;
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
            CH_WALL => {
                if let Ok(walls) = serde_json::from_str::<Vec<Wall>>(&payload) {
                    store.write().on_walls(walls);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

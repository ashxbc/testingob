use anyhow::{anyhow, Result};
use backoff::backoff::Backoff;
use backoff::ExponentialBackoff;
use chrono::Utc;
use common::{
    AppConfig, BookSnapshot, RedisBus, TradeEvent, CH_BOOK, CH_TRADE, CH_VACUUM, CH_WALL,
    KEY_STATE, KEY_VACUUMS, KEY_WALLS,
};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::orderbook::{BinanceDepthUpdate, BinanceSnapshot, OrderBook};
use crate::walls::WallTracker;

pub async fn run(
    cfg: AppConfig,
    bus: RedisBus,
    book: Arc<Mutex<OrderBook>>,
    walls: Arc<Mutex<WallTracker>>,
) -> Result<()> {
    // Two concurrent tasks: depth stream and aggTrade stream.
    let cfg2 = cfg.clone();
    let bus2 = bus.clone();
    let book2 = book.clone();
    let walls2 = walls.clone();

    let depth_task = tokio::spawn(async move {
        loop {
            if let Err(e) = run_depth_loop(&cfg2, bus2.clone(), book2.clone(), walls2.clone()).await {
                tracing::error!(?e, "depth loop crashed; reconnecting");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    let trade_task = tokio::spawn(async move {
        loop {
            if let Err(e) = run_trade_loop(&cfg, bus.clone(), walls.clone()).await {
                tracing::error!(?e, "trade loop crashed; reconnecting");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    tokio::try_join!(depth_task, trade_task)?;
    Ok(())
}

async fn run_depth_loop(
    cfg: &AppConfig,
    mut bus: RedisBus,
    book: Arc<Mutex<OrderBook>>,
    walls: Arc<Mutex<WallTracker>>,
) -> Result<()> {
    let mut backoff = ExponentialBackoff {
        initial_interval: Duration::from_millis(500),
        max_interval: Duration::from_secs(30),
        max_elapsed_time: None,
        ..Default::default()
    };

    let symbol_lower = cfg.symbol.to_lowercase();
    let url = format!("{}/ws/{}@depth@100ms", cfg.binance_ws_base, symbol_lower);

    tracing::info!(%url, "connecting to depth stream");
    let (ws_stream, _) = connect_async(&url).await?;
    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    // Buffer deltas while we fetch the snapshot
    let mut buffered: Vec<BinanceDepthUpdate> = Vec::new();
    let mut snapshot_done = false;

    // Fetch REST snapshot
    let snapshot = fetch_snapshot(cfg).await?;
    let snapshot_id = snapshot.last_update_id;
    {
        let mut b = book.lock().await;
        b.apply_snapshot(snapshot);
    }
    tracing::info!(snapshot_id, "snapshot loaded");

    let mut last_publish = std::time::Instant::now();

    while let Some(msg) = ws_rx.next().await {
        let msg = msg?;
        match msg {
            Message::Text(text) => {
                let val: serde_json::Value = serde_json::from_str(&text)?;
                // Single-stream payload — depthUpdate event
                if val.get("e").and_then(|v| v.as_str()) != Some("depthUpdate") {
                    continue;
                }
                let upd: BinanceDepthUpdate = serde_json::from_value(val)?;

                if !snapshot_done {
                    if upd.final_update_id <= snapshot_id {
                        continue;
                    }
                    if upd.first_update_id > snapshot_id + 1 {
                        return Err(anyhow!("snapshot/delta gap; resync needed"));
                    }
                    buffered.push(upd);
                    let mut b = book.lock().await;
                    for u in buffered.drain(..) {
                        b.apply_delta(&u);
                    }
                    snapshot_done = true;
                    continue;
                }

                {
                    let mut b = book.lock().await;
                    b.apply_delta(&upd);
                }

                backoff.reset();

                // Throttle publishes to ~10/s
                if last_publish.elapsed() >= Duration::from_millis(100) {
                    last_publish = std::time::Instant::now();
                    publish_state(&mut bus, &book, &walls).await?;
                }
            }
            Message::Ping(p) => ws_tx.send(Message::Pong(p)).await?,
            Message::Close(_) => return Err(anyhow!("server closed")),
            _ => {}
        }
    }

    Err(anyhow!("ws stream ended"))
}

async fn fetch_snapshot(cfg: &AppConfig) -> Result<BinanceSnapshot> {
    let url = format!(
        "{}/api/v3/depth?symbol={}&limit={}",
        cfg.binance_rest_base, cfg.symbol, cfg.depth_levels
    );
    let snap = reqwest::Client::new()
        .get(&url)
        .send()
        .await?
        .json::<BinanceSnapshot>()
        .await?;
    Ok(snap)
}

async fn run_trade_loop(
    cfg: &AppConfig,
    mut bus: RedisBus,
    walls: Arc<Mutex<WallTracker>>,
) -> Result<()> {
    let symbol_lower = cfg.symbol.to_lowercase();
    let url = format!("{}/ws/{}@aggTrade", cfg.binance_ws_base, symbol_lower);
    tracing::info!(%url, "connecting to trade stream");
    let (ws_stream, _) = connect_async(&url).await?;
    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    while let Some(msg) = ws_rx.next().await {
        let msg = msg?;
        match msg {
            Message::Text(text) => {
                let v: serde_json::Value = serde_json::from_str(&text)?;
                if v.get("e").and_then(|x| x.as_str()) != Some("aggTrade") {
                    continue;
                }
                let price: f64 = v["p"].as_str().unwrap_or("0").parse().unwrap_or(0.0);
                let qty: f64 = v["q"].as_str().unwrap_or("0").parse().unwrap_or(0.0);
                let is_buyer_maker = v["m"].as_bool().unwrap_or(false);
                let ts = v["T"].as_i64().unwrap_or_else(|| Utc::now().timestamp_millis());

                {
                    let mut w = walls.lock().await;
                    w.record_trade(price, qty, ts);
                }

                let event = TradeEvent { ts, price, qty, is_buyer_maker };
                bus.publish(CH_TRADE, &event).await?;
            }
            Message::Ping(p) => ws_tx.send(Message::Pong(p)).await?,
            Message::Close(_) => return Err(anyhow!("trade stream closed")),
            _ => {}
        }
    }
    Err(anyhow!("trade stream ended"))
}

async fn publish_state(
    bus: &mut RedisBus,
    book: &Arc<Mutex<OrderBook>>,
    walls: &Arc<Mutex<WallTracker>>,
) -> Result<()> {
    let ts = Utc::now().timestamp_millis();
    let (snap, active_walls, vacuums) = {
        let b = book.lock().await;
        let Some(mid) = b.mid() else {
            return Ok(());
        };
        let best_bid = b.best_bid().unwrap_or(0.0);
        let best_ask = b.best_ask().unwrap_or(0.0);
        let spread_bps = if mid > 0.0 {
            ((best_ask - best_bid) / mid) * 10_000.0
        } else {
            0.0
        };
        let bid_depth = b.bid_depth_within(mid, 0.01);
        let ask_depth = b.ask_depth_within(mid, 0.01);

        let snap = BookSnapshot {
            ts,
            mid,
            best_bid,
            best_ask,
            spread_bps,
            bid_depth_1pct: bid_depth,
            ask_depth_1pct: ask_depth,
        };

        let mut w = walls.lock().await;
        let (active, vacs) = w.reconcile(&b, ts);
        (snap, active, vacs)
    };

    bus.set_json(KEY_STATE, &snap).await?;
    bus.publish(CH_BOOK, &snap).await?;

    bus.set_json(KEY_WALLS, &active_walls).await?;
    bus.publish(CH_WALL, &active_walls).await?;

    for v in &vacuums {
        bus.publish(CH_VACUUM, v).await?;
        bus.lpush_capped(KEY_VACUUMS, v, 200).await?;
        tracing::info!(side = ?v.side, price = v.price, notional = v.notional_pulled, reason = ?v.reason, "VACUUM");
    }

    Ok(())
}

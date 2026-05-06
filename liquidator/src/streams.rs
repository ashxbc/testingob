use anyhow::{anyhow, Result};
use chrono::Utc;
use common::{LiqSide, Liquidation};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc::UnboundedSender;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

// ============================================================
// Binance Futures: forced order stream
//   wss://fstream.binance.com/ws/btcusdt@forceOrder
//   { "e":"forceOrder","E":...,
//     "o":{"s":"BTCUSDT","S":"SELL"|"BUY","p":"price","q":"qty","T":timestamp_ms,...} }
// SELL side = long got liquidated (forced sell)
// BUY  side = short got liquidated (forced buy)
// ============================================================

pub async fn run_binance(tx: UnboundedSender<Liquidation>) -> Result<()> {
    let url = "wss://fstream.binance.com/ws/btcusdt@forceOrder";
    tracing::info!(%url, "binance liq connecting");
    let (ws, _) = connect_async(url).await?;
    let (mut wtx, mut wrx) = ws.split();

    while let Some(msg) = wrx.next().await {
        match msg? {
            Message::Text(t) => {
                let v: serde_json::Value = match serde_json::from_str(&t) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if v.get("e").and_then(|x| x.as_str()) != Some("forceOrder") {
                    continue;
                }
                let o = match v.get("o") {
                    Some(x) => x,
                    None => continue,
                };
                let side_raw = o.get("S").and_then(|x| x.as_str()).unwrap_or("");
                let price: f64 = o
                    .get("p")
                    .and_then(|x| x.as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.0);
                let qty: f64 = o
                    .get("q")
                    .and_then(|x| x.as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.0);
                let ts: i64 = o
                    .get("T")
                    .and_then(|x| x.as_i64())
                    .unwrap_or_else(|| Utc::now().timestamp_millis());
                if price <= 0.0 || qty <= 0.0 {
                    continue;
                }
                let side = match side_raw {
                    "SELL" => LiqSide::Long,
                    "BUY" => LiqSide::Short,
                    _ => continue,
                };
                let liq = Liquidation {
                    ts,
                    exchange: "binance".to_string(),
                    side,
                    price,
                    qty,
                    notional: price * qty,
                };
                let _ = tx.send(liq);
            }
            Message::Ping(p) => wtx.send(Message::Pong(p)).await?,
            Message::Close(_) => return Err(anyhow!("binance closed")),
            _ => {}
        }
    }
    Err(anyhow!("binance liq stream ended"))
}

// ============================================================
// Bybit V5: liquidation stream
//   wss://stream.bybit.com/v5/public/linear
//   subscribe: {"op":"subscribe","args":["liquidation.BTCUSDT"]}
//   data: {"topic":"liquidation.BTCUSDT","data":{"updatedTime":..,"symbol":"BTCUSDT",
//          "side":"Buy"|"Sell","size":"qty","price":"price"}}
// Bybit "side" = the side that liquidated. "Buy" means short was liquidated (forced buy).
//                                          "Sell" means long was liquidated (forced sell).
// ============================================================

pub async fn run_bybit(tx: UnboundedSender<Liquidation>) -> Result<()> {
    let url = "wss://stream.bybit.com/v5/public/linear";
    tracing::info!(%url, "bybit liq connecting");
    let (ws, _) = connect_async(url).await?;
    let (mut wtx, mut wrx) = ws.split();

    let sub = serde_json::json!({
        "op":"subscribe",
        "args":["liquidation.BTCUSDT"]
    });
    wtx.send(Message::Text(sub.to_string())).await?;

    // Heartbeat task — Bybit requires ping every 20s
    // We use a simple periodic flush via tokio interval
    let ping_task = tokio::spawn(async move {
        let mut int = tokio::time::interval(std::time::Duration::from_secs(20));
        int.tick().await;
        loop {
            int.tick().await;
            if let Err(_) = wtx
                .send(Message::Text(r#"{"op":"ping"}"#.to_string()))
                .await
            {
                break;
            }
        }
    });

    let result: Result<()> = async {
        while let Some(msg) = wrx.next().await {
            match msg? {
                Message::Text(t) => {
                    let v: serde_json::Value = match serde_json::from_str(&t) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let topic = v.get("topic").and_then(|x| x.as_str()).unwrap_or("");
                    if !topic.starts_with("liquidation.") {
                        continue;
                    }
                    let data = match v.get("data") {
                        Some(x) => x,
                        None => continue,
                    };
                    // data may be object or array depending on bybit version
                    let entries: Vec<&serde_json::Value> = if data.is_array() {
                        data.as_array().unwrap().iter().collect()
                    } else {
                        vec![data]
                    };
                    for d in entries {
                        let side_raw = d.get("side").and_then(|x| x.as_str()).unwrap_or("");
                        let price: f64 = d
                            .get("price")
                            .and_then(|x| x.as_str())
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0.0);
                        let qty: f64 = d
                            .get("size")
                            .and_then(|x| x.as_str())
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0.0);
                        let ts: i64 = d
                            .get("updatedTime")
                            .and_then(|x| x.as_str())
                            .and_then(|s| s.parse().ok())
                            .unwrap_or_else(|| Utc::now().timestamp_millis());
                        if price <= 0.0 || qty <= 0.0 {
                            continue;
                        }
                        let side = match side_raw {
                            "Sell" => LiqSide::Long,
                            "Buy" => LiqSide::Short,
                            _ => continue,
                        };
                        let liq = Liquidation {
                            ts,
                            exchange: "bybit".to_string(),
                            side,
                            price,
                            qty,
                            notional: price * qty,
                        };
                        let _ = tx.send(liq);
                    }
                }
                Message::Close(_) => return Err(anyhow!("bybit closed")),
                _ => {}
            }
        }
        Err(anyhow!("bybit liq stream ended"))
    }
    .await;

    ping_task.abort();
    result
}

// ============================================================
// OKX V5: liquidation orders
//   wss://ws.okx.com:8443/ws/v5/public
//   subscribe: {"op":"subscribe","args":[{"channel":"liquidation-orders","instType":"SWAP","instFamily":"BTC-USDT"}]}
//   data: { "data":[{ "instId":"BTC-USDT-SWAP","details":[
//             {"side":"sell"|"buy","sz":"qty","bkPx":"price","ts":"timestamp_ms"}
//          ]}] }
// OKX "side" = posSide that was closed. "sell" -> long liquidated. "buy" -> short liquidated.
// `sz` for SWAP is in contracts; for BTC-USDT-SWAP, 1 contract = 0.01 BTC.
// ============================================================

const OKX_CONTRACT_MULTIPLIER: f64 = 0.01;

pub async fn run_okx(tx: UnboundedSender<Liquidation>) -> Result<()> {
    let url = "wss://ws.okx.com:8443/ws/v5/public";
    tracing::info!(%url, "okx liq connecting");
    let (ws, _) = connect_async(url).await?;
    let (mut wtx, mut wrx) = ws.split();

    let sub = serde_json::json!({
        "op":"subscribe",
        "args":[{
            "channel":"liquidation-orders",
            "instType":"SWAP",
            "instFamily":"BTC-USDT"
        }]
    });
    wtx.send(Message::Text(sub.to_string())).await?;

    while let Some(msg) = wrx.next().await {
        match msg? {
            Message::Text(t) => {
                if t == "pong" {
                    continue;
                }
                let v: serde_json::Value = match serde_json::from_str(&t) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let arr = match v.get("data").and_then(|d| d.as_array()) {
                    Some(a) => a,
                    None => continue,
                };
                for item in arr {
                    let inst_id = item.get("instId").and_then(|x| x.as_str()).unwrap_or("");
                    if !inst_id.starts_with("BTC-USDT") {
                        continue;
                    }
                    let details = match item.get("details").and_then(|d| d.as_array()) {
                        Some(a) => a,
                        None => continue,
                    };
                    for d in details {
                        let side_raw = d.get("side").and_then(|x| x.as_str()).unwrap_or("");
                        let price: f64 = d
                            .get("bkPx")
                            .and_then(|x| x.as_str())
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0.0);
                        let sz: f64 = d
                            .get("sz")
                            .and_then(|x| x.as_str())
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0.0);
                        let qty = sz * OKX_CONTRACT_MULTIPLIER;
                        let ts: i64 = d
                            .get("ts")
                            .and_then(|x| x.as_str())
                            .and_then(|s| s.parse().ok())
                            .unwrap_or_else(|| Utc::now().timestamp_millis());
                        if price <= 0.0 || qty <= 0.0 {
                            continue;
                        }
                        let side = match side_raw {
                            "sell" => LiqSide::Long,
                            "buy" => LiqSide::Short,
                            _ => continue,
                        };
                        let liq = Liquidation {
                            ts,
                            exchange: "okx".to_string(),
                            side,
                            price,
                            qty,
                            notional: price * qty,
                        };
                        let _ = tx.send(liq);
                    }
                }
            }
            Message::Ping(p) => wtx.send(Message::Pong(p)).await?,
            Message::Close(_) => return Err(anyhow!("okx closed")),
            _ => {}
        }
    }
    Err(anyhow!("okx liq stream ended"))
}

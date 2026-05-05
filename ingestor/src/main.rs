mod orderbook;
mod walls;
mod stream;

use anyhow::Result;
use common::{telemetry, AppConfig, RedisBus};
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> Result<()> {
    telemetry::init("ingestor");
    let cfg = AppConfig::load()?;
    tracing::info!(?cfg.symbol, "starting ingestor");

    let bus = RedisBus::connect(&cfg.redis_url).await?;
    let book = Arc::new(Mutex::new(orderbook::OrderBook::new()));
    let wall_tracker = Arc::new(Mutex::new(walls::WallTracker::new(&cfg)));

    stream::run(cfg, bus, book, wall_tracker).await
}

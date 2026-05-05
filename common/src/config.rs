use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub redis_url: String,
    pub symbol: String,
    pub binance_ws_base: String,
    pub binance_rest_base: String,
    pub depth_levels: usize,
    pub wall_min_notional_usd: f64,
    pub wall_relative_multiplier: f64,
    pub vacuum_window_ms: i64,
    pub api_bind: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            redis_url: "redis://127.0.0.1:6379".to_string(),
            symbol: "BTCUSDT".to_string(),
            binance_ws_base: "wss://stream.binance.com:9443".to_string(),
            binance_rest_base: "https://api.binance.com".to_string(),
            depth_levels: 1000,
            wall_min_notional_usd: 1_500_000.0,
            wall_relative_multiplier: 5.0,
            vacuum_window_ms: 1500,
            api_bind: "0.0.0.0:8080".to_string(),
        }
    }
}

impl AppConfig {
    pub fn load() -> anyhow::Result<Self> {
        let cfg = config::Config::builder()
            .add_source(config::Config::try_from(&AppConfig::default())?)
            .add_source(config::File::with_name("config").required(false))
            .add_source(config::Environment::with_prefix("LV").separator("__"))
            .build()?;
        Ok(cfg.try_deserialize()?)
    }
}

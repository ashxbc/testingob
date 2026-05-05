use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub fn init(service: &str) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tokio_tungstenite=warn,tungstenite=warn"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false).with_ansi(false))
        .init();

    tracing::info!(service = service, "telemetry initialized");
}

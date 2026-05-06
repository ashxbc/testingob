use anyhow::Result;
use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use common::{
    telemetry, AppConfig, BookSnapshot, ClusterSnapshot, Liquidation, PredictPayload, RedisBus,
    Thesis, VacuumEvent, Wall, CH_BOOK, CH_CLUSTER, CH_LIQ, CH_PREDICT, CH_VACUUM, CH_WALL,
    KEY_CLUSTERS, KEY_HISTORY, KEY_LIQ_RECENT, KEY_PREDICT, KEY_STATE, KEY_VACUUMS, KEY_WALLS,
};
use futures_util::StreamExt;
use redis::AsyncCommands;
use std::convert::Infallible;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::compression::CompressionLayer;

#[derive(Clone)]
struct AppState {
    bus: RedisBus,
    redis_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    telemetry::init("api");
    let cfg = AppConfig::load()?;
    let bus = RedisBus::connect(&cfg.redis_url).await?;
    let state = AppState {
        bus,
        redis_url: cfg.redis_url.clone(),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/state", get(get_state))
        .route("/api/walls", get(get_walls))
        .route("/api/vacuums", get(get_vacuums))
        .route("/api/predict", get(get_predict))
        .route("/api/history", get(get_history))
        .route("/api/clusters", get(get_clusters))
        .route("/api/liquidations", get(get_liquidations))
        .route("/api/stream", get(stream))
        .layer(CompressionLayer::new())
        .layer(cors)
        .with_state(Arc::new(state));

    let listener = tokio::net::TcpListener::bind(&cfg.api_bind).await?;
    tracing::info!(addr = %cfg.api_bind, "api listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

async fn get_state(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    fetch_json::<BookSnapshot>(&s, KEY_STATE).await
}

async fn get_walls(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    fetch_json::<Vec<Wall>>(&s, KEY_WALLS).await
}

async fn get_predict(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    fetch_json::<PredictPayload>(&s, KEY_PREDICT).await
}

async fn get_history(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    let mut conn = s.bus.conn.clone();
    let raws: Vec<String> = conn.lrange(KEY_HISTORY, 0, 50).await.unwrap_or_default();
    let parsed: Vec<Thesis> = raws
        .into_iter()
        .filter_map(|r| serde_json::from_str::<Thesis>(&r).ok())
        .collect();
    Json(parsed)
}

async fn get_clusters(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    fetch_json::<ClusterSnapshot>(&s, KEY_CLUSTERS).await
}

async fn get_liquidations(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    let mut conn = s.bus.conn.clone();
    let raws: Vec<String> = conn.lrange(KEY_LIQ_RECENT, 0, 100).await.unwrap_or_default();
    let parsed: Vec<Liquidation> = raws
        .into_iter()
        .filter_map(|r| serde_json::from_str::<Liquidation>(&r).ok())
        .collect();
    Json(parsed)
}

async fn get_vacuums(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    let mut conn = s.bus.conn.clone();
    let raws: Vec<String> = conn.lrange(KEY_VACUUMS, 0, 50).await.unwrap_or_default();
    let parsed: Vec<VacuumEvent> = raws
        .into_iter()
        .filter_map(|r| serde_json::from_str::<VacuumEvent>(&r).ok())
        .collect();
    Json(parsed)
}

async fn fetch_json<T: serde::de::DeserializeOwned + serde::Serialize>(
    s: &AppState,
    key: &str,
) -> Json<serde_json::Value> {
    let mut conn = s.bus.conn.clone();
    let raw: Option<String> = conn.get(key).await.unwrap_or(None);
    match raw.and_then(|r| serde_json::from_str::<T>(&r).ok()) {
        Some(v) => Json(serde_json::to_value(v).unwrap()),
        None => Json(serde_json::Value::Null),
    }
}

async fn stream(
    State(s): State<Arc<AppState>>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let url = s.redis_url.clone();
    let stream = async_stream::stream! {
        loop {
            let client = match redis::Client::open(url.clone()) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(?e, "redis client open failed in sse");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            };
            let mut pubsub = match client.get_async_pubsub().await {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!(?e, "redis pubsub connect failed");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            };
            if let Err(e) = pubsub
                .subscribe(&[CH_BOOK, CH_VACUUM, CH_WALL, CH_PREDICT, CH_CLUSTER, CH_LIQ])
                .await
            {
                tracing::error!(?e, "subscribe failed");
                continue;
            }

            let mut msgs = pubsub.on_message();
            while let Some(msg) = msgs.next().await {
                let chan = msg.get_channel_name().to_string();
                let payload: String = match msg.get_payload() {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                let event_name = match chan.as_str() {
                    CH_BOOK => "book",
                    CH_VACUUM => "vacuum",
                    CH_WALL => "walls",
                    CH_PREDICT => "predict",
                    CH_CLUSTER => "clusters",
                    CH_LIQ => "liq",
                    _ => continue,
                };
                yield Ok(Event::default().event(event_name).data(payload));
            }
        }
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}

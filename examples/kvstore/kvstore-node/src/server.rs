use std::net::SocketAddr;

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::get,
};
use serde::{Deserialize, Serialize};
use tower_http::trace::TraceLayer;

use crate::{
    config::KvConfig,
    state::{KvState, Snapshot, ValueRecord},
};

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Deserialize)]
struct PutRequest {
    value: String,
    expected_version: Option<u64>,
}

#[derive(Serialize)]
struct PutResponse {
    applied: bool,
    version: u64,
}

#[derive(Serialize)]
struct GetResponse {
    key: String,
    record: Option<ValueRecord>,
}

pub async fn start_server(config: KvConfig, state: KvState) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/health/live", get(health_live))
        .route("/health/ready", get(health_ready))
        .route("/kv/:key", get(get_key).put(put_key))
        .route("/internal/snapshot", get(get_snapshot))
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let addr = SocketAddr::from(([0, 0, 0, 0], config.http_port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    state.set_ready(true).await;
    tracing::info!(node_id = state.node_id(), %addr, "kv node ready");

    axum::serve(listener, app).await?;
    Ok(())
}

async fn health_live() -> (StatusCode, Json<HealthResponse>) {
    (StatusCode::OK, Json(HealthResponse { status: "alive" }))
}

async fn health_ready(State(state): State<KvState>) -> (StatusCode, Json<HealthResponse>) {
    if state.is_ready().await {
        (StatusCode::OK, Json(HealthResponse { status: "ready" }))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                status: "not-ready",
            }),
        )
    }
}

async fn get_key(Path(key): Path<String>, State(state): State<KvState>) -> Json<GetResponse> {
    let record = state.get(&key).await;
    Json(GetResponse { key, record })
}

async fn put_key(
    Path(key): Path<String>,
    State(state): State<KvState>,
    Json(request): Json<PutRequest>,
) -> (StatusCode, Json<PutResponse>) {
    let outcome = state
        .put_local(key, request.value, request.expected_version)
        .await;

    if outcome.applied {
        (
            StatusCode::OK,
            Json(PutResponse {
                applied: true,
                version: outcome.current_version,
            }),
        )
    } else {
        (
            StatusCode::CONFLICT,
            Json(PutResponse {
                applied: false,
                version: outcome.current_version,
            }),
        )
    }
}

async fn get_snapshot(State(state): State<KvState>) -> Json<Snapshot> {
    Json(state.snapshot().await)
}

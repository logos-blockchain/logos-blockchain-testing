use std::net::SocketAddr;

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use prometheus::{Encoder, TextEncoder};
use serde::Serialize;
use tower_http::trace::TraceLayer;

use crate::{
    config::CounterConfig,
    state::{CounterState, CounterView},
};

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

pub async fn start_server(config: CounterConfig, state: CounterState) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/health/live", get(health_live))
        .route("/health/ready", get(health_ready))
        .route("/counter/inc", post(increment))
        .route("/counter/value", get(counter_value))
        .route("/metrics", get(metrics))
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let addr = SocketAddr::from(([0, 0, 0, 0], config.http_port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    state.set_ready(true).await;
    tracing::info!(node_id = config.node_id, %addr, "metrics-counter node ready");

    axum::serve(listener, app).await?;
    Ok(())
}

async fn health_live() -> (StatusCode, Json<HealthResponse>) {
    (StatusCode::OK, Json(HealthResponse { status: "alive" }))
}

async fn health_ready(State(state): State<CounterState>) -> (StatusCode, Json<HealthResponse>) {
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

async fn increment(State(state): State<CounterState>) -> Json<CounterView> {
    Json(state.increment())
}

async fn counter_value(State(state): State<CounterState>) -> Json<CounterView> {
    Json(state.view())
}

async fn metrics() -> Response {
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    let encoder = TextEncoder::new();
    if encoder.encode(&metric_families, &mut buffer).is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    (
        StatusCode::OK,
        [("content-type", encoder.format_type().to_string())],
        buffer,
    )
        .into_response()
}

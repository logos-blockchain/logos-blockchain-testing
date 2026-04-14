use std::net::SocketAddr;

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tower_http::trace::TraceLayer;

use crate::{
    config::QueueConfig,
    state::{QueueMessage, QueueRevision, QueueState, QueueStateView, Snapshot},
};

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Deserialize)]
struct EnqueueRequest {
    payload: String,
}

#[derive(Serialize)]
struct EnqueueResponse {
    accepted: bool,
    id: u64,
    queue_len: usize,
    revision: QueueRevision,
}

#[derive(Serialize)]
struct DequeueResponse {
    message: Option<QueueMessage>,
    queue_len: usize,
    revision: QueueRevision,
}

pub async fn start_server(config: QueueConfig, state: QueueState) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/health/live", get(health_live))
        .route("/health/ready", get(health_ready))
        .route("/queue/enqueue", post(enqueue))
        .route("/queue/dequeue", post(dequeue))
        .route("/queue/state", get(queue_state))
        .route("/internal/snapshot", get(get_snapshot))
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let addr = SocketAddr::from(([0, 0, 0, 0], config.http_port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    state.set_ready(true).await;
    tracing::info!(node_id = state.node_id(), %addr, "queue node ready");

    axum::serve(listener, app).await?;
    Ok(())
}

async fn health_live() -> (StatusCode, Json<HealthResponse>) {
    (StatusCode::OK, Json(HealthResponse { status: "alive" }))
}

async fn health_ready(State(state): State<QueueState>) -> (StatusCode, Json<HealthResponse>) {
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

async fn enqueue(
    State(state): State<QueueState>,
    Json(request): Json<EnqueueRequest>,
) -> (StatusCode, Json<EnqueueResponse>) {
    let outcome = state.enqueue_local(request.payload).await;
    (
        StatusCode::OK,
        Json(EnqueueResponse {
            accepted: outcome.accepted,
            id: outcome.id,
            queue_len: outcome.queue_len,
            revision: outcome.revision,
        }),
    )
}

async fn dequeue(State(state): State<QueueState>) -> (StatusCode, Json<DequeueResponse>) {
    let outcome = state.dequeue_local().await;
    (
        StatusCode::OK,
        Json(DequeueResponse {
            message: outcome.message,
            queue_len: outcome.queue_len,
            revision: outcome.revision,
        }),
    )
}

async fn queue_state(State(state): State<QueueState>) -> Json<QueueStateView> {
    Json(state.queue_state().await)
}

async fn get_snapshot(State(state): State<QueueState>) -> Json<Snapshot> {
    Json(state.snapshot().await)
}

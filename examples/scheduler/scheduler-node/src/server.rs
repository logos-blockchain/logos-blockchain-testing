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
    config::SchedulerConfig,
    state::{SchedulerState, Snapshot, StateView},
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
    id: u64,
}

#[derive(Deserialize)]
struct ClaimRequest {
    worker_id: String,
    max_jobs: usize,
}

#[derive(Serialize)]
struct ClaimResponse {
    jobs: Vec<ClaimedJob>,
}

#[derive(Serialize)]
struct ClaimedJob {
    id: u64,
    payload: String,
    attempt: u32,
}

#[derive(Deserialize)]
struct HeartbeatRequest {
    worker_id: String,
    job_id: u64,
}

#[derive(Deserialize)]
struct AckRequest {
    worker_id: String,
    job_id: u64,
}

#[derive(Serialize)]
struct OperationResponse {
    ok: bool,
}

pub async fn start_server(config: SchedulerConfig, state: SchedulerState) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/health/live", get(health_live))
        .route("/health/ready", get(health_ready))
        .route("/jobs/enqueue", post(enqueue))
        .route("/jobs/claim", post(claim))
        .route("/jobs/heartbeat", post(heartbeat))
        .route("/jobs/ack", post(ack))
        .route("/jobs/state", get(state_view))
        .route("/internal/snapshot", get(snapshot))
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let addr = SocketAddr::from(([0, 0, 0, 0], config.http_port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    state.set_ready(true).await;
    tracing::info!(node_id = state.node_id(), %addr, "scheduler node ready");

    axum::serve(listener, app).await?;
    Ok(())
}

async fn health_live() -> (StatusCode, Json<HealthResponse>) {
    (StatusCode::OK, Json(HealthResponse { status: "alive" }))
}

async fn health_ready(State(state): State<SchedulerState>) -> (StatusCode, Json<HealthResponse>) {
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
    State(state): State<SchedulerState>,
    Json(request): Json<EnqueueRequest>,
) -> (StatusCode, Json<EnqueueResponse>) {
    let id = state.enqueue(request.payload).await;
    (StatusCode::OK, Json(EnqueueResponse { id }))
}

async fn claim(
    State(state): State<SchedulerState>,
    Json(request): Json<ClaimRequest>,
) -> (StatusCode, Json<ClaimResponse>) {
    let result = state.claim(request.worker_id, request.max_jobs).await;
    let jobs = result
        .jobs
        .into_iter()
        .map(|job| ClaimedJob {
            id: job.id,
            payload: job.payload,
            attempt: job.attempt,
        })
        .collect();

    (StatusCode::OK, Json(ClaimResponse { jobs }))
}

async fn heartbeat(
    State(state): State<SchedulerState>,
    Json(request): Json<HeartbeatRequest>,
) -> (StatusCode, Json<OperationResponse>) {
    let ok = state.heartbeat(&request.worker_id, request.job_id).await;
    (StatusCode::OK, Json(OperationResponse { ok }))
}

async fn ack(
    State(state): State<SchedulerState>,
    Json(request): Json<AckRequest>,
) -> (StatusCode, Json<OperationResponse>) {
    let ok = state.ack(&request.worker_id, request.job_id).await;
    (StatusCode::OK, Json(OperationResponse { ok }))
}

async fn state_view(State(state): State<SchedulerState>) -> Json<StateView> {
    Json(state.state_view().await)
}

async fn snapshot(State(state): State<SchedulerState>) -> Json<Snapshot> {
    Json(state.snapshot().await)
}

use std::{io, net::Ipv4Addr, sync::Arc};

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::oneshot::channel;

use crate::repo::{CfgSyncErrorResponse, ConfigRepo, RepoResponse};

#[derive(Serialize, Deserialize)]
pub struct ClientIp {
    /// Node IP that can be used by clients for observability/logging.
    pub ip: Ipv4Addr,
    /// Stable node identifier used as key in cfgsync bundle lookup.
    pub identifier: String,
}

pub struct CfgSyncState {
    repo: Arc<ConfigRepo>,
}

impl CfgSyncState {
    #[must_use]
    pub fn new(repo: Arc<ConfigRepo>) -> Self {
        Self { repo }
    }
}

#[derive(Debug, Error)]
pub enum RunCfgsyncError {
    #[error("failed to bind cfgsync server on {bind_addr}: {source}")]
    Bind {
        bind_addr: String,
        #[source]
        source: io::Error,
    },
    #[error("cfgsync server terminated unexpectedly: {source}")]
    Serve {
        #[source]
        source: io::Error,
    },
}

async fn node_config(
    State(state): State<Arc<CfgSyncState>>,
    Json(payload): Json<ClientIp>,
) -> impl IntoResponse {
    let identifier = payload.identifier.clone();
    let (reply_tx, reply_rx) = channel();
    state.repo.register(identifier, reply_tx).await;

    match reply_rx.await {
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(CfgSyncErrorResponse::internal(
                "error receiving config from repo",
            )),
        )
            .into_response(),
        Ok(RepoResponse::Config(payload_data)) => {
            (StatusCode::OK, Json(payload_data)).into_response()
        }

        Ok(RepoResponse::Error(error)) => {
            let status = match error.code {
                crate::repo::CfgSyncErrorCode::MissingConfig => StatusCode::NOT_FOUND,
                crate::repo::CfgSyncErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (status, Json(error)).into_response()
        }
    }
}

pub fn cfgsync_app(state: CfgSyncState) -> Router {
    Router::new()
        .route("/node", post(node_config))
        .route("/init-with-node", post(node_config))
        .with_state(Arc::new(state))
}

pub async fn run_cfgsync(port: u16, state: CfgSyncState) -> Result<(), RunCfgsyncError> {
    let app = cfgsync_app(state);
    println!("Server running on http://0.0.0.0:{port}");

    let bind_addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .map_err(|source| RunCfgsyncError::Bind { bind_addr, source })?;

    axum::serve(listener, app)
        .await
        .map_err(|source| RunCfgsyncError::Serve { source })?;

    Ok(())
}

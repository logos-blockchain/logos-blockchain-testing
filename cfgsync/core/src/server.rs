use std::{io, net::Ipv4Addr, sync::Arc};

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::repo::{CfgSyncErrorCode, ConfigProvider, RepoResponse};

/// Request payload used by cfgsync client for node config resolution.
#[derive(Serialize, Deserialize)]
pub struct ClientIp {
    /// Node IP that can be used by clients for observability/logging.
    pub ip: Ipv4Addr,
    /// Stable node identifier used as key in cfgsync bundle lookup.
    pub identifier: String,
}

/// Runtime state shared across cfgsync HTTP handlers.
pub struct CfgSyncState {
    repo: Arc<dyn ConfigProvider>,
}

impl CfgSyncState {
    #[must_use]
    pub fn new(repo: Arc<dyn ConfigProvider>) -> Self {
        Self { repo }
    }
}

/// Fatal runtime failures when serving cfgsync HTTP endpoints.
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
    let response = resolve_node_config_response(&state, &payload.identifier);

    match response {
        RepoResponse::Config(payload_data) => (StatusCode::OK, Json(payload_data)).into_response(),
        RepoResponse::Error(error) => {
            let status = error_status(&error.code);

            (status, Json(error)).into_response()
        }
    }
}

fn resolve_node_config_response(state: &CfgSyncState, identifier: &str) -> RepoResponse {
    state.repo.resolve(identifier)
}

fn error_status(code: &CfgSyncErrorCode) -> StatusCode {
    match code {
        CfgSyncErrorCode::MissingConfig => StatusCode::NOT_FOUND,
        CfgSyncErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub fn cfgsync_app(state: CfgSyncState) -> Router {
    Router::new()
        .route("/node", post(node_config))
        .route("/init-with-node", post(node_config))
        .with_state(Arc::new(state))
}

/// Runs cfgsync HTTP server on the provided port until shutdown/error.
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

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

    use super::{CfgSyncState, ClientIp, node_config};
    use crate::repo::{
        CFGSYNC_SCHEMA_VERSION, CfgSyncErrorCode, CfgSyncErrorResponse, CfgSyncFile,
        CfgSyncPayload, ConfigProvider, RepoResponse,
    };

    struct StaticProvider {
        data: HashMap<String, CfgSyncPayload>,
    }

    impl ConfigProvider for StaticProvider {
        fn resolve(&self, identifier: &str) -> RepoResponse {
            self.data.get(identifier).cloned().map_or_else(
                || RepoResponse::Error(CfgSyncErrorResponse::missing_config(identifier)),
                RepoResponse::Config,
            )
        }
    }

    fn sample_payload() -> CfgSyncPayload {
        CfgSyncPayload {
            schema_version: CFGSYNC_SCHEMA_VERSION,
            files: vec![CfgSyncFile::new("/app-config.yaml", "app: test")],
        }
    }

    #[tokio::test]
    async fn node_config_resolves_from_non_tf_provider() {
        let mut data = HashMap::new();
        data.insert("node-a".to_owned(), sample_payload());

        let provider = Arc::new(StaticProvider { data });
        let state = Arc::new(CfgSyncState::new(provider));
        let payload = ClientIp {
            ip: "127.0.0.1".parse().expect("valid ip"),
            identifier: "node-a".to_owned(),
        };

        let response = node_config(State(state), Json(payload))
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn node_config_returns_not_found_for_unknown_identifier() {
        let provider = Arc::new(StaticProvider {
            data: HashMap::new(),
        });
        let state = Arc::new(CfgSyncState::new(provider));
        let payload = ClientIp {
            ip: "127.0.0.1".parse().expect("valid ip"),
            identifier: "missing-node".to_owned(),
        };

        let response = node_config(State(state), Json(payload))
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn missing_config_error_uses_expected_code() {
        let error = CfgSyncErrorResponse::missing_config("missing-node");

        assert!(matches!(error.code, CfgSyncErrorCode::MissingConfig));
    }
}

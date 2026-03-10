use std::{io, sync::Arc};

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
use thiserror::Error;

use crate::repo::{
    CfgsyncErrorCode, ConfigResolveResponse, NodeConfigSource, NodeRegistration,
    RegisterNodeResponse,
};

/// Runtime state shared across cfgsync HTTP handlers.
pub struct CfgsyncServerState {
    repo: Arc<dyn NodeConfigSource>,
}

impl CfgsyncServerState {
    #[must_use]
    pub fn new(repo: Arc<dyn NodeConfigSource>) -> Self {
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
    State(state): State<Arc<CfgsyncServerState>>,
    Json(payload): Json<NodeRegistration>,
) -> impl IntoResponse {
    let response = resolve_node_config_response(&state, &payload);

    match response {
        ConfigResolveResponse::Config(payload_data) => {
            (StatusCode::OK, Json(payload_data)).into_response()
        }
        ConfigResolveResponse::Error(error) => {
            let status = error_status(&error.code);

            (status, Json(error)).into_response()
        }
    }
}

async fn register_node(
    State(state): State<Arc<CfgsyncServerState>>,
    Json(payload): Json<NodeRegistration>,
) -> impl IntoResponse {
    match state.repo.register(payload) {
        RegisterNodeResponse::Registered => StatusCode::ACCEPTED.into_response(),
        RegisterNodeResponse::Error(error) => {
            let status = error_status(&error.code);

            (status, Json(error)).into_response()
        }
    }
}

fn resolve_node_config_response(
    state: &CfgsyncServerState,
    registration: &NodeRegistration,
) -> ConfigResolveResponse {
    state.repo.resolve(registration)
}

fn error_status(code: &CfgsyncErrorCode) -> StatusCode {
    match code {
        CfgsyncErrorCode::MissingConfig => StatusCode::NOT_FOUND,
        CfgsyncErrorCode::NotReady => StatusCode::TOO_EARLY,
        CfgsyncErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub fn build_cfgsync_router(state: CfgsyncServerState) -> Router {
    Router::new()
        .route("/register", post(register_node))
        .route("/node", post(node_config))
        .route("/init-with-node", post(node_config))
        .with_state(Arc::new(state))
}

/// Runs cfgsync HTTP server on the provided port until shutdown/error.
pub async fn serve_cfgsync(port: u16, state: CfgsyncServerState) -> Result<(), RunCfgsyncError> {
    let app = build_cfgsync_router(state);
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

#[doc(hidden)]
pub type CfgSyncState = CfgsyncServerState;

#[doc(hidden)]
pub use build_cfgsync_router as cfgsync_app;
#[doc(hidden)]
pub use serve_cfgsync as run_cfgsync;

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

    use super::{CfgsyncServerState, NodeRegistration, node_config, register_node};
    use crate::repo::{
        CFGSYNC_SCHEMA_VERSION, CfgsyncErrorCode, CfgsyncErrorResponse, ConfigResolveResponse,
        NodeArtifactFile, NodeArtifactsPayload, NodeConfigSource, RegisterNodeResponse,
    };

    struct StaticProvider {
        data: HashMap<String, NodeArtifactsPayload>,
    }

    impl NodeConfigSource for StaticProvider {
        fn register(&self, registration: NodeRegistration) -> RegisterNodeResponse {
            if self.data.contains_key(&registration.identifier) {
                RegisterNodeResponse::Registered
            } else {
                RegisterNodeResponse::Error(CfgsyncErrorResponse::missing_config(
                    &registration.identifier,
                ))
            }
        }

        fn resolve(&self, registration: &NodeRegistration) -> ConfigResolveResponse {
            self.data
                .get(&registration.identifier)
                .cloned()
                .map_or_else(
                    || {
                        ConfigResolveResponse::Error(CfgsyncErrorResponse::missing_config(
                            &registration.identifier,
                        ))
                    },
                    ConfigResolveResponse::Config,
                )
        }
    }

    struct RegistrationAwareProvider {
        data: HashMap<String, NodeArtifactsPayload>,
        registrations: std::sync::Mutex<HashMap<String, NodeRegistration>>,
    }

    impl NodeConfigSource for RegistrationAwareProvider {
        fn register(&self, registration: NodeRegistration) -> RegisterNodeResponse {
            if !self.data.contains_key(&registration.identifier) {
                return RegisterNodeResponse::Error(CfgsyncErrorResponse::missing_config(
                    &registration.identifier,
                ));
            }

            let mut registrations = self
                .registrations
                .lock()
                .expect("test registration store should not be poisoned");
            registrations.insert(registration.identifier.clone(), registration);

            RegisterNodeResponse::Registered
        }

        fn resolve(&self, registration: &NodeRegistration) -> ConfigResolveResponse {
            let registrations = self
                .registrations
                .lock()
                .expect("test registration store should not be poisoned");

            if !registrations.contains_key(&registration.identifier) {
                return ConfigResolveResponse::Error(CfgsyncErrorResponse::not_ready(
                    &registration.identifier,
                ));
            }

            self.data
                .get(&registration.identifier)
                .cloned()
                .map_or_else(
                    || {
                        ConfigResolveResponse::Error(CfgsyncErrorResponse::missing_config(
                            &registration.identifier,
                        ))
                    },
                    ConfigResolveResponse::Config,
                )
        }
    }

    fn sample_payload() -> NodeArtifactsPayload {
        NodeArtifactsPayload {
            schema_version: CFGSYNC_SCHEMA_VERSION,
            files: vec![NodeArtifactFile::new("/app-config.yaml", "app: test")],
        }
    }

    #[tokio::test]
    async fn node_config_resolves_from_non_tf_provider() {
        let mut data = HashMap::new();
        data.insert("node-a".to_owned(), sample_payload());

        let provider = Arc::new(RegistrationAwareProvider {
            data,
            registrations: std::sync::Mutex::new(HashMap::new()),
        });
        let state = Arc::new(CfgsyncServerState::new(provider));
        let payload = NodeRegistration::new("node-a", "127.0.0.1".parse().expect("valid ip"));

        let _ = register_node(State(state.clone()), Json(payload.clone()))
            .await
            .into_response();

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
        let state = Arc::new(CfgsyncServerState::new(provider));
        let payload = NodeRegistration::new("missing-node", "127.0.0.1".parse().expect("valid ip"));

        let response = node_config(State(state), Json(payload))
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn missing_config_error_uses_expected_code() {
        let error = CfgsyncErrorResponse::missing_config("missing-node");

        assert!(matches!(error.code, CfgsyncErrorCode::MissingConfig));
    }

    #[tokio::test]
    async fn node_config_returns_not_ready_before_registration() {
        let mut data = HashMap::new();
        data.insert("node-a".to_owned(), sample_payload());

        let provider = Arc::new(RegistrationAwareProvider {
            data,
            registrations: std::sync::Mutex::new(HashMap::new()),
        });
        let state = Arc::new(CfgsyncServerState::new(provider));
        let payload = NodeRegistration::new("node-a", "127.0.0.1".parse().expect("valid ip"));

        let response = node_config(State(state), Json(payload))
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::TOO_EARLY);
    }
}

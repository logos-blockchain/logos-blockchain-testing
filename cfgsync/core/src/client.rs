use serde::Serialize;
use thiserror::Error;

use crate::{
    CfgsyncErrorCode, CfgsyncErrorResponse, NodeArtifactFile, NodeArtifactsPayload,
    NodeRegistration, ReplaceNodeArtifactsRequest,
};

/// cfgsync client-side request/response failures.
#[derive(Debug, Error)]
pub enum ClientError {
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("cfgsync server error {status}: {message}")]
    Status {
        status: reqwest::StatusCode,
        message: String,
        error: Option<CfgsyncErrorResponse>,
    },
    #[error("failed to parse cfgsync response: {0}")]
    Decode(serde_json::Error),
}

/// Result of probing cfgsync for a node's current artifact availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFetchStatus {
    /// The node payload is ready and can be fetched successfully.
    Ready,
    /// The node has registered but artifacts are not ready yet.
    NotReady,
    /// The server does not know how to materialize artifacts for this node.
    Missing,
}

/// Reusable HTTP client for cfgsync server endpoints.
#[derive(Clone, Debug)]
pub struct Client {
    base_url: String,
    http: reqwest::Client,
}

impl Client {
    /// Creates a cfgsync client pointed at the given server base URL.
    #[must_use]
    pub fn new(base_url: String) -> Self {
        let mut base_url = base_url;
        while base_url.ends_with('/') {
            base_url.pop();
        }
        Self {
            base_url,
            http: reqwest::Client::new(),
        }
    }

    /// Returns the normalized cfgsync server base URL used for requests.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Registers a node before requesting config.
    pub async fn register_node(&self, payload: &NodeRegistration) -> Result<(), ClientError> {
        self.post_status_only("/register", payload).await
    }

    /// Fetches `/node` payload for a node identifier.
    pub async fn fetch_node_config(
        &self,
        payload: &NodeRegistration,
    ) -> Result<NodeArtifactsPayload, ClientError> {
        self.post_json("/node", payload).await
    }

    /// Probes whether artifacts for a node are ready, missing, or still
    /// pending.
    pub async fn fetch_node_config_status(
        &self,
        payload: &NodeRegistration,
    ) -> Result<ConfigFetchStatus, ClientError> {
        match self.fetch_node_config(payload).await {
            Ok(_) => Ok(ConfigFetchStatus::Ready),
            Err(ClientError::Status {
                status,
                error: Some(error),
                ..
            }) => match error.code {
                CfgsyncErrorCode::NotReady => Ok(ConfigFetchStatus::NotReady),
                CfgsyncErrorCode::MissingConfig => Ok(ConfigFetchStatus::Missing),
                CfgsyncErrorCode::Internal => Err(ClientError::Status {
                    status,
                    message: error.message.clone(),
                    error: Some(error),
                }),
            },
            Err(error) => Err(error),
        }
    }

    /// Replaces the served files for one node identifier.
    pub async fn replace_node_artifacts(
        &self,
        identifier: impl Into<String>,
        files: Vec<NodeArtifactFile>,
    ) -> Result<(), ClientError> {
        self.post_status_only(
            "/node/replace",
            &ReplaceNodeArtifactsRequest {
                identifier: identifier.into(),
                files,
            },
        )
        .await
    }

    /// Posts JSON payload to a cfgsync endpoint and decodes cfgsync payload.
    pub async fn post_json<P: Serialize>(
        &self,
        path: &str,
        payload: &P,
    ) -> Result<NodeArtifactsPayload, ClientError> {
        let url = self.endpoint_url(path);
        let response = self.http.post(url).json(payload).send().await?;

        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            let error = serde_json::from_str::<CfgsyncErrorResponse>(&body).ok();
            let message = error
                .as_ref()
                .map(|err| err.message.clone())
                .unwrap_or_else(|| body.clone());
            return Err(ClientError::Status {
                status,
                message,
                error,
            });
        }

        serde_json::from_str(&body).map_err(ClientError::Decode)
    }

    async fn post_status_only<P: Serialize>(
        &self,
        path: &str,
        payload: &P,
    ) -> Result<(), ClientError> {
        let url = self.endpoint_url(path);
        let response = self.http.post(url).json(payload).send().await?;

        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            let error = serde_json::from_str::<CfgsyncErrorResponse>(&body).ok();
            let message = error
                .as_ref()
                .map(|err| err.message.clone())
                .unwrap_or_else(|| body.clone());
            return Err(ClientError::Status {
                status,
                message,
                error,
            });
        }

        Ok(())
    }

    fn endpoint_url(&self, path: &str) -> String {
        if path.starts_with('/') {
            format!("{}{}", self.base_url, path)
        } else {
            format!("{}/{}", self.base_url, path)
        }
    }
}

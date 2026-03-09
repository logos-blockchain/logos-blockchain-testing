use serde::Serialize;
use thiserror::Error;

use crate::repo::{CfgSyncErrorResponse, CfgSyncPayload, NodeRegistration};

/// cfgsync client-side request/response failures.
#[derive(Debug, Error)]
pub enum ClientError {
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("cfgsync server error {status}: {message}")]
    Status {
        status: reqwest::StatusCode,
        message: String,
        error: Option<CfgSyncErrorResponse>,
    },
    #[error("failed to parse cfgsync response: {0}")]
    Decode(serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFetchStatus {
    Ready,
    NotReady,
}

/// Reusable HTTP client for cfgsync server endpoints.
#[derive(Clone, Debug)]
pub struct CfgSyncClient {
    base_url: String,
    http: reqwest::Client,
}

impl CfgSyncClient {
    #[must_use]
    pub fn new(base_url: impl Into<String>) -> Self {
        let mut base_url = base_url.into();
        while base_url.ends_with('/') {
            base_url.pop();
        }
        Self {
            base_url,
            http: reqwest::Client::new(),
        }
    }

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
    ) -> Result<CfgSyncPayload, ClientError> {
        self.post_json("/node", payload).await
    }

    /// Fetches `/init-with-node` payload for a node identifier.
    pub async fn fetch_init_with_node_config(
        &self,
        payload: &NodeRegistration,
    ) -> Result<CfgSyncPayload, ClientError> {
        self.post_json("/init-with-node", payload).await
    }

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
            }) if status == reqwest::StatusCode::TOO_EARLY => {
                let _ = error;
                Ok(ConfigFetchStatus::NotReady)
            }
            Err(error) => Err(error),
        }
    }

    /// Posts JSON payload to a cfgsync endpoint and decodes cfgsync payload.
    pub async fn post_json<P: Serialize>(
        &self,
        path: &str,
        payload: &P,
    ) -> Result<CfgSyncPayload, ClientError> {
        let url = self.endpoint_url(path);
        let response = self.http.post(url).json(payload).send().await?;

        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            let error = serde_json::from_str::<CfgSyncErrorResponse>(&body).ok();
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
            let error = serde_json::from_str::<CfgSyncErrorResponse>(&body).ok();
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

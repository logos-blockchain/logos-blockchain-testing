use std::{collections::HashMap, sync::Arc};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::oneshot::Sender;

pub const CFGSYNC_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgSyncPayload {
    pub schema_version: u16,
    pub config_yaml: String,
}

impl CfgSyncPayload {
    #[must_use]
    pub fn new(config_yaml: String) -> Self {
        Self {
            schema_version: CFGSYNC_SCHEMA_VERSION,
            config_yaml,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CfgSyncErrorCode {
    MissingConfig,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, Error)]
#[error("{code:?}: {message}")]
pub struct CfgSyncErrorResponse {
    pub code: CfgSyncErrorCode,
    pub message: String,
}

impl CfgSyncErrorResponse {
    #[must_use]
    pub fn missing_config(identifier: &str) -> Self {
        Self {
            code: CfgSyncErrorCode::MissingConfig,
            message: format!("missing config for host {identifier}"),
        }
    }

    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: CfgSyncErrorCode::Internal,
            message: message.into(),
        }
    }
}

pub enum RepoResponse {
    Config(CfgSyncPayload),
    Error(CfgSyncErrorResponse),
}

pub struct ConfigRepo {
    configs: HashMap<String, CfgSyncPayload>,
}

impl ConfigRepo {
    #[must_use]
    pub fn from_bundle(configs: HashMap<String, CfgSyncPayload>) -> Arc<Self> {
        Arc::new(Self { configs })
    }

    pub async fn register(&self, identifier: String, reply_tx: Sender<RepoResponse>) {
        let response = self.configs.get(&identifier).cloned().map_or_else(
            || RepoResponse::Error(CfgSyncErrorResponse::missing_config(&identifier)),
            RepoResponse::Config,
        );

        let _ = reply_tx.send(response);
    }
}

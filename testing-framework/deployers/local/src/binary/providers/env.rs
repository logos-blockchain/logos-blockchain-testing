//! Env-var provider.
//!
//! This provider is the explicit override path: the configured env var must
//! contain a concrete executable path. Missing or non-file values are treated
//! as unresolved so a fallback provider can continue.

use std::{env, path::PathBuf};

use async_trait::async_trait;
use tracing::{debug, info};

use crate::binary::{BinaryProvider, BinaryProviderError, EnvBinaryProvider};

#[async_trait]
impl BinaryProvider for EnvBinaryProvider {
    async fn try_resolve(&self) -> Result<Option<PathBuf>, BinaryProviderError> {
        let Some(path) = env::var_os(&self.env_var).map(PathBuf::from) else {
            return Ok(None);
        };

        if !path.is_file() {
            debug!(
                env = self.env_var,
                path = %path.display(),
                "binary env override does not point to a file"
            );

            return Ok(None);
        }

        info!(
            env = self.env_var,
            path = %path.display(),
            "resolved binary from env override"
        );

        Ok(Some(path))
    }

    fn display(&self) -> String {
        "env".to_owned()
    }

    fn cache_key(&self) -> String {
        format!("env:{}", self.env_var)
    }
}

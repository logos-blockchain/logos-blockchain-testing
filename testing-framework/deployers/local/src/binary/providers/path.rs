//! Explicit path provider.
//!
//! This provider uses an absolute executable path supplied by the test or app
//! integration. It does not search the filesystem or inspect the process
//! `PATH`, which keeps CI and mixed-version cluster setups deterministic.

use std::path::PathBuf;

use tracing::info;

use crate::binary::{BinaryProvider, BinaryProviderError, PathBinaryProvider};

impl BinaryProvider for PathBinaryProvider {
    fn try_resolve(&self) -> Result<Option<PathBuf>, BinaryProviderError> {
        if !self.path.is_absolute() {
            return Err(BinaryProviderError::RelativePath {
                path: self.path.clone(),
            });
        }

        if !self.path.is_file() {
            return Ok(None);
        }

        info!(
            path = %self.path.display(),
            "resolved binary from configured path"
        );

        Ok(Some(self.path.clone()))
    }

    fn display(&self) -> String {
        "path".to_owned()
    }

    fn cache_key(&self) -> String {
        format!("path:{}", self.path.display())
    }
}

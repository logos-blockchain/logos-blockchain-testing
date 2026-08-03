//! Fallback provider.
//!
//! A fallback is still a single configured provider from the process launch
//! perspective. It simply composes several concrete providers and returns the
//! first executable path they can produce.

use std::path::PathBuf;

use async_trait::async_trait;

use crate::binary::{BinaryProvider, BinaryProviderError, FallbackBinaryProvider};

#[async_trait]
impl BinaryProvider for FallbackBinaryProvider {
    async fn try_resolve(&self) -> Result<Option<PathBuf>, BinaryProviderError> {
        for provider in &self.providers {
            match provider.resolve().await {
                Ok(path) => return Ok(Some(path)),
                Err(BinaryProviderError::NotFound { .. }) => continue,
                Err(error) => return Err(error),
            }
        }

        Ok(None)
    }

    fn display(&self) -> String {
        self.providers
            .iter()
            .map(|provider| provider.display())
            .collect::<Vec<_>>()
            .join(",")
    }

    fn cache_key(&self) -> String {
        self.providers
            .iter()
            .map(|provider| provider.cache_key())
            .collect::<Vec<_>>()
            .join(",")
    }
}

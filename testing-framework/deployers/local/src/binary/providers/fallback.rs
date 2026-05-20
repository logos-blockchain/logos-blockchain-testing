//! Fallback provider.
//!
//! A fallback is still a single configured provider from the process launch
//! perspective. It simply composes several concrete providers and returns the
//! first executable path they can produce.

use std::path::PathBuf;

use crate::binary::{BinaryProvider, BinaryProviderError, FallbackBinaryProvider};

impl BinaryProvider for FallbackBinaryProvider {
    fn try_resolve(&self) -> Result<Option<PathBuf>, BinaryProviderError> {
        for provider in &self.providers {
            if let Some(path) = provider.try_resolve()? {
                return Ok(Some(path));
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

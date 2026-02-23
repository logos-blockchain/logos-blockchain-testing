use async_trait::async_trait;

use crate::scenario::{Application, DynError, ExternalNodeSource};

/// External node client prepared from a static external source endpoint.
#[derive(Clone, Debug)]
pub struct ExternalNode<E: Application> {
    /// Optional stable identity hint used by runtime inventory dedup logic.
    pub identity_hint: Option<String>,
    /// Application-specific client for the external node endpoint.
    pub client: E::NodeClient,
}

/// Errors returned while constructing node clients from external sources.
#[derive(Debug, thiserror::Error)]
pub enum ExternalProviderError {
    #[error("external source is not supported by this provider: {external_source:?}")]
    UnsupportedSource { external_source: ExternalNodeSource },
    #[error("failed to build external node from source {source_label}: {source}")]
    Build {
        source_label: String,
        #[source]
        source: DynError,
    },
}

/// Internal adapter interface for constructing node clients from static
/// external endpoint sources.
///
/// This is scaffolding-only in phase 1 and is intentionally not wired into
/// deployer runtime orchestration yet.
#[async_trait]
pub trait ExternalProvider<E: Application>: Send + Sync {
    /// Builds external node handles from external source descriptors.
    async fn provide(
        &self,
        sources: &[ExternalNodeSource],
    ) -> Result<Vec<ExternalNode<E>>, ExternalProviderError>;
}

/// Default external provider stub used while external wiring is not
/// implemented.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopExternalProvider;

#[async_trait]
impl<E: Application> ExternalProvider<E> for NoopExternalProvider {
    async fn provide(
        &self,
        sources: &[ExternalNodeSource],
    ) -> Result<Vec<ExternalNode<E>>, ExternalProviderError> {
        let Some(first) = sources.first() else {
            return Ok(Vec::new());
        };

        Err(ExternalProviderError::UnsupportedSource {
            external_source: first.clone(),
        })
    }
}

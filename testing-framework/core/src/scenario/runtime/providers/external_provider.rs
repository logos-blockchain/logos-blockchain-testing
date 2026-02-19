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
pub trait ExternalProvider<E: Application>: Send + Sync {
    /// Builds one external node handle from one external source descriptor.
    fn build_node(
        &self,
        source: &ExternalNodeSource,
    ) -> Result<ExternalNode<E>, ExternalProviderError>;
}

/// Default external provider stub used while external wiring is not
/// implemented.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopExternalProvider;

impl<E: Application> ExternalProvider<E> for NoopExternalProvider {
    fn build_node(
        &self,
        source: &ExternalNodeSource,
    ) -> Result<ExternalNode<E>, ExternalProviderError> {
        Err(ExternalProviderError::UnsupportedSource {
            external_source: source.clone(),
        })
    }
}

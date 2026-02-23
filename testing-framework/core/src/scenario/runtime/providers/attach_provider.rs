use async_trait::async_trait;

use crate::scenario::{Application, AttachSource, DynError};

/// Attached node discovered from an existing external cluster source.
#[derive(Clone, Debug)]
pub struct AttachedNode<E: Application> {
    /// Optional stable identity hint used by runtime inventory dedup logic.
    pub identity_hint: Option<String>,
    /// Application-specific client for the discovered node.
    pub client: E::NodeClient,
}

/// Errors returned by attach providers while discovering attached nodes.
#[derive(Debug, thiserror::Error)]
pub enum AttachProviderError {
    #[error("attach source is not supported by this provider: {attach_source:?}")]
    UnsupportedSource { attach_source: AttachSource },
    #[error("attach discovery failed: {source}")]
    Discovery {
        #[source]
        source: DynError,
    },
}

/// Internal adapter interface for discovering pre-existing nodes.
///
/// This is scaffolding-only in phase 1 and is intentionally not wired into
/// deployer runtime orchestration yet.
#[async_trait]
pub trait AttachProvider<E: Application>: Send + Sync {
    /// Discovers node clients for the requested attach source.
    async fn discover(
        &self,
        source: &AttachSource,
    ) -> Result<Vec<AttachedNode<E>>, AttachProviderError>;
}

/// Default attach provider stub used while attach wiring is not implemented.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopAttachProvider;

#[async_trait]
impl<E: Application> AttachProvider<E> for NoopAttachProvider {
    async fn discover(
        &self,
        source: &AttachSource,
    ) -> Result<Vec<AttachedNode<E>>, AttachProviderError> {
        Err(AttachProviderError::UnsupportedSource {
            attach_source: source.clone(),
        })
    }
}

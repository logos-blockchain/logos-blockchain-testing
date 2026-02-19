use async_trait::async_trait;

use crate::scenario::{Application, DynError, runtime::orchestration::ManagedSource};

/// Managed node produced by the managed provider path.
#[derive(Clone, Debug)]
pub struct ManagedProvisionedNode<E: Application> {
    /// Optional stable identity hint used by runtime inventory dedup logic.
    pub identity_hint: Option<String>,
    /// Application-specific client for the managed node.
    pub client: E::NodeClient,
}

/// Errors returned by managed providers while provisioning managed nodes.
#[derive(Debug, thiserror::Error)]
pub enum ManagedProviderError {
    #[error("managed source is not supported by this provider: {managed_source:?}")]
    UnsupportedSource { managed_source: ManagedSource },
    #[error("managed provisioning failed: {source}")]
    Provisioning {
        #[source]
        source: DynError,
    },
}

/// Internal adapter interface for managed node provisioning.
///
/// This is scaffolding-only in phase 1 and is intentionally not wired into
/// deployer runtime orchestration yet.
#[async_trait]
pub trait ManagedProvider<E: Application>: Send + Sync {
    /// Provisions or resolves managed nodes for the requested managed source.
    async fn provide(
        &self,
        source: &ManagedSource,
    ) -> Result<Vec<ManagedProvisionedNode<E>>, ManagedProviderError>;
}

/// Default managed provider stub used while unified provider wiring is not
/// implemented.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopManagedProvider;

#[async_trait]
impl<E: Application> ManagedProvider<E> for NoopManagedProvider {
    async fn provide(
        &self,
        source: &ManagedSource,
    ) -> Result<Vec<ManagedProvisionedNode<E>>, ManagedProviderError> {
        Err(ManagedProviderError::UnsupportedSource {
            managed_source: *source,
        })
    }
}

/// Managed provider that returns a precomputed set of managed node clients.
///
/// Useful as an adapter while deployers are still the source of managed
/// provisioning.
#[derive(Clone, Debug)]
pub struct StaticManagedProvider<E: Application> {
    clients: Vec<E::NodeClient>,
}

impl<E: Application> StaticManagedProvider<E> {
    #[must_use]
    pub fn new(clients: Vec<E::NodeClient>) -> Self {
        Self { clients }
    }
}

#[async_trait]
impl<E: Application> ManagedProvider<E> for StaticManagedProvider<E> {
    async fn provide(
        &self,
        source: &ManagedSource,
    ) -> Result<Vec<ManagedProvisionedNode<E>>, ManagedProviderError> {
        match source {
            ManagedSource::DeployerManaged => Ok(self
                .clients
                .iter()
                .cloned()
                .map(|client| ManagedProvisionedNode {
                    identity_hint: None,
                    client,
                })
                .collect()),
        }
    }
}

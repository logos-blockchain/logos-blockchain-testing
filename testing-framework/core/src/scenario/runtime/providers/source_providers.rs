use std::sync::Arc;

use super::{
    attach_provider::{AttachProvider, NoopAttachProvider},
    external_provider::{ExternalProvider, NoopExternalProvider},
    managed_provider::{ManagedProvider, NoopManagedProvider},
};
use crate::scenario::Application;

/// Unified provider set used by source orchestration.
///
/// This is scaffolding-only and is intentionally not wired into runtime
/// deployer orchestration yet.
pub struct SourceProviders<E: Application> {
    pub managed: Arc<dyn ManagedProvider<E>>,
    pub attach: Arc<dyn AttachProvider<E>>,
    pub external: Arc<dyn ExternalProvider<E>>,
}

impl<E: Application> Default for SourceProviders<E> {
    fn default() -> Self {
        Self {
            managed: Arc::new(NoopManagedProvider),
            attach: Arc::new(NoopAttachProvider),
            external: Arc::new(NoopExternalProvider),
        }
    }
}

impl<E: Application> SourceProviders<E> {
    #[must_use]
    pub fn with_managed(mut self, provider: Arc<dyn ManagedProvider<E>>) -> Self {
        self.managed = provider;
        self
    }

    #[must_use]
    pub fn with_attach(mut self, provider: Arc<dyn AttachProvider<E>>) -> Self {
        self.attach = provider;
        self
    }

    #[must_use]
    pub fn with_external(mut self, provider: Arc<dyn ExternalProvider<E>>) -> Self {
        self.external = provider;
        self
    }
}

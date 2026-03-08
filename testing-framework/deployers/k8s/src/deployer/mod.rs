mod attach_provider;
mod orchestrator;

pub use orchestrator::{K8sDeployer, K8sRunnerError};
use testing_framework_core::scenario::{AttachSource, DynError};

/// Kubernetes deployment metadata returned by k8s-specific deployment APIs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct K8sDeploymentMetadata {
    /// Namespace used for this deployment when available.
    pub namespace: Option<String>,
    /// Attach selector used to discover node services.
    pub label_selector: Option<String>,
}

#[derive(Debug, thiserror::Error)]
enum K8sMetadataError {
    #[error("k8s deployment metadata has no namespace")]
    MissingNamespace,
    #[error("k8s deployment metadata has no label selector")]
    MissingLabelSelector,
}

impl K8sDeploymentMetadata {
    /// Returns namespace when deployment is bound to a specific namespace.
    #[must_use]
    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    /// Returns attach label selector when available.
    #[must_use]
    pub fn label_selector(&self) -> Option<&str> {
        self.label_selector.as_deref()
    }

    /// Builds an existing-cluster descriptor for the same k8s deployment scope.
    pub fn existing_cluster(&self) -> Result<AttachSource, DynError> {
        let namespace = self.namespace().ok_or(K8sMetadataError::MissingNamespace)?;
        let label_selector = self
            .label_selector()
            .ok_or(K8sMetadataError::MissingLabelSelector)?;

        Ok(AttachSource::k8s_in_namespace(
            label_selector.to_owned(),
            namespace.to_owned(),
        ))
    }

    #[doc(hidden)]
    pub fn attach_source(&self) -> Result<AttachSource, DynError> {
        self.existing_cluster()
    }
}

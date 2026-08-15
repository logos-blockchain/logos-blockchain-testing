mod attach_provider;
mod node_control;
mod orchestrator;

pub use node_control::{K8sNodeControl, K8sNodeControlError};
pub use orchestrator::{K8sDeployer, K8sRunnerError};
use testing_framework_core::scenario::{DynError, ExistingCluster, IntoExistingCluster};

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
    #[must_use]
    pub fn from_existing_cluster(cluster: Option<&ExistingCluster>) -> Self {
        Self {
            namespace: cluster
                .and_then(ExistingCluster::k8s_namespace)
                .map(ToOwned::to_owned),
            label_selector: cluster
                .and_then(ExistingCluster::k8s_label_selector)
                .map(ToOwned::to_owned),
        }
    }

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
    pub fn existing_cluster(&self) -> Result<ExistingCluster, DynError> {
        let namespace = self.namespace().ok_or(K8sMetadataError::MissingNamespace)?;
        let label_selector = self
            .label_selector()
            .ok_or(K8sMetadataError::MissingLabelSelector)?;

        Ok(ExistingCluster::for_k8s_selector_in_namespace(
            namespace.to_owned(),
            label_selector.to_owned(),
        ))
    }

    #[doc(hidden)]
    pub fn attach_source(&self) -> Result<ExistingCluster, DynError> {
        self.existing_cluster()
    }
}

impl IntoExistingCluster for K8sDeploymentMetadata {
    fn into_existing_cluster(self) -> Result<ExistingCluster, DynError> {
        self.existing_cluster()
    }
}

impl IntoExistingCluster for &K8sDeploymentMetadata {
    fn into_existing_cluster(self) -> Result<ExistingCluster, DynError> {
        self.existing_cluster()
    }
}

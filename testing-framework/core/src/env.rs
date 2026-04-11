use std::io;

use async_trait::async_trait;

use crate::{
    scenario::{DynError, ExternalNodeSource, NodeAccess},
    topology::DeploymentDescriptor,
};

/// Bundles all backend-specific types used by the core scenario engine.
#[async_trait]
pub trait Application: Send + Sync + 'static {
    type Deployment: DeploymentDescriptor + Clone + 'static;

    type NodeClient: Clone + Send + Sync + 'static;

    type NodeConfig: Clone + Send + Sync + 'static;

    /// Build an application node client from a static external source.
    ///
    /// Environments that support external nodes should override this.
    fn external_node_client(_source: &ExternalNodeSource) -> Result<Self::NodeClient, DynError> {
        Err(io::Error::other("external node sources are not supported").into())
    }

    /// Build an application node client from deployer-provided node access.
    fn build_node_client(_access: &NodeAccess) -> Result<Self::NodeClient, DynError> {
        Err(io::Error::other("node access is not supported").into())
    }

    /// Path appended by deployers during default readiness probing.
    fn node_readiness_path() -> &'static str {
        "/"
    }
}

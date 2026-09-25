use std::io;

use async_trait::async_trait;

use crate::{
    scenario::{DynError, ExternalNodeSource, NodeAccess, ReadinessProbe},
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

    /// Selects the node readiness check, including the path for HTTP probes.
    fn node_readiness_probe() -> ReadinessProbe {
        ReadinessProbe::Http { path: "/" }
    }
}

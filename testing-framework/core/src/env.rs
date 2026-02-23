use std::io;

use async_trait::async_trait;

use crate::{
    scenario::{DynError, ExternalNodeSource, FeedRuntime},
    topology::DeploymentDescriptor,
};

/// Bundles all backend-specific types used by the core scenario engine.
#[async_trait]
pub trait Application: Send + Sync + 'static {
    type Deployment: DeploymentDescriptor + Clone + 'static;

    type NodeClient: Clone + Send + Sync + 'static;

    type NodeConfig: Clone + Send + Sync + 'static;

    type FeedRuntime: FeedRuntime;

    /// Build an application node client from a static external source.
    ///
    /// Environments that support external nodes should override this.
    fn external_node_client(_source: &ExternalNodeSource) -> Result<Self::NodeClient, DynError> {
        Err(io::Error::other("external node sources are not supported").into())
    }

    async fn prepare_feed(
        client: Self::NodeClient,
    ) -> Result<(<Self::FeedRuntime as FeedRuntime>::Feed, Self::FeedRuntime), DynError>;
}

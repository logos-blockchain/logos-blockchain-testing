use async_trait::async_trait;

use crate::{
    scenario::{DynError, FeedRuntime},
    topology::DeploymentDescriptor,
};

/// Bundles all backend-specific types used by the core scenario engine.
#[async_trait]
pub trait Application: Send + Sync + 'static {
    type Deployment: DeploymentDescriptor + Clone + 'static;

    type NodeClient: Clone + Send + Sync + 'static;

    type NodeConfig: Clone + Send + Sync + 'static;

    type FeedRuntime: FeedRuntime;

    /// Optional stable node identity (for example a peer id) used for
    /// deduplication when nodes are discovered from multiple sources.
    fn node_peer_identity(_client: &Self::NodeClient) -> Option<String> {
        None
    }

    /// Optional endpoint identity used as a dedup fallback when no peer id is
    /// available.
    fn node_endpoint_identity(_client: &Self::NodeClient) -> Option<String> {
        None
    }

    async fn prepare_feed(
        client: Self::NodeClient,
    ) -> Result<(<Self::FeedRuntime as FeedRuntime>::Feed, Self::FeedRuntime), DynError>;
}

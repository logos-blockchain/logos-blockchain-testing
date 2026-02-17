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

    async fn prepare_feed(
        client: Self::NodeClient,
    ) -> Result<(<Self::FeedRuntime as FeedRuntime>::Feed, Self::FeedRuntime), DynError>;
}

use async_trait::async_trait;

use super::{Application, DynError, Feed, FeedRuntime, NodeAccess, NodeClients};

#[derive(Clone)]
pub struct DefaultFeed;

impl Feed for DefaultFeed {
    type Subscription = ();

    fn subscribe(&self) -> Self::Subscription {}
}

pub struct DefaultFeedRuntime;

#[async_trait]
impl FeedRuntime for DefaultFeedRuntime {
    type Feed = DefaultFeed;

    async fn run(self: Box<Self>) {}
}

/// App surface for the common case where the framework default feed behavior is
/// sufficient and no custom feed runtime is needed.
#[async_trait]
pub trait ScenarioApplication: Send + Sync + 'static {
    type Deployment: crate::topology::DeploymentDescriptor + Clone + 'static;
    type NodeClient: Clone + Send + Sync + 'static;
    type NodeConfig: Clone + Send + Sync + 'static;

    fn external_node_client(
        _source: &super::ExternalNodeSource,
    ) -> Result<Self::NodeClient, DynError> {
        Err(std::io::Error::other("external node sources are not supported").into())
    }

    fn build_node_client(_access: &NodeAccess) -> Result<Self::NodeClient, DynError> {
        Err(std::io::Error::other("node access is not supported").into())
    }

    fn node_readiness_path() -> &'static str {
        "/"
    }
}

#[async_trait]
impl<T> Application for T
where
    T: ScenarioApplication,
{
    type Deployment = T::Deployment;
    type NodeClient = T::NodeClient;
    type NodeConfig = T::NodeConfig;
    type FeedRuntime = DefaultFeedRuntime;

    fn external_node_client(
        source: &super::ExternalNodeSource,
    ) -> Result<Self::NodeClient, DynError> {
        T::external_node_client(source)
    }

    fn build_node_client(access: &NodeAccess) -> Result<Self::NodeClient, DynError> {
        T::build_node_client(access)
    }

    fn node_readiness_path() -> &'static str {
        T::node_readiness_path()
    }

    async fn prepare_feed(
        _node_clients: NodeClients<Self>,
    ) -> Result<(<Self::FeedRuntime as FeedRuntime>::Feed, Self::FeedRuntime), DynError>
    where
        Self: Sized,
    {
        Ok((DefaultFeed, DefaultFeedRuntime))
    }
}

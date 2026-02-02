use async_trait::async_trait;

use crate::scenario::{Application, DynError, NodeControlHandle, StartNodeOptions, StartedNode};

/// Interface for imperative, deployer-backed manual clusters.
#[async_trait]
pub trait ManualClusterHandle<E: Application>: NodeControlHandle<E> {
    async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, DynError>;

    async fn wait_network_ready(&self) -> Result<(), DynError>;
}

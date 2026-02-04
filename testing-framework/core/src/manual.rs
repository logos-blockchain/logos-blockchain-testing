use async_trait::async_trait;

use crate::scenario::{DynError, NodeControlHandle, StartNodeOptions, StartedNode};

/// Interface for imperative, deployer-backed manual clusters.
#[async_trait]
pub trait ManualClusterHandle: NodeControlHandle {
    async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions,
    ) -> Result<StartedNode, DynError>;

    async fn wait_network_ready(&self) -> Result<(), DynError>;
}

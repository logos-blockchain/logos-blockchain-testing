use async_trait::async_trait;

use crate::scenario::{DynError, StartNodeOptions, StartedNode};

/// Interface for imperative, deployer-backed manual clusters.
#[async_trait]
pub trait ManualClusterHandle: Send + Sync {
    async fn start_validator_with(
        &self,
        name: &str,
        options: StartNodeOptions,
    ) -> Result<StartedNode, DynError>;

    async fn wait_network_ready(&self) -> Result<(), DynError>;
}

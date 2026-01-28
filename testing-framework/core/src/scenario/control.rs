use async_trait::async_trait;

use crate::{
    nodes::ApiClient,
    scenario::{DynError, StartNodeOptions, StartedNode},
};

/// Deployer-agnostic control surface for runtime node operations.
#[async_trait]
pub trait NodeControlHandle: Send + Sync {
    async fn restart_node(&self, _index: usize) -> Result<(), DynError> {
        Err("restart_node not supported by this deployer".into())
    }

    async fn start_node(&self, _name: &str) -> Result<StartedNode, DynError> {
        Err("start_node not supported by this deployer".into())
    }

    async fn start_node_with(
        &self,
        _name: &str,
        _options: StartNodeOptions,
    ) -> Result<StartedNode, DynError> {
        Err("start_node_with not supported by this deployer".into())
    }

    async fn stop_node(&self, _index: usize) -> Result<(), DynError> {
        Err("stop_node not supported by this deployer".into())
    }

    fn node_client(&self, _name: &str) -> Option<ApiClient> {
        None
    }

    fn node_pid(&self, _index: usize) -> Option<u32> {
        None
    }
}

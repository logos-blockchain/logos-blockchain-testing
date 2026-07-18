use async_trait::async_trait;

use crate::scenario::{Application, DynError, StartNodeOptions, StartedNode};

/// Deployer-agnostic control surface for runtime node operations.
#[async_trait]
pub trait NodeControlHandle<E: Application>: Send + Sync {
    async fn restart_node(&self, _name: &str) -> Result<(), DynError> {
        Err("restart_node not supported by this deployer".into())
    }

    async fn restart_node_with(
        &self,
        _name: &str,
        _options: StartNodeOptions<E>,
    ) -> Result<(), DynError> {
        Err("restart_node_with not supported by this deployer".into())
    }

    async fn start_node(&self, _name: &str) -> Result<StartedNode<E>, DynError> {
        Err("start_node not supported by this deployer".into())
    }

    async fn start_node_with(
        &self,
        _name: &str,
        _options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, DynError> {
        Err("start_node_with not supported by this deployer".into())
    }

    async fn stop_node(&self, _name: &str) -> Result<(), DynError> {
        Err("stop_node not supported by this deployer".into())
    }

    async fn wait_node_ready(&self, _name: &str) -> Result<(), DynError> {
        Err("wait_node_ready not supported by this deployer".into())
    }

    fn node_client(&self, _name: &str) -> Option<E::NodeClient> {
        None
    }

    fn node_pid(&self, _name: &str) -> Option<u32> {
        None
    }
}

/// Deployer-agnostic wait surface for cluster readiness checks.
#[async_trait]
pub trait ClusterWaitHandle<E: Application>: Send + Sync {
    async fn wait_network_ready(&self) -> Result<(), DynError> {
        Err("wait_network_ready not supported by this deployer".into())
    }
}

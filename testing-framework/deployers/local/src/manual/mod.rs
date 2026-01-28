use testing_framework_core::{
    manual::ManualClusterHandle,
    nodes::ApiClient,
    scenario::{DynError, NodeControlHandle, StartNodeOptions, StartedNode},
    topology::{
        config::{TopologyBuildError, TopologyBuilder, TopologyConfig},
        readiness::{ReadinessCheck, ReadinessError},
    },
};
use thiserror::Error;

use crate::node_control::{LocalNodeManager, LocalNodeManagerError, ReadinessNode};

mod readiness;

use readiness::ManualNetworkReadiness;

#[derive(Debug, Error)]
pub enum ManualClusterError {
    #[error("failed to build topology: {source}")]
    Build {
        #[source]
        source: TopologyBuildError,
    },
    #[error(transparent)]
    Dynamic(#[from] LocalNodeManagerError),
}

/// Imperative, in-process cluster that can start nodes on demand.
pub struct LocalManualCluster {
    nodes: LocalNodeManager,
}

impl LocalManualCluster {
    pub(crate) fn from_config(config: TopologyConfig) -> Result<Self, ManualClusterError> {
        let builder = TopologyBuilder::new(config);
        let descriptors = builder
            .build()
            .map_err(|source| ManualClusterError::Build { source })?;
        let nodes = LocalNodeManager::new(
            descriptors,
            testing_framework_core::scenario::NodeClients::default(),
        );
        Ok(Self { nodes })
    }

    #[must_use]
    pub fn node_client(&self, name: &str) -> Option<ApiClient> {
        self.nodes.node_client(name)
    }

    #[must_use]
    pub fn node_pid(&self, index: usize) -> Option<u32> {
        self.nodes.node_pid(index)
    }

    pub async fn start_node(&self, name: &str) -> Result<StartedNode, ManualClusterError> {
        Ok(self
            .nodes
            .start_node_with(name, StartNodeOptions::default())
            .await?)
    }

    pub async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions,
    ) -> Result<StartedNode, ManualClusterError> {
        Ok(self.nodes.start_node_with(name, options).await?)
    }

    pub fn stop_all(&self) {
        self.nodes.stop_all();
    }

    pub async fn restart_node(&self, index: usize) -> Result<(), ManualClusterError> {
        Ok(self.nodes.restart_node(index).await?)
    }

    pub async fn stop_node(&self, index: usize) -> Result<(), ManualClusterError> {
        Ok(self.nodes.stop_node(index).await?)
    }

    pub async fn wait_network_ready(&self) -> Result<(), ReadinessError> {
        let nodes = self.nodes.readiness_nodes();
        if self.is_singleton(&nodes) {
            return Ok(());
        }

        self.wait_nodes_ready(nodes).await
    }

    fn is_singleton(&self, nodes: &[ReadinessNode]) -> bool {
        nodes.len() <= 1
    }

    async fn wait_nodes_ready(&self, nodes: Vec<ReadinessNode>) -> Result<(), ReadinessError> {
        ManualNetworkReadiness::new(nodes).wait().await
    }
}

impl Drop for LocalManualCluster {
    fn drop(&mut self) {
        self.stop_all();
    }
}

#[async_trait::async_trait]
impl NodeControlHandle for LocalManualCluster {
    async fn restart_node(&self, index: usize) -> Result<(), DynError> {
        self.nodes
            .restart_node(index)
            .await
            .map_err(|err| err.into())
    }

    async fn stop_node(&self, index: usize) -> Result<(), DynError> {
        self.nodes.stop_node(index).await.map_err(|err| err.into())
    }

    async fn start_node(&self, name: &str) -> Result<StartedNode, DynError> {
        self.start_node_with(name, StartNodeOptions::default())
            .await
            .map_err(|err| err.into())
    }

    async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions,
    ) -> Result<StartedNode, DynError> {
        self.start_node_with(name, options)
            .await
            .map_err(|err| err.into())
    }

    fn node_client(&self, name: &str) -> Option<ApiClient> {
        self.node_client(name)
    }

    fn node_pid(&self, index: usize) -> Option<u32> {
        self.node_pid(index)
    }
}

#[async_trait::async_trait]
impl ManualClusterHandle for LocalManualCluster {
    async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions,
    ) -> Result<StartedNode, DynError> {
        self.start_node_with(name, options)
            .await
            .map_err(|err| err.into())
    }

    async fn wait_network_ready(&self) -> Result<(), DynError> {
        self.wait_network_ready().await.map_err(|err| err.into())
    }
}

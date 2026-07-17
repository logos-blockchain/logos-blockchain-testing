use testing_framework_core::{
    manual::ManualClusterHandle,
    scenario::{
        ClusterWaitHandle, DynError, ExternalNodeSource, NodeClients, NodeControlHandle,
        ReadinessError, StartNodeOptions, StartedNode,
    },
};
use thiserror::Error;

use crate::{LocalCluster, LocalClusterProvisioner, LocalDeployerEnv};

#[derive(Debug, Error)]
pub enum ManualClusterError {
    #[error(transparent)]
    Dynamic(#[from] DynError),
}

/// Imperative view over the same local cluster runtime used by scenarios and
/// apps.
pub struct ManualCluster<E: LocalDeployerEnv> {
    cluster: LocalCluster<E>,
}

impl<E: LocalDeployerEnv> ManualCluster<E> {
    pub fn from_topology(deployment: E::Deployment) -> Self {
        Self {
            cluster: LocalClusterProvisioner.manual_cluster(deployment),
        }
    }

    #[must_use]
    pub fn node_client(&self, name: &str) -> Option<E::NodeClient> {
        self.cluster.node_client(name)
    }

    #[must_use]
    pub fn node_pid(&self, name: &str) -> Option<u32> {
        self.cluster.node_pid(name)
    }

    pub async fn start_node(&self, name: &str) -> Result<StartedNode<E>, ManualClusterError> {
        Ok(self.cluster.start_node(name).await?)
    }

    pub async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, ManualClusterError> {
        Ok(self.cluster.start_node_with(name, options).await?)
    }

    pub fn stop_all(&self) {
        let _ = self.cluster.stop_all();
    }

    pub async fn restart_node(&self, name: &str) -> Result<(), ManualClusterError> {
        Ok(self.cluster.restart_node(name).await?)
    }

    pub async fn restart_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<(), ManualClusterError> {
        Ok(self.cluster.restart_node_with(name, options).await?)
    }

    pub async fn stop_node(&self, name: &str) -> Result<(), ManualClusterError> {
        Ok(self.cluster.stop_node(name).await?)
    }

    pub async fn wait_network_ready(&self) -> Result<(), ReadinessError> {
        self.cluster.wait_network_ready_typed().await
    }

    pub async fn wait_node_ready(&self, name: &str) -> Result<(), ManualClusterError> {
        Ok(self.cluster.wait_node_ready(name).await?)
    }

    #[must_use]
    pub fn node_clients(&self) -> NodeClients<E> {
        self.cluster.node_clients()
    }

    pub fn add_external_sources(
        &self,
        sources: impl IntoIterator<Item = ExternalNodeSource>,
    ) -> Result<(), DynError> {
        self.cluster.add_external_sources(sources)
    }

    pub fn add_external_clients(&self, clients: impl IntoIterator<Item = E::NodeClient>) {
        let node_clients = self.cluster.node_clients();
        for client in clients {
            node_clients.add_node(client);
        }
    }
}

#[async_trait::async_trait]
impl<E: LocalDeployerEnv> NodeControlHandle<E> for ManualCluster<E> {
    async fn restart_node(&self, name: &str) -> Result<(), DynError> {
        self.cluster.restart_node(name).await
    }

    async fn restart_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<(), DynError> {
        self.cluster.restart_node_with(name, options).await
    }

    async fn stop_node(&self, name: &str) -> Result<(), DynError> {
        self.cluster.stop_node(name).await
    }

    async fn start_node(&self, name: &str) -> Result<StartedNode<E>, DynError> {
        self.cluster.start_node(name).await
    }

    async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, DynError> {
        self.cluster.start_node_with(name, options).await
    }

    fn node_client(&self, name: &str) -> Option<E::NodeClient> {
        self.cluster.node_client(name)
    }

    fn node_pid(&self, name: &str) -> Option<u32> {
        self.cluster.node_pid(name)
    }
}

#[async_trait::async_trait]
impl<E: LocalDeployerEnv> ClusterWaitHandle<E> for ManualCluster<E> {
    async fn wait_network_ready(&self) -> Result<(), DynError> {
        self.cluster.wait_network_ready().await
    }
}

#[async_trait::async_trait]
impl<E: LocalDeployerEnv> ManualClusterHandle<E> for ManualCluster<E> {}

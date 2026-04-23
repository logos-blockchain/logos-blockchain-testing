use testing_framework_core::{
    manual::ManualClusterHandle,
    scenario::{
        ClusterWaitHandle, DynError, ExternalNodeSource, NodeClients, NodeControlHandle,
        ReadinessError, StartNodeOptions, StartedNode,
    },
};
use thiserror::Error;

use crate::{
    env::LocalDeployerEnv,
    external::build_external_client,
    keep_tempdir_from_env,
    node_control::{NodeManager, NodeManagerError, NodeManagerSeed},
};

#[derive(Debug, Error)]
pub enum ManualClusterError {
    #[error(transparent)]
    Dynamic(#[from] NodeManagerError),
}

/// Imperative, in-process cluster that can start nodes on demand.
pub struct ManualCluster<E: LocalDeployerEnv> {
    nodes: NodeManager<E>,
}

impl<E: LocalDeployerEnv> ManualCluster<E> {
    pub fn from_topology(descriptors: E::Deployment) -> Self {
        let nodes = NodeManager::new_with_seed(
            descriptors,
            testing_framework_core::scenario::NodeClients::default(),
            keep_tempdir_from_env(),
            NodeManagerSeed::default(),
        );

        Self { nodes }
    }

    #[must_use]
    pub fn node_client(&self, name: &str) -> Option<E::NodeClient> {
        self.nodes.node_client(name)
    }

    #[must_use]
    pub fn node_pid(&self, name: &str) -> Option<u32> {
        self.nodes.node_pid(name)
    }

    pub async fn start_node(&self, name: &str) -> Result<StartedNode<E>, ManualClusterError> {
        Ok(self
            .nodes
            .start_node_with(name, StartNodeOptions::<E>::default())
            .await?)
    }

    pub async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, ManualClusterError> {
        Ok(self.nodes.start_node_with(name, options).await?)
    }

    pub fn stop_all(&self) {
        self.nodes.stop_all();
    }

    pub async fn restart_node(&self, name: &str) -> Result<(), ManualClusterError> {
        Ok(self.nodes.restart_node(name).await?)
    }

    pub async fn restart_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<(), ManualClusterError> {
        Ok(self.nodes.restart_node_with(name, options).await?)
    }

    pub async fn stop_node(&self, name: &str) -> Result<(), ManualClusterError> {
        Ok(self.nodes.stop_node(name).await?)
    }

    pub async fn wait_network_ready(&self) -> Result<(), ReadinessError> {
        self.nodes.wait_network_ready().await
    }

    pub async fn wait_node_ready(&self, name: &str) -> Result<(), ManualClusterError> {
        self.nodes.wait_node_ready(name).await?;
        Ok(())
    }

    #[must_use]
    pub fn node_clients(&self) -> NodeClients<E> {
        self.nodes.node_clients()
    }

    pub fn add_external_sources(
        &self,
        external_sources: impl IntoIterator<Item = ExternalNodeSource>,
    ) -> Result<(), DynError> {
        let node_clients = self.nodes.node_clients();
        for source in external_sources {
            let client = E::external_node_client(&source)
                .or_else(|_| build_external_client::<E>(&source))?;
            node_clients.add_node(client);
        }

        Ok(())
    }

    pub fn add_external_clients(&self, clients: impl IntoIterator<Item = E::NodeClient>) {
        let node_clients = self.nodes.node_clients();
        for client in clients {
            node_clients.add_node(client);
        }
    }
}

impl<E: LocalDeployerEnv> Drop for ManualCluster<E> {
    fn drop(&mut self) {
        self.stop_all();
    }
}

#[async_trait::async_trait]
impl<E: LocalDeployerEnv> NodeControlHandle<E> for ManualCluster<E> {
    async fn restart_node(&self, name: &str) -> Result<(), DynError> {
        self.nodes
            .restart_node(name)
            .await
            .map_err(|err| err.into())
    }

    async fn restart_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<(), DynError> {
        self.nodes
            .restart_node_with(name, options)
            .await
            .map_err(|err| err.into())
    }

    async fn stop_node(&self, name: &str) -> Result<(), DynError> {
        self.nodes.stop_node(name).await.map_err(|err| err.into())
    }

    async fn start_node(&self, name: &str) -> Result<StartedNode<E>, DynError> {
        self.nodes
            .start_node_with(name, StartNodeOptions::<E>::default())
            .await
            .map_err(|err| err.into())
    }

    async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, DynError> {
        self.nodes
            .start_node_with(name, options)
            .await
            .map_err(|err| err.into())
    }

    fn node_client(&self, name: &str) -> Option<E::NodeClient> {
        self.node_client(name)
    }

    fn node_pid(&self, name: &str) -> Option<u32> {
        self.node_pid(name)
    }
}

#[async_trait::async_trait]
impl<E: LocalDeployerEnv> ClusterWaitHandle<E> for ManualCluster<E> {
    async fn wait_network_ready(&self) -> Result<(), DynError> {
        self.wait_network_ready().await.map_err(|err| err.into())
    }
}

#[async_trait::async_trait]
impl<E: LocalDeployerEnv> ManualClusterHandle<E> for ManualCluster<E> {}

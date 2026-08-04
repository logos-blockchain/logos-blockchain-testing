use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use testing_framework_core::scenario::{
    ClusterWaitHandle, DynError, ExternalNodeSource, NodeClients, NodeControlHandle,
    ReadinessError, StartNodeOptions, StartedNode, internal::CleanupGuard,
};

use crate::{
    LocalDeployerEnv, NodeManager, NodeManagerSeed, env::Node, external::build_external_client,
};

/// Shared local cluster runtime used by scenarios, app deployments, and manual
/// tests.
pub struct LocalCluster<E: LocalDeployerEnv> {
    deployment: E::Deployment,
    owner: Arc<LocalClusterOwner<E>>,
}

struct LocalClusterOwner<E: LocalDeployerEnv> {
    nodes: NodeManager<E>,
    closed: AtomicBool,
}

impl<E: LocalDeployerEnv> LocalClusterOwner<E> {
    fn close(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            self.nodes.stop_all();
            E::cleanup_local_cluster(self.nodes.deployment());
        }
    }

    fn ensure_open(&self) -> Result<(), DynError> {
        if self.closed.load(Ordering::Acquire) {
            Err("local cluster is no longer owned by an active run".into())
        } else {
            Ok(())
        }
    }
}

impl<E: LocalDeployerEnv> Drop for LocalClusterOwner<E> {
    fn drop(&mut self) {
        self.close();
    }
}

struct LocalClusterCleanup<E: LocalDeployerEnv> {
    owner: Arc<LocalClusterOwner<E>>,
}

impl<E: LocalDeployerEnv> CleanupGuard for LocalClusterCleanup<E> {
    fn cleanup(self: Box<Self>) {
        self.owner.close();
    }
}

impl<E: LocalDeployerEnv> Clone for LocalCluster<E> {
    fn clone(&self) -> Self {
        Self {
            deployment: self.deployment.clone(),
            owner: Arc::clone(&self.owner),
        }
    }
}

impl<E: LocalDeployerEnv> LocalCluster<E> {
    pub(crate) fn empty(deployment: E::Deployment, keep_tempdir: bool) -> Self {
        E::prepare_local_cluster(&deployment);
        let nodes = NodeManager::new_with_seed(
            deployment.clone(),
            NodeClients::default(),
            keep_tempdir,
            NodeManagerSeed::default(),
        );
        Self {
            deployment,
            owner: Arc::new(LocalClusterOwner {
                nodes,
                closed: AtomicBool::new(false),
            }),
        }
    }

    pub(crate) fn initialize_with_nodes(&self, nodes: Vec<Node<E>>) {
        self.owner.nodes.initialize_with_nodes(nodes);
    }

    pub(crate) fn add_external_sources(
        &self,
        sources: impl IntoIterator<Item = ExternalNodeSource>,
    ) -> Result<(), DynError> {
        for source in sources {
            let client = E::external_node_client(&source)
                .or_else(|_| build_external_client::<E>(&source))?;
            self.owner.nodes.node_clients().add_node(client);
        }
        Ok(())
    }

    #[must_use]
    pub fn deployment(&self) -> &E::Deployment {
        &self.deployment
    }

    #[must_use]
    pub fn node_count(&self) -> usize {
        testing_framework_core::topology::DeploymentDescriptor::node_count(&self.deployment)
    }

    #[must_use]
    pub fn node_clients(&self) -> NodeClients<E> {
        self.owner.nodes.node_clients()
    }

    #[must_use]
    pub fn node_client(&self, name: &str) -> Option<E::NodeClient> {
        self.owner.nodes.node_client(name)
    }

    #[must_use]
    pub fn node_pid(&self, name: &str) -> Option<u32> {
        self.owner.nodes.node_pid(name)
    }

    #[must_use]
    pub fn clients(&self) -> Vec<E::NodeClient> {
        self.node_clients().snapshot()
    }

    #[must_use]
    pub fn first_client(&self) -> Option<E::NodeClient> {
        self.node_clients()
            .with_clients(|clients| clients.first().cloned())
    }

    pub async fn start_node(&self, name: &str) -> Result<StartedNode<E>, DynError> {
        self.start_node_with(name, StartNodeOptions::default())
            .await
    }

    pub async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, DynError> {
        self.owner.ensure_open()?;
        Ok(self.owner.nodes.start_node_with(name, options).await?)
    }

    pub async fn stop_node(&self, name: &str) -> Result<(), DynError> {
        self.owner.ensure_open()?;
        Ok(self.owner.nodes.stop_node(name).await?)
    }

    pub async fn restart_node(&self, name: &str) -> Result<(), DynError> {
        self.owner.ensure_open()?;
        Ok(self.owner.nodes.restart_node(name).await?)
    }

    pub async fn restart_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<(), DynError> {
        self.owner.ensure_open()?;
        Ok(self.owner.nodes.restart_node_with(name, options).await?)
    }

    pub async fn wait_network_ready(&self) -> Result<(), DynError> {
        self.owner.ensure_open()?;
        Ok(self.wait_network_ready_typed().await?)
    }

    pub(crate) async fn wait_network_ready_typed(&self) -> Result<(), ReadinessError> {
        self.owner.nodes.wait_network_ready().await
    }

    pub async fn wait_node_ready(&self, name: &str) -> Result<(), DynError> {
        self.owner.ensure_open()?;
        Ok(self.owner.nodes.wait_node_ready(name).await?)
    }

    pub fn stop_all(&self) -> Result<(), DynError> {
        self.owner.ensure_open()?;
        self.owner.nodes.stop_all();
        Ok(())
    }

    pub async fn start_all(&self) -> Result<(), DynError> {
        self.owner.ensure_open()?;
        for index in 0..self.node_count() {
            self.start_node(&format!("node-{index}")).await?;
        }
        self.wait_network_ready().await
    }

    pub async fn restart_all(&self) -> Result<(), DynError> {
        self.stop_all()?;
        self.start_all().await
    }

    #[must_use]
    #[doc(hidden)]
    pub fn cleanup_guard(&self) -> Box<dyn CleanupGuard> {
        Box::new(LocalClusterCleanup {
            owner: Arc::clone(&self.owner),
        })
    }
}

#[async_trait]
impl<E: LocalDeployerEnv> NodeControlHandle<E> for LocalCluster<E> {
    async fn restart_node(&self, name: &str) -> Result<(), DynError> {
        self.restart_node(name).await
    }

    async fn restart_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<(), DynError> {
        self.restart_node_with(name, options).await
    }

    async fn start_node(&self, name: &str) -> Result<StartedNode<E>, DynError> {
        self.start_node(name).await
    }

    async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, DynError> {
        self.start_node_with(name, options).await
    }

    async fn stop_node(&self, name: &str) -> Result<(), DynError> {
        self.stop_node(name).await
    }

    async fn wait_node_ready(&self, name: &str) -> Result<(), DynError> {
        self.wait_node_ready(name).await
    }

    fn node_client(&self, name: &str) -> Option<E::NodeClient> {
        self.node_client(name)
    }

    fn node_pid(&self, name: &str) -> Option<u32> {
        self.node_pid(name)
    }
}

#[async_trait]
impl<E: LocalDeployerEnv> ClusterWaitHandle<E> for LocalCluster<E> {
    async fn wait_network_ready(&self) -> Result<(), DynError> {
        self.wait_network_ready().await
    }
}

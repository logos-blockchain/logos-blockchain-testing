use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use testing_framework_core::{
    scenario::{DynError, NodeClients, StartNodeOptions, StartedNode, internal::CleanupGuard},
    topology::DeploymentDescriptor,
};
use testing_framework_runner_local::{LocalDeployerEnv, ManualCluster, ProcessDeployer};

/// Control handle for an additional uniform cluster of local processes.
///
/// Unlike the outer scenario cluster, this cluster is deployed as one child of
/// a heterogeneous application stack. The scenario owns its lifetime and stops
/// every node during cleanup even if a handle clone remains elsewhere.
pub struct LocalAppCluster<E>
where
    E: LocalDeployerEnv,
{
    deployment: E::Deployment,
    owner: Arc<LocalClusterOwner<E>>,
}

struct LocalClusterOwner<E>
where
    E: LocalDeployerEnv,
{
    cluster: ManualCluster<E>,
    closed: AtomicBool,
}

impl<E> Drop for LocalClusterOwner<E>
where
    E: LocalDeployerEnv,
{
    fn drop(&mut self) {
        self.close();
    }
}

impl<E> LocalClusterOwner<E>
where
    E: LocalDeployerEnv,
{
    fn close(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            self.cluster.stop_all();
        }
    }

    fn ensure_open(&self) -> Result<(), DynError> {
        if self.closed.load(Ordering::Acquire) {
            Err("local app cluster is no longer owned by an active run".into())
        } else {
            Ok(())
        }
    }
}

struct LocalClusterCleanup<E>
where
    E: LocalDeployerEnv,
{
    owner: Arc<LocalClusterOwner<E>>,
}

impl<E> CleanupGuard for LocalClusterCleanup<E>
where
    E: LocalDeployerEnv,
{
    fn cleanup(self: Box<Self>) {
        self.owner.close();
    }
}

impl<E> Clone for LocalAppCluster<E>
where
    E: LocalDeployerEnv,
{
    fn clone(&self) -> Self {
        Self {
            deployment: self.deployment.clone(),
            owner: Arc::clone(&self.owner),
        }
    }
}

impl<E> LocalAppCluster<E>
where
    E: LocalDeployerEnv,
{
    /// Starts every node described by `deployment` and waits for readiness.
    pub(crate) async fn start(deployment: E::Deployment) -> Result<Self, DynError> {
        let cluster =
            ProcessDeployer::<E>::default().manual_cluster_from_descriptors(deployment.clone());
        let owner = Arc::new(LocalClusterOwner {
            cluster,
            closed: AtomicBool::new(false),
        });

        start_all_nodes(&owner.cluster, deployment.node_count()).await?;
        owner.cluster.wait_network_ready().await?;

        Ok(Self { deployment, owner })
    }

    /// Returns this cluster's deployment descriptor.
    #[must_use]
    pub fn deployment(&self) -> &E::Deployment {
        &self.deployment
    }

    /// Returns the number of nodes described by the deployment.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.deployment.node_count()
    }

    /// Returns a shared snapshot-capable collection of node clients.
    #[must_use]
    pub fn node_clients(&self) -> NodeClients<E> {
        self.owner.cluster.node_clients()
    }

    /// Returns the client for `name`, if that node has started.
    #[must_use]
    pub fn node_client(&self, name: &str) -> Option<E::NodeClient> {
        self.owner.cluster.node_client(name)
    }

    /// Returns the operating-system process ID for `name`, if it is running.
    #[must_use]
    pub fn node_pid(&self, name: &str) -> Option<u32> {
        self.owner.cluster.node_pid(name)
    }

    /// Returns a snapshot of all currently available node clients.
    #[must_use]
    pub fn clients(&self) -> Vec<E::NodeClient> {
        self.node_clients().snapshot()
    }

    /// Returns the first available node client.
    #[must_use]
    pub fn first_client(&self) -> Option<E::NodeClient> {
        self.node_clients()
            .with_clients(|clients| clients.first().cloned())
    }

    /// Starts the named node with default start options.
    pub async fn start_node(&self, name: &str) -> Result<StartedNode<E>, DynError> {
        self.owner.ensure_open()?;
        Ok(self.owner.cluster.start_node(name).await?)
    }

    /// Starts the named node with explicit start options.
    pub async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, DynError> {
        self.owner.ensure_open()?;
        Ok(self.owner.cluster.start_node_with(name, options).await?)
    }

    /// Stops the named node.
    pub async fn stop_node(&self, name: &str) -> Result<(), DynError> {
        self.owner.ensure_open()?;
        Ok(self.owner.cluster.stop_node(name).await?)
    }

    /// Restarts the named node with its existing configuration.
    pub async fn restart_node(&self, name: &str) -> Result<(), DynError> {
        self.owner.ensure_open()?;
        Ok(self.owner.cluster.restart_node(name).await?)
    }

    /// Restarts the named node with explicit start options.
    pub async fn restart_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<(), DynError> {
        self.owner.ensure_open()?;
        Ok(self.owner.cluster.restart_node_with(name, options).await?)
    }

    /// Waits until the cluster-level network readiness condition succeeds.
    pub async fn wait_network_ready(&self) -> Result<(), DynError> {
        self.owner.ensure_open()?;
        Ok(self.owner.cluster.wait_network_ready().await?)
    }

    /// Waits until the named node reports ready.
    pub async fn wait_node_ready(&self, name: &str) -> Result<(), DynError> {
        self.owner.ensure_open()?;
        Ok(self.owner.cluster.wait_node_ready(name).await?)
    }

    /// Stops every node while keeping the cluster available for a later start.
    pub fn stop_all(&self) -> Result<(), DynError> {
        self.owner.ensure_open()?;
        self.owner.cluster.stop_all();
        Ok(())
    }

    /// Starts every node with its original deployment and waits for readiness.
    pub async fn start_all(&self) -> Result<(), DynError> {
        self.owner.ensure_open()?;
        start_all_nodes(&self.owner.cluster, self.node_count()).await?;
        self.owner.cluster.wait_network_ready().await?;
        Ok(())
    }

    /// Stops and starts every node, then waits for network readiness.
    pub async fn restart_all(&self) -> Result<(), DynError> {
        self.stop_all()?;
        self.start_all().await
    }

    pub(crate) fn cleanup_guard(&self) -> Box<dyn CleanupGuard> {
        Box::new(LocalClusterCleanup {
            owner: Arc::clone(&self.owner),
        })
    }
}

async fn start_all_nodes<E>(cluster: &ManualCluster<E>, node_count: usize) -> Result<(), DynError>
where
    E: LocalDeployerEnv,
{
    for index in 0..node_count {
        cluster.start_node(&format!("node-{index}")).await?;
    }

    Ok(())
}

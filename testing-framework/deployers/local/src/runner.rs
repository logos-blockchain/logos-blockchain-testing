use std::sync::Arc;

use async_trait::async_trait;
use testing_framework_core::{
    nodes::common::node::SpawnNodeError,
    scenario::{
        BlockFeed, BlockFeedTask, Deployer, DynError, Metrics, NodeClients, NodeControlCapability,
        RunContext, Runner, Scenario, ScenarioError, spawn_block_feed,
    },
    topology::{config::TopologyConfig, deployment::Topology, readiness::ReadinessError},
};
use thiserror::Error;
use tracing::{debug, info};

use crate::{
    manual::{LocalManualCluster, ManualClusterError},
    node_control::{LocalNodeManager, LocalNodeManagerSeed},
};
/// Spawns nodes as local processes, reusing the existing
/// integration harness.
#[derive(Clone)]
pub struct LocalDeployer {
    membership_check: bool,
}

/// Errors surfaced by the local deployer while driving a scenario.
#[derive(Debug, Error)]
pub enum LocalDeployerError {
    #[error("failed to spawn local topology: {source}")]
    Spawn {
        #[source]
        source: SpawnNodeError,
    },
    #[error("readiness probe failed: {source}")]
    ReadinessFailed {
        #[source]
        source: ReadinessError,
    },
    #[error("workload failed: {source}")]
    WorkloadFailed {
        #[source]
        source: DynError,
    },
    #[error("expectations failed: {source}")]
    ExpectationsFailed {
        #[source]
        source: DynError,
    },
}

impl From<ScenarioError> for LocalDeployerError {
    fn from(value: ScenarioError) -> Self {
        match value {
            ScenarioError::Workload(source) => Self::WorkloadFailed { source },
            ScenarioError::ExpectationCapture(source) | ScenarioError::Expectations(source) => {
                Self::ExpectationsFailed { source }
            }
        }
    }
}

#[async_trait]
impl Deployer<()> for LocalDeployer {
    type Error = LocalDeployerError;

    async fn deploy(&self, scenario: &Scenario<()>) -> Result<Runner, Self::Error> {
        info!(
            nodes = scenario.topology().nodes().len(),
            "starting local deployment"
        );
        let topology = Self::prepare_topology(scenario, self.membership_check).await?;
        let node_clients = NodeClients::from_topology(scenario.topology(), &topology);

        let (block_feed, block_feed_guard) = spawn_block_feed_with(&node_clients).await?;

        let context = RunContext::new(
            scenario.topology().clone(),
            Some(topology),
            node_clients,
            scenario.duration(),
            Metrics::empty(),
            block_feed,
            None,
        );

        Ok(Runner::new(context, Some(Box::new(block_feed_guard))))
    }
}

#[async_trait]
impl Deployer<NodeControlCapability> for LocalDeployer {
    type Error = LocalDeployerError;

    async fn deploy(
        &self,
        scenario: &Scenario<NodeControlCapability>,
    ) -> Result<Runner, Self::Error> {
        info!(
            nodes = scenario.topology().nodes().len(),
            "starting local deployment with node control"
        );

        let mut nodes = LocalNodeManager::spawn_initial_nodes(scenario.topology())
            .await
            .map_err(|source| LocalDeployerError::Spawn { source })?;

        if self.membership_check {
            let topology = Topology::from_nodes(nodes);

            wait_for_readiness(&topology).await.map_err(|source| {
                debug!(error = ?source, "local readiness failed");
                LocalDeployerError::ReadinessFailed { source }
            })?;

            nodes = topology.into_nodes();

            info!("local nodes are ready");
        } else {
            info!("skipping local membership readiness checks");
        }

        let node_control = Arc::new(LocalNodeManager::new_with_seed(
            scenario.topology().clone(),
            NodeClients::default(),
            LocalNodeManagerSeed::from_topology(scenario.topology()),
        ));

        node_control.initialize_with_nodes(nodes);
        let node_clients = node_control.node_clients();

        let (block_feed, block_feed_guard) = spawn_block_feed_with(&node_clients).await?;

        let context = RunContext::new(
            scenario.topology().clone(),
            None,
            node_clients,
            scenario.duration(),
            Metrics::empty(),
            block_feed,
            Some(node_control),
        );

        Ok(Runner::new(context, Some(Box::new(block_feed_guard))))
    }
}

impl LocalDeployer {
    #[must_use]
    /// Construct a local deployer.
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    /// Configure whether the deployer should enforce membership readiness
    /// checks.
    pub fn with_membership_check(mut self, enabled: bool) -> Self {
        self.membership_check = enabled;
        self
    }

    /// Build a manual cluster using this deployer's local implementation.
    pub fn manual_cluster(
        &self,
        config: TopologyConfig,
    ) -> Result<LocalManualCluster, ManualClusterError> {
        LocalManualCluster::from_config(config)
    }

    async fn prepare_topology<Caps>(
        scenario: &Scenario<Caps>,
        membership_check: bool,
    ) -> Result<Topology, LocalDeployerError> {
        let descriptors = scenario.topology();

        info!(nodes = descriptors.nodes().len(), "spawning local nodes");

        let topology = LocalNodeManager::spawn_initial_topology(descriptors)
            .await
            .map_err(|source| LocalDeployerError::Spawn { source })?;

        if membership_check {
            wait_for_readiness(&topology).await.map_err(|source| {
                debug!(error = ?source, "local readiness failed");
                LocalDeployerError::ReadinessFailed { source }
            })?;

            info!("local nodes are ready");
        } else {
            info!("skipping local membership readiness checks");
        }

        Ok(topology)
    }
}

impl Default for LocalDeployer {
    fn default() -> Self {
        Self {
            membership_check: true,
        }
    }
}

async fn wait_for_readiness(topology: &Topology) -> Result<(), ReadinessError> {
    info!("waiting for local network readiness");

    topology.wait_network_ready().await?;
    Ok(())
}

async fn spawn_block_feed_with(
    node_clients: &NodeClients,
) -> Result<(BlockFeed, BlockFeedTask), LocalDeployerError> {
    debug!(
        nodes = node_clients.node_clients().len(),
        "selecting node client for local block feed"
    );

    let Some(block_source_client) = node_clients.random_node() else {
        return Err(LocalDeployerError::WorkloadFailed {
            source: "block feed requires at least one node".into(),
        });
    };

    info!("starting block feed");

    spawn_block_feed(block_source_client)
        .await
        .map_err(workload_error)
}

fn workload_error(source: impl Into<DynError>) -> LocalDeployerError {
    LocalDeployerError::WorkloadFailed {
        source: source.into(),
    }
}

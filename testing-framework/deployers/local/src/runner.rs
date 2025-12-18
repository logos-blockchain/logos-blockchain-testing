use async_trait::async_trait;
use testing_framework_core::{
    scenario::{
        BlockFeed, BlockFeedTask, Deployer, DynError, Metrics, NodeClients, RunContext, Runner,
        Scenario, ScenarioError, spawn_block_feed,
    },
    topology::{
        deployment::{SpawnTopologyError, Topology},
        readiness::ReadinessError,
    },
};
use thiserror::Error;
use tracing::{debug, info};

/// Spawns validators and executors as local processes, reusing the existing
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
        source: SpawnTopologyError,
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
            validators = scenario.topology().validators().len(),
            executors = scenario.topology().executors().len(),
            membership_checks = self.membership_check,
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

impl LocalDeployer {
    #[must_use]
    /// Construct with membership readiness checks enabled.
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    /// Enable or disable membership readiness probes.
    pub const fn with_membership_check(mut self, enabled: bool) -> Self {
        self.membership_check = enabled;
        self
    }

    async fn prepare_topology(
        scenario: &Scenario<()>,
        membership_check: bool,
    ) -> Result<Topology, LocalDeployerError> {
        let descriptors = scenario.topology();
        info!(
            validators = descriptors.validators().len(),
            executors = descriptors.executors().len(),
            "spawning local validators/executors"
        );
        let topology = descriptors
            .clone()
            .spawn_local()
            .await
            .map_err(|source| LocalDeployerError::Spawn { source })?;

        let skip_membership = !membership_check;
        wait_for_readiness(&topology, skip_membership)
            .await
            .map_err(|source| {
                debug!(error = ?source, "local readiness failed");
                LocalDeployerError::ReadinessFailed { source }
            })?;

        info!("local nodes are ready");
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

async fn wait_for_readiness(
    topology: &Topology,
    skip_membership: bool,
) -> Result<(), ReadinessError> {
    info!("waiting for local network readiness");
    topology.wait_network_ready().await?;
    if skip_membership {
        // Allow callers to bypass deeper readiness for lightweight demos.
        return Ok(());
    }
    info!("waiting for membership readiness");
    topology.wait_membership_ready().await?;

    info!("waiting for DA balancer readiness");
    topology.wait_da_balancer_ready().await
}

async fn spawn_block_feed_with(
    node_clients: &NodeClients,
) -> Result<(BlockFeed, BlockFeedTask), LocalDeployerError> {
    debug!(
        validators = node_clients.validator_clients().len(),
        executors = node_clients.executor_clients().len(),
        "selecting validator client for local block feed"
    );

    let Some(block_source_client) = node_clients.random_validator().cloned() else {
        return Err(LocalDeployerError::WorkloadFailed {
            source: "block feed requires at least one validator".into(),
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

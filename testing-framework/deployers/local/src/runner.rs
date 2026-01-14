use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use nomos_utils::net::get_available_udp_port;
use rand::Rng as _;
use testing_framework_config::topology::configs::{
    consensus,
    runtime::{build_general_config_for_node, build_initial_peers},
};
use testing_framework_core::{
    nodes::{ApiClient, executor::Executor, validator::Validator},
    scenario::{
        BlockFeed, BlockFeedTask, Deployer, DynError, Metrics, NodeClients, NodeControlCapability,
        NodeControlHandle, RunContext, Runner, Scenario, ScenarioError, StartedNode,
        spawn_block_feed,
    },
    topology::{
        deployment::{SpawnTopologyError, Topology},
        generation::{GeneratedTopology, NodeRole},
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

#[async_trait]
impl Deployer<NodeControlCapability> for LocalDeployer {
    type Error = LocalDeployerError;

    async fn deploy(
        &self,
        scenario: &Scenario<NodeControlCapability>,
    ) -> Result<Runner, Self::Error> {
        info!(
            validators = scenario.topology().validators().len(),
            executors = scenario.topology().executors().len(),
            membership_checks = self.membership_check,
            "starting local deployment with node control"
        );

        let topology = Self::prepare_topology(scenario, self.membership_check).await?;
        let node_clients = NodeClients::from_topology(scenario.topology(), &topology);
        let node_control = Arc::new(LocalNodeControl::new(
            scenario.topology().clone(),
            node_clients.clone(),
        ));

        let (block_feed, block_feed_guard) = spawn_block_feed_with(&node_clients).await?;

        let context = RunContext::new(
            scenario.topology().clone(),
            Some(topology),
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

    async fn prepare_topology<Caps>(
        scenario: &Scenario<Caps>,
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

    let Some(block_source_client) = node_clients.random_validator() else {
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

struct LocalNodeControl {
    descriptors: GeneratedTopology,
    node_clients: NodeClients,
    base_consensus: consensus::GeneralConsensusConfig,
    state: Mutex<LocalNodeControlState>,
}

struct LocalNodeControlState {
    validator_count: usize,
    executor_count: usize,
    peer_ports: Vec<u16>,
    validators: Vec<Validator>,
    executors: Vec<Executor>,
}

#[async_trait]
impl NodeControlHandle for LocalNodeControl {
    async fn restart_validator(&self, _index: usize) -> Result<(), DynError> {
        Err("local deployer does not support restart_validator".into())
    }

    async fn restart_executor(&self, _index: usize) -> Result<(), DynError> {
        Err("local deployer does not support restart_executor".into())
    }

    async fn start_validator(&self, name: &str) -> Result<StartedNode, DynError> {
        self.start_node(NodeRole::Validator, name).await
    }

    async fn start_executor(&self, name: &str) -> Result<StartedNode, DynError> {
        self.start_node(NodeRole::Executor, name).await
    }
}

impl LocalNodeControl {
    fn new(descriptors: GeneratedTopology, node_clients: NodeClients) -> Self {
        let base_consensus = descriptors
            .validators()
            .first()
            .or_else(|| descriptors.executors().first())
            .map(|node| node.general.consensus_config.clone())
            .expect("generated topology must contain at least one node");

        let peer_ports = descriptors
            .nodes()
            .map(|node| node.network_port())
            .collect::<Vec<_>>();

        let state = LocalNodeControlState {
            validator_count: descriptors.validators().len(),
            executor_count: descriptors.executors().len(),
            peer_ports,
            validators: Vec::new(),
            executors: Vec::new(),
        };

        Self {
            descriptors,
            node_clients,
            base_consensus,
            state: Mutex::new(state),
        }
    }

    async fn start_node(&self, role: NodeRole, name: &str) -> Result<StartedNode, DynError> {
        let (peer_ports, node_name) = {
            let state = self.state.lock().expect("local node control lock poisoned");
            let index = match role {
                NodeRole::Validator => state.validator_count,
                NodeRole::Executor => state.executor_count,
            };

            let role_label = match role {
                NodeRole::Validator => "validator",
                NodeRole::Executor => "executor",
            };

            let label = if name.trim().is_empty() {
                format!("{role_label}-{index}")
            } else {
                format!("{role_label}-{name}")
            };

            (state.peer_ports.clone(), label)
        };

        let id = random_node_id();
        let network_port = allocate_udp_port("network port")?;
        let da_port = allocate_udp_port("DA port")?;
        let blend_port = allocate_udp_port("Blend port")?;

        let topology = self.descriptors.config();
        let initial_peers = build_initial_peers(&topology.network_params, &peer_ports);

        let general_config = build_general_config_for_node(
            id,
            network_port,
            initial_peers,
            da_port,
            blend_port,
            &topology.consensus_params,
            &topology.da_params,
            &topology.wallet_config,
            &self.base_consensus,
        )?;

        let api_client = match role {
            NodeRole::Validator => {
                let config = testing_framework_core::nodes::validator::create_validator_config(
                    general_config,
                );

                let node = Validator::spawn(config, &node_name).await?;
                let client = ApiClient::from_urls(node.url(), node.testing_url());

                self.node_clients.add_validator(client.clone());

                let mut state = self.state.lock().expect("local node control lock poisoned");

                state.peer_ports.push(network_port);
                state.validator_count += 1;
                state.validators.push(node);

                client
            }
            NodeRole::Executor => {
                let config =
                    testing_framework_core::nodes::executor::create_executor_config(general_config);

                let node = Executor::spawn(config, &node_name).await?;
                let client = ApiClient::from_urls(node.url(), node.testing_url());

                self.node_clients.add_executor(client.clone());

                let mut state = self.state.lock().expect("local node control lock poisoned");

                state.peer_ports.push(network_port);
                state.executor_count += 1;
                state.executors.push(node);

                client
            }
        };

        Ok(StartedNode {
            name: node_name,
            role,
            api: api_client,
        })
    }
}

fn random_node_id() -> [u8; 32] {
    let mut id = [0u8; 32];
    rand::thread_rng().fill(&mut id);
    id
}

fn allocate_udp_port(label: &'static str) -> Result<u16, DynError> {
    get_available_udp_port()
        .ok_or_else(|| format!("failed to allocate free UDP port for {label}").into())
}

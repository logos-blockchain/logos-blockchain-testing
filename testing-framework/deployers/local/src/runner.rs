use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use nomos_libp2p::Multiaddr;
use nomos_utils::net::get_available_udp_port;
use rand::Rng as _;
use testing_framework_config::topology::configs::{
    consensus,
    runtime::{build_general_config_for_node, build_initial_peers},
    time,
};
use testing_framework_core::{
    node_address_from_port,
    nodes::{ApiClient, executor::Executor, validator::Validator},
    scenario::{
        BlockFeed, BlockFeedTask, Deployer, DynError, Metrics, NodeClients, NodeControlCapability,
        NodeControlHandle, RunContext, Runner, Scenario, ScenarioError, StartNodeOptions,
        StartedNode, spawn_block_feed,
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
pub struct LocalDeployer {}

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
            "starting local deployment"
        );
        let topology = Self::prepare_topology(scenario).await?;
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
            "starting local deployment with node control"
        );

        let topology = Self::prepare_topology(scenario).await?;
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
    /// Construct a local deployer.
    pub fn new() -> Self {
        Self::default()
    }

    async fn prepare_topology<C>(scenario: &Scenario<C>) -> Result<Topology, LocalDeployerError> {
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

        wait_for_readiness(&topology).await.map_err(|source| {
            debug!(error = ?source, "local readiness failed");
            LocalDeployerError::ReadinessFailed { source }
        })?;

        info!("local nodes are ready");
        Ok(topology)
    }
}

impl Default for LocalDeployer {
    fn default() -> Self {
        Self {}
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
    base_time: time::GeneralTimeConfig,
    state: Mutex<LocalNodeControlState>,
}

struct LocalNodeControlState {
    validator_count: usize,
    executor_count: usize,
    peer_ports: Vec<u16>,
    peer_ports_by_name: HashMap<String, u16>,
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
        self.start_node(NodeRole::Validator, name, StartNodeOptions::default())
            .await
    }

    async fn start_executor(&self, name: &str) -> Result<StartedNode, DynError> {
        self.start_node(NodeRole::Executor, name, StartNodeOptions::default())
            .await
    }

    async fn start_validator_with(
        &self,
        name: &str,
        options: StartNodeOptions,
    ) -> Result<StartedNode, DynError> {
        self.start_node(NodeRole::Validator, name, options).await
    }

    async fn start_executor_with(
        &self,
        name: &str,
        options: StartNodeOptions,
    ) -> Result<StartedNode, DynError> {
        self.start_node(NodeRole::Executor, name, options).await
    }
}

impl LocalNodeControl {
    fn new(descriptors: GeneratedTopology, node_clients: NodeClients) -> Self {
        let base_node = descriptors
            .validators()
            .first()
            .or_else(|| descriptors.executors().first())
            .expect("generated topology must contain at least one node");

        let base_consensus = base_node.general.consensus_config.clone();
        let base_time = base_node.general.time_config.clone();

        let peer_ports = descriptors
            .nodes()
            .map(|node| node.network_port())
            .collect::<Vec<_>>();

        let peer_ports_by_name = descriptors
            .validators()
            .iter()
            .map(|node| (format!("validator-{}", node.index()), node.network_port()))
            .chain(
                descriptors
                    .executors()
                    .iter()
                    .map(|node| (format!("executor-{}", node.index()), node.network_port())),
            )
            .collect();

        let state = LocalNodeControlState {
            validator_count: descriptors.validators().len(),
            executor_count: descriptors.executors().len(),
            peer_ports,
            peer_ports_by_name,
            validators: Vec::new(),
            executors: Vec::new(),
        };

        Self {
            descriptors,
            node_clients,
            base_consensus,
            base_time,
            state: Mutex::new(state),
        }
    }

    async fn start_node(
        &self,
        role: NodeRole,
        name: &str,
        options: StartNodeOptions,
    ) -> Result<StartedNode, DynError> {
        let (peer_ports, peer_ports_by_name, node_name) = {
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

            if state.peer_ports_by_name.contains_key(&label) {
                return Err(format!("node name '{label}' already exists").into());
            }

            (
                state.peer_ports.clone(),
                state.peer_ports_by_name.clone(),
                label,
            )
        };

        let id = random_node_id();
        let network_port = allocate_udp_port("network port")?;
        let da_port = allocate_udp_port("DA port")?;
        let blend_port = allocate_udp_port("Blend port")?;

        let topology = self.descriptors.config();
        let initial_peers = if options.peer_names.is_empty() {
            build_initial_peers(&topology.network_params, &peer_ports)
        } else {
            resolve_peer_names(&peer_ports_by_name, &options.peer_names)?
        };

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
            &self.base_time,
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
                state
                    .peer_ports_by_name
                    .insert(node_name.clone(), network_port);
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
                state
                    .peer_ports_by_name
                    .insert(node_name.clone(), network_port);
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

fn resolve_peer_names(
    peer_ports_by_name: &HashMap<String, u16>,
    peer_names: &[String],
) -> Result<Vec<Multiaddr>, DynError> {
    let mut peers = Vec::with_capacity(peer_names.len());
    for name in peer_names {
        let port = peer_ports_by_name
            .get(name)
            .ok_or_else(|| format!("unknown peer name '{name}'"))?;
        peers.push(node_address_from_port(*port));
    }
    Ok(peers)
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

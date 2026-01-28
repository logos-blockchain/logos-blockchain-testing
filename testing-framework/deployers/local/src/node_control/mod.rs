use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

use nomos_node::config::RunConfig;
use testing_framework_config::topology::configs::{consensus, time};
use testing_framework_core::{
    nodes::{
        ApiClient,
        node::{Node, apply_node_config_patch, create_node_config},
    },
    scenario::{DynError, NodeControlHandle, StartNodeOptions, StartedNode},
    topology::{
        generation::{GeneratedTopology, find_expected_peer_counts},
        utils::multiaddr_port,
    },
};
use thiserror::Error;

mod config;
mod state;

use config::build_general_config_for;
use state::LocalDynamicState;
use testing_framework_core::scenario::NodeClients;

#[derive(Debug, Error)]
pub enum LocalDynamicError {
    #[error("failed to generate node config: {source}")]
    Config {
        #[source]
        source: testing_framework_config::topology::configs::GeneralConfigError,
    },
    #[error("failed to spawn node: {source}")]
    Spawn {
        #[source]
        source: testing_framework_core::nodes::common::node::SpawnNodeError,
    },
    #[error("{message}")]
    InvalidArgument { message: String },
    #[error("{message}")]
    PortAllocation { message: String },
    #[error("node config patch failed: {message}")]
    ConfigPatch { message: String },
}

pub struct LocalDynamicNodes {
    descriptors: GeneratedTopology,
    base_consensus: consensus::GeneralConsensusConfig,
    base_time: time::GeneralTimeConfig,
    node_clients: NodeClients,
    seed: LocalDynamicSeed,
    state: Mutex<LocalDynamicState>,
}

#[derive(Clone, Default)]
pub struct LocalDynamicSeed {
    pub node_count: usize,
    pub peer_ports: Vec<u16>,
    pub peer_ports_by_name: HashMap<String, u16>,
}

impl LocalDynamicSeed {
    #[must_use]
    pub fn from_topology(descriptors: &GeneratedTopology) -> Self {
        let peer_ports = descriptors
            .nodes()
            .iter()
            .map(|node| node.network_port())
            .collect::<Vec<_>>();

        let peer_ports_by_name = descriptors
            .nodes()
            .iter()
            .map(|node| (format!("node-{}", node.index()), node.network_port()))
            .collect();

        Self {
            node_count: descriptors.nodes().len(),
            peer_ports,
            peer_ports_by_name,
        }
    }
}

pub(crate) struct ReadinessNode {
    pub(crate) label: String,
    pub(crate) expected_peers: Option<usize>,
    pub(crate) api: ApiClient,
}

impl LocalDynamicNodes {
    pub fn new(descriptors: GeneratedTopology, node_clients: NodeClients) -> Self {
        Self::new_with_seed(descriptors, node_clients, LocalDynamicSeed::default())
    }

    pub fn new_with_seed(
        descriptors: GeneratedTopology,
        node_clients: NodeClients,
        seed: LocalDynamicSeed,
    ) -> Self {
        let base_node = descriptors
            .nodes()
            .first()
            .expect("generated topology must include at least one node");

        let base_consensus = base_node.general.consensus_config.clone();
        let base_time = base_node.general.time_config.clone();

        let state = LocalDynamicState {
            node_count: seed.node_count,
            peer_ports: seed.peer_ports.clone(),
            peer_ports_by_name: seed.peer_ports_by_name.clone(),
            clients_by_name: HashMap::new(),
            nodes: Vec::new(),
        };

        Self {
            descriptors,
            base_consensus,
            base_time,
            node_clients,
            seed,
            state: Mutex::new(state),
        }
    }

    #[must_use]
    pub fn node_client(&self, name: &str) -> Option<ApiClient> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        state.clients_by_name.get(name).cloned()
    }

    pub fn stop_all(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        state.nodes.clear();
        state.peer_ports.clone_from(&self.seed.peer_ports);
        state
            .peer_ports_by_name
            .clone_from(&self.seed.peer_ports_by_name);
        state.clients_by_name.clear();
        state.node_count = self.seed.node_count;
        self.node_clients.clear();
    }

    pub async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions,
    ) -> Result<StartedNode, LocalDynamicError> {
        self.start_node(name, options).await
    }

    pub(crate) fn readiness_nodes(&self) -> Vec<ReadinessNode> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let listen_ports = state
            .nodes
            .iter()
            .map(|node| node.config().user.network.backend.swarm.port)
            .collect::<Vec<_>>();

        let initial_peer_ports = state
            .nodes
            .iter()
            .map(|node| {
                node.config()
                    .user
                    .network
                    .backend
                    .initial_peers
                    .iter()
                    .filter_map(multiaddr_port)
                    .collect::<HashSet<u16>>()
            })
            .collect::<Vec<_>>();

        let expected_peer_counts = find_expected_peer_counts(&listen_ports, &initial_peer_ports);

        state
            .nodes
            .iter()
            .enumerate()
            .map(|(idx, node)| ReadinessNode {
                label: format!(
                    "node#{idx}@{}",
                    node.config().user.network.backend.swarm.port
                ),
                expected_peers: expected_peer_counts.get(idx).copied(),
                api: node.api().clone(),
            })
            .collect::<Vec<_>>()
    }

    async fn start_node(
        &self,
        name: &str,
        options: StartNodeOptions,
    ) -> Result<StartedNode, LocalDynamicError> {
        let (peer_ports, peer_ports_by_name, node_name, index) = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());

            let index = state.node_count;
            let label = if name.trim().is_empty() {
                format!("node-{index}")
            } else {
                format!("node-{name}")
            };

            if state.peer_ports_by_name.contains_key(&label) {
                return Err(LocalDynamicError::InvalidArgument {
                    message: format!("node name '{label}' already exists"),
                });
            }

            (
                state.peer_ports.clone(),
                state.peer_ports_by_name.clone(),
                label,
                index,
            )
        };

        let (general_config, network_port, descriptor_patch) = build_general_config_for(
            &self.descriptors,
            &self.base_consensus,
            &self.base_time,
            index,
            &peer_ports_by_name,
            &options,
            &peer_ports,
        )?;

        let config = build_node_config(
            general_config,
            descriptor_patch.as_ref(),
            options.config_patch.as_ref(),
        )?;

        let api_client = self
            .spawn_and_register_node(&node_name, network_port, config)
            .await?;

        Ok(StartedNode {
            name: node_name,
            api: api_client,
        })
    }

    async fn spawn_and_register_node(
        &self,
        node_name: &str,
        network_port: u16,
        config: RunConfig,
    ) -> Result<ApiClient, LocalDynamicError> {
        let node = Node::spawn(config, node_name)
            .await
            .map_err(|source| LocalDynamicError::Spawn { source })?;
        let client = node.api().clone();

        self.node_clients.add_node(client.clone());

        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        state.register_node(node_name, network_port, client.clone(), node);

        Ok(client)
    }
}

fn build_node_config(
    general_config: testing_framework_config::topology::configs::GeneralConfig,
    descriptor_patch: Option<&testing_framework_core::topology::config::NodeConfigPatch>,
    options_patch: Option<&testing_framework_core::topology::config::NodeConfigPatch>,
) -> Result<RunConfig, LocalDynamicError> {
    let mut config = create_node_config(general_config);
    config = apply_patch_if_needed(config, descriptor_patch)?;
    config = apply_patch_if_needed(config, options_patch)?;

    Ok(config)
}

fn apply_patch_if_needed(
    config: RunConfig,
    patch: Option<&testing_framework_core::topology::config::NodeConfigPatch>,
) -> Result<RunConfig, LocalDynamicError> {
    let Some(patch) = patch else {
        return Ok(config);
    };

    apply_node_config_patch(config, patch).map_err(|err| LocalDynamicError::ConfigPatch {
        message: err.to_string(),
    })
}

#[async_trait::async_trait]
impl NodeControlHandle for LocalDynamicNodes {
    async fn restart_node(&self, _index: usize) -> Result<(), DynError> {
        Err("local deployer does not support restart_node".into())
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
}

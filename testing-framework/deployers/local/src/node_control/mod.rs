use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

use nomos_node::Config as ValidatorConfig;
use testing_framework_config::topology::configs::{consensus, time};
use testing_framework_core::{
    nodes::{
        ApiClient,
        validator::{Validator, create_validator_config},
    },
    scenario::{DynError, NodeControlHandle, StartNodeOptions, StartedNode},
    topology::{
        generation::{GeneratedTopology, NodeRole, find_expected_peer_counts},
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
    pub validator_count: usize,
    pub peer_ports: Vec<u16>,
    pub peer_ports_by_name: HashMap<String, u16>,
}

impl LocalDynamicSeed {
    #[must_use]
    pub fn from_topology(descriptors: &GeneratedTopology) -> Self {
        let peer_ports = descriptors
            .nodes()
            .map(|node| node.network_port())
            .collect::<Vec<_>>();

        let peer_ports_by_name = descriptors
            .validators()
            .iter()
            .map(|node| (format!("validator-{}", node.index()), node.network_port()))
            .collect();

        Self {
            validator_count: descriptors.validators().len(),
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
            .validators()
            .first()
            .expect("generated topology must include at least one node");

        let base_consensus = base_node.general.consensus_config.clone();
        let base_time = base_node.general.time_config.clone();

        let state = LocalDynamicState {
            validator_count: seed.validator_count,
            peer_ports: seed.peer_ports.clone(),
            peer_ports_by_name: seed.peer_ports_by_name.clone(),
            clients_by_name: HashMap::new(),
            validators: Vec::new(),
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

        state.validators.clear();
        state.peer_ports.clone_from(&self.seed.peer_ports);
        state
            .peer_ports_by_name
            .clone_from(&self.seed.peer_ports_by_name);
        state.clients_by_name.clear();
        state.validator_count = self.seed.validator_count;
        self.node_clients.clear();
    }

    pub async fn start_validator_with(
        &self,
        name: &str,
        options: StartNodeOptions,
    ) -> Result<StartedNode, LocalDynamicError> {
        self.start_node(NodeRole::Validator, name, options).await
    }

    pub(crate) fn readiness_nodes(&self) -> Vec<ReadinessNode> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let listen_ports = state
            .validators
            .iter()
            .map(|node| node.config().network.backend.swarm.port)
            .collect::<Vec<_>>();

        let initial_peer_ports = state
            .validators
            .iter()
            .map(|node| {
                node.config()
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
            .validators
            .iter()
            .enumerate()
            .map(|(idx, node)| ReadinessNode {
                label: format!(
                    "validator#{idx}@{}",
                    node.config().network.backend.swarm.port
                ),
                expected_peers: expected_peer_counts.get(idx).copied(),
                api: node.api().clone(),
            })
            .collect::<Vec<_>>()
    }

    async fn start_node(
        &self,
        role: NodeRole,
        name: &str,
        options: StartNodeOptions,
    ) -> Result<StartedNode, LocalDynamicError> {
        let (peer_ports, peer_ports_by_name, node_name, index) = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());

            let (index, role_label) = match role {
                NodeRole::Validator => (state.validator_count, "validator"),
            };

            let label = if name.trim().is_empty() {
                format!("{role_label}-{index}")
            } else {
                format!("{role_label}-{name}")
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

        let (general_config, network_port) = build_general_config_for(
            &self.descriptors,
            &self.base_consensus,
            &self.base_time,
            role,
            index,
            &peer_ports_by_name,
            &options,
            &peer_ports,
        )?;

        let api_client = match role {
            NodeRole::Validator => {
                let config = create_validator_config(general_config);
                self.spawn_and_register_validator(&node_name, network_port, config)
                    .await?
            }
        };

        Ok(StartedNode {
            name: node_name,
            role,
            api: api_client,
        })
    }

    async fn spawn_and_register_validator(
        &self,
        node_name: &str,
        network_port: u16,
        config: ValidatorConfig,
    ) -> Result<ApiClient, LocalDynamicError> {
        let node = Validator::spawn(config, node_name)
            .await
            .map_err(|source| LocalDynamicError::Spawn { source })?;
        let client = node.api().clone();

        self.node_clients.add_validator(client.clone());

        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        state.register_validator(node_name, network_port, client.clone(), node);

        Ok(client)
    }
}

#[async_trait::async_trait]
impl NodeControlHandle for LocalDynamicNodes {
    async fn restart_validator(&self, _index: usize) -> Result<(), DynError> {
        Err("local deployer does not support restart_validator".into())
    }

    async fn start_validator(&self, name: &str) -> Result<StartedNode, DynError> {
        self.start_validator_with(name, StartNodeOptions::default())
            .await
            .map_err(|err| err.into())
    }

    async fn start_validator_with(
        &self,
        name: &str,
        options: StartNodeOptions,
    ) -> Result<StartedNode, DynError> {
        self.start_validator_with(name, options)
            .await
            .map_err(|err| err.into())
    }

    fn node_client(&self, name: &str) -> Option<ApiClient> {
        self.node_client(name)
    }
}

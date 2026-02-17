use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
};

use testing_framework_core::scenario::{
    Application, DynError, NodeClients, NodeControlHandle, ReadinessError, StartNodeOptions,
    StartedNode, wait_for_http_ports,
};
use thiserror::Error;

use crate::{
    env::{LocalDeployerEnv, Node, spawn_node_from_config},
    process::ProcessSpawnError,
};

mod state;

use state::LocalNodeManagerState;

#[derive(Clone)]
struct NodeStartSnapshot {
    peer_ports: Vec<u16>,
    peer_ports_by_name: HashMap<String, u16>,
    node_name: String,
    index: usize,
}

#[derive(Debug, Error)]
pub enum NodeManagerError {
    #[error("failed to generate node config: {source}")]
    Config {
        #[source]
        source: DynError,
    },
    #[error("failed to spawn node: {source}")]
    Spawn {
        #[source]
        source: DynError,
    },
    #[error("{message}")]
    InvalidArgument { message: String },
    #[error("{message}")]
    PortAllocation { message: String },
    #[error("node config patch failed: {message}")]
    ConfigPatch { message: String },
    #[error("node name '{name}' is unknown")]
    NodeName { name: String },
    #[error("failed to restart node: {source}")]
    Restart {
        #[source]
        source: DynError,
    },
}

pub struct NodeManager<E: LocalDeployerEnv> {
    descriptors: E::Deployment,
    node_clients: NodeClients<E>,
    keep_tempdir: bool,
    seed: NodeManagerSeed,
    state: Mutex<LocalNodeManagerState<E>>,
}

#[derive(Clone, Default)]
pub struct NodeManagerSeed {
    pub node_count: usize,
    pub peer_ports: Vec<u16>,
    pub peer_ports_by_name: HashMap<String, u16>,
}

impl<E: LocalDeployerEnv> NodeManager<E> {
    pub async fn spawn_initial_nodes(
        descriptors: &E::Deployment,
        keep_tempdir: bool,
    ) -> Result<Vec<Node<E>>, ProcessSpawnError> {
        let configs = E::build_initial_node_configs(descriptors)?;
        let mut spawned = Vec::with_capacity(configs.len());

        for (index, config_entry) in configs.into_iter().enumerate() {
            let persist_dir = E::initial_persist_dir(descriptors, &config_entry.name, index);
            spawned.push(
                spawn_node_from_config::<E>(
                    config_entry.name,
                    config_entry.config,
                    keep_tempdir,
                    persist_dir.as_deref(),
                )
                .await?,
            );
        }

        Ok(spawned)
    }
    pub fn new(descriptors: E::Deployment, node_clients: NodeClients<E>) -> Self {
        Self::new_with_seed(descriptors, node_clients, false, NodeManagerSeed::default())
    }

    pub fn new_with_seed(
        descriptors: E::Deployment,
        node_clients: NodeClients<E>,
        keep_tempdir: bool,
        seed: NodeManagerSeed,
    ) -> Self {
        let state = LocalNodeManagerState {
            node_count: seed.node_count,
            peer_ports: seed.peer_ports.clone(),
            peer_ports_by_name: seed.peer_ports_by_name.clone(),
            clients_by_name: HashMap::new(),
            indices_by_name: HashMap::new(),
            nodes: Vec::new(),
        };

        Self {
            descriptors,
            node_clients,
            keep_tempdir,
            seed,
            state: Mutex::new(state),
        }
    }

    #[must_use]
    pub fn node_client(&self, name: &str) -> Option<E::NodeClient> {
        let state = self.lock_state();

        state.clients_by_name.get(name).cloned()
    }

    #[must_use]
    pub fn node_pid(&self, name: &str) -> Option<u32> {
        let mut state = self.lock_state();

        let index = *state.indices_by_name.get(name)?;
        let node = state.nodes.get_mut(index)?;
        if node.is_running() {
            Some(node.pid())
        } else {
            None
        }
    }

    pub fn stop_all(&self) {
        let mut state = self.lock_state();

        state.nodes.clear();
        state.peer_ports.clone_from(&self.seed.peer_ports);
        state
            .peer_ports_by_name
            .clone_from(&self.seed.peer_ports_by_name);
        state.clients_by_name.clear();
        state.indices_by_name.clear();
        state.node_count = self.seed.node_count;
        self.node_clients.clear();
    }

    pub fn initialize_with_nodes(&self, nodes: Vec<Node<E>>) {
        self.node_clients.clear();

        let mut state = self.lock_state();
        clear_registered_nodes(&mut state);

        for (idx, node) in nodes.into_iter().enumerate() {
            let name = default_node_label(idx);
            let port = E::node_peer_port(&node);
            let client = node.client();

            self.node_clients.add_node(client.clone());
            state.register_node(&name, port, client, node);
        }
    }

    #[must_use]
    pub fn node_clients(&self) -> NodeClients<E> {
        self.node_clients.clone()
    }

    pub async fn wait_network_ready(&self) -> Result<(), ReadinessError> {
        let ports: Vec<_> = {
            let state = self.lock_state();
            state
                .nodes
                .iter()
                .map(|node| node.endpoints().api.port())
                .collect()
        };

        if ports.is_empty() {
            return Ok(());
        }

        wait_for_http_ports(&ports, E::readiness_endpoint_path()).await
    }

    pub async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, NodeManagerError> {
        let snapshot = self.start_snapshot(name)?;

        let mut built = E::build_node_config(
            &self.descriptors,
            snapshot.index,
            &snapshot.peer_ports_by_name,
            &options,
            &snapshot.peer_ports,
        )
        .map_err(|source| NodeManagerError::Config { source })?;

        if let Some(config_patch) = &options.config_patch {
            built.config =
                config_patch(built.config).map_err(|source| NodeManagerError::ConfigPatch {
                    message: source.to_string(),
                })?;
        }

        let client = self
            .spawn_and_register_node(
                &snapshot.node_name,
                built.network_port,
                built.config,
                options.persist_dir.as_deref(),
            )
            .await?;

        Ok(StartedNode {
            name: snapshot.node_name,
            client,
        })
    }

    pub async fn restart_node(&self, name: &str) -> Result<(), NodeManagerError> {
        let (index, mut node) = self.take_node(name)?;

        if let Err(source) = node.restart().await {
            self.put_node_back(index, node);

            return Err(NodeManagerError::Restart {
                source: source.into(),
            });
        }

        self.put_node_back(index, node);

        Ok(())
    }

    pub async fn stop_node(&self, name: &str) -> Result<(), NodeManagerError> {
        let (index, mut node) = self.take_node(name)?;

        node.stop().await;

        self.put_node_back(index, node);

        Ok(())
    }
    async fn spawn_and_register_node(
        &self,
        node_name: &str,
        network_port: u16,
        config: <E as Application>::NodeConfig,
        persist_dir: Option<&std::path::Path>,
    ) -> Result<E::NodeClient, NodeManagerError> {
        let node = spawn_node_from_config::<E>(
            node_name.to_string(),
            config,
            self.keep_tempdir,
            persist_dir,
        )
        .await
        .map_err(|source| NodeManagerError::Spawn {
            source: source.into(),
        })?;
        let client = node.client();

        self.node_clients.add_node(client.clone());

        let mut state = self.lock_state();

        state.register_node(node_name, network_port, client.clone(), node);

        Ok(client)
    }

    fn take_node(&self, name: &str) -> Result<(usize, Node<E>), NodeManagerError> {
        let mut state = self.lock_state();
        remove_node_from_state(&mut state, name)
    }

    fn put_node_back(&self, index: usize, node: Node<E>) {
        let mut state = self.lock_state();
        reinsert_node_at(&mut state, index, node);
    }

    fn start_snapshot(&self, requested_name: &str) -> Result<NodeStartSnapshot, NodeManagerError> {
        let state = self.lock_state();
        let index = state.node_count;
        let node_name = validate_new_node_name::<E>(state.node_count, &state, requested_name)?;

        Ok(NodeStartSnapshot {
            peer_ports: state.peer_ports.clone(),
            peer_ports_by_name: state.peer_ports_by_name.clone(),
            node_name,
            index,
        })
    }

    fn lock_state(&self) -> MutexGuard<'_, LocalNodeManagerState<E>> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn clear_registered_nodes<E: LocalDeployerEnv>(state: &mut LocalNodeManagerState<E>) {
    state.nodes.clear();
    state.peer_ports.clear();
    state.peer_ports_by_name.clear();
    state.clients_by_name.clear();
    state.indices_by_name.clear();
    state.node_count = 0;
}

fn validate_new_node_name<E: LocalDeployerEnv>(
    node_count: usize,
    state: &LocalNodeManagerState<E>,
    requested_name: &str,
) -> Result<String, NodeManagerError> {
    let label = normalize_node_name(node_count, requested_name);

    if state.peer_ports_by_name.contains_key(&label) {
        return Err(NodeManagerError::InvalidArgument {
            message: format!("node name '{label}' already exists"),
        });
    }

    Ok(label)
}

fn normalize_node_name(index: usize, requested_name: &str) -> String {
    if requested_name.trim().is_empty() {
        return default_node_label(index);
    }

    if requested_name.starts_with("node-") {
        return requested_name.to_string();
    }

    format!("node-{requested_name}")
}

fn default_node_label(index: usize) -> String {
    format!("node-{index}")
}

fn remove_node_from_state<E: LocalDeployerEnv>(
    state: &mut LocalNodeManagerState<E>,
    name: &str,
) -> Result<(usize, Node<E>), NodeManagerError> {
    let Some(index) = state.indices_by_name.get(name).copied() else {
        return Err(NodeManagerError::NodeName {
            name: name.to_string(),
        });
    };

    if index >= state.nodes.len() {
        return Err(NodeManagerError::NodeName {
            name: name.to_string(),
        });
    }

    Ok((index, state.nodes.remove(index)))
}

fn reinsert_node_at<E: LocalDeployerEnv>(
    state: &mut LocalNodeManagerState<E>,
    index: usize,
    node: Node<E>,
) {
    if index <= state.nodes.len() {
        state.nodes.insert(index, node);
    } else {
        state.nodes.push(node);
    }
}

#[async_trait::async_trait]
impl<E: LocalDeployerEnv> NodeControlHandle<E> for NodeManager<E> {
    async fn restart_node(&self, name: &str) -> Result<(), DynError> {
        self.restart_node(name).await.map_err(|err| err.into())
    }

    async fn stop_node(&self, name: &str) -> Result<(), DynError> {
        self.stop_node(name).await.map_err(|err| err.into())
    }

    async fn start_node(&self, name: &str) -> Result<StartedNode<E>, DynError> {
        self.start_node_with(name, StartNodeOptions::<E>::default())
            .await
            .map_err(|err| err.into())
    }

    async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, DynError> {
        self.start_node_with(name, options)
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

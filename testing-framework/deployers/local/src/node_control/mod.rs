use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, MutexGuard},
};

use testing_framework_core::scenario::{
    Application, DynError, HttpReadinessRequirement, NodeClients, NodeControlHandle,
    NodeRuntimeOptions, ReadinessError, StartNodeOptions, StartedNode,
};
use thiserror::Error;

use crate::{
    env::{
        LocalDeployerEnv, Node, build_initial_node_configs, build_launch_spec_with_args,
        build_node_from_template, initial_persist_dir, initial_snapshot_dir, node_peer_port,
        spawn_node_from_config, wait_for_local_readiness_ports,
    },
    process::ProcessSpawnError,
};

mod state;

use state::LocalNodeManagerState;

const RESTART_SETTLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const RESTART_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

#[derive(Clone)]
struct NodeStartSnapshot<Config> {
    peer_ports: Vec<u16>,
    peer_ports_by_name: HashMap<String, u16>,
    node_name: String,
    index: usize,
    template_config: Option<Config>,
}

#[derive(Clone, Copy)]
struct NodeReadinessTarget {
    port: u16,
    runtime: NodeRuntimeOptions,
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
    #[error("failed readiness check: {source}")]
    Readiness {
        #[source]
        source: ReadinessError,
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
    pub(crate) const fn deployment(&self) -> &E::Deployment {
        &self.descriptors
    }

    pub async fn spawn_initial_nodes(
        descriptors: &E::Deployment,
        keep_tempdir: bool,
    ) -> Result<Vec<Node<E>>, ProcessSpawnError> {
        let configs = build_initial_node_configs::<E>(descriptors)?;
        let mut spawned = Vec::with_capacity(configs.len());

        for (index, config_entry) in configs.into_iter().enumerate() {
            let persist_dir = initial_persist_dir::<E>(descriptors, &config_entry.name, index);
            let snapshot_dir = initial_snapshot_dir::<E>(descriptors, &config_entry.name, index);
            spawned.push(
                spawn_node_from_config::<E>(
                    config_entry.name,
                    config_entry.config,
                    keep_tempdir,
                    persist_dir.as_deref(),
                    snapshot_dir.as_deref(),
                    &[],
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
            runtime_by_name: HashMap::new(),
            stopped_names: HashSet::new(),
            restarting_names: HashSet::new(),
            nodes: Vec::new(),
            template_config: None,
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
        let node = state.nodes.get_mut(index)?.as_mut()?;
        if node.is_running() {
            Some(node.pid())
        } else {
            None
        }
    }

    pub fn stop_all(&self) {
        let mut state = self.lock_state();
        for node in state.nodes.iter_mut().flatten() {
            node.stop_blocking();
        }

        state.nodes.clear();
        state.peer_ports.clone_from(&self.seed.peer_ports);
        state
            .peer_ports_by_name
            .clone_from(&self.seed.peer_ports_by_name);
        state.clients_by_name.clear();
        state.indices_by_name.clear();
        state.runtime_by_name.clear();
        state.stopped_names.clear();
        state.node_count = self.seed.node_count;
        state.template_config = None;
        self.node_clients.clear();
    }

    pub fn initialize_with_nodes(&self, nodes: Vec<Node<E>>) {
        self.node_clients.clear();

        let mut state = self.lock_state();
        clear_registered_nodes(&mut state);

        for (idx, node) in nodes.into_iter().enumerate() {
            let name = default_node_label(idx);
            let port = node_peer_port::<E>(&node);
            let client = node.client();

            self.node_clients.add_node(client.clone());
            state.register_node(&name, port, client, NodeRuntimeOptions::default(), node);
        }
    }

    #[must_use]
    pub fn node_clients(&self) -> NodeClients<E> {
        self.node_clients.clone()
    }

    #[must_use]
    pub fn node_names(&self) -> Vec<String> {
        let state = self.lock_state();
        let mut entries: Vec<_> = state
            .indices_by_name
            .iter()
            .map(|(name, index)| (*index, name.clone()))
            .collect();
        entries.sort_unstable_by_key(|(index, _)| *index);
        entries.into_iter().map(|(_, name)| name).collect()
    }

    #[cfg(test)]
    fn running_probe_ports(&self) -> Vec<u16> {
        let state = self.lock_state();
        running_probe_ports_in(&state)
    }

    pub async fn wait_network_ready(&self) -> Result<(), ReadinessError> {
        let deadline = std::time::Instant::now() + RESTART_SETTLE_TIMEOUT;
        let ports = loop {
            let (ports, total_nodes, restarting) = {
                let state = self.lock_state();
                (
                    running_probe_ports_in(&state),
                    state.indices_by_name.len(),
                    state.restarting_names.len(),
                )
            };

            if !ports.is_empty() {
                break ports;
            }
            if total_nodes == 0 {
                return Ok(());
            }
            if restarting == 0 {
                return Err(ReadinessError::ProbeTimeout {
                    message: format!(
                        "all {total_nodes} nodes are stopped; no running nodes to await readiness"
                    ),
                });
            }
            if std::time::Instant::now() >= deadline {
                return Err(ReadinessError::ProbeTimeout {
                    message: format!(
                        "{restarting} restarting nodes did not return within {}s",
                        RESTART_SETTLE_TIMEOUT.as_secs()
                    ),
                });
            }
            tokio::time::sleep(RESTART_POLL_INTERVAL).await;
        };

        wait_for_local_readiness_ports::<E>(&ports, HttpReadinessRequirement::AllNodesReady, None)
            .await
    }

    pub async fn wait_node_ready(&self, name: &str) -> Result<(), NodeManagerError> {
        let target = self.readiness_target(name)?;

        wait_for_local_readiness_ports::<E>(
            &[target.port],
            HttpReadinessRequirement::AllNodesReady,
            target.runtime.start_timeout,
        )
        .await
        .map_err(|source| NodeManagerError::Readiness { source })
    }

    pub async fn start_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, NodeManagerError> {
        let snapshot = self.start_snapshot(name)?;

        let mut built = build_node_from_template::<E>(
            &self.descriptors,
            snapshot.index,
            &snapshot.peer_ports_by_name,
            &options,
            &snapshot.peer_ports,
            snapshot.template_config.as_ref(),
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
                options.runtime,
                options.persist_dir.as_deref(),
                options.snapshot_dir.as_deref(),
                &options.args,
            )
            .await?;

        Ok(StartedNode {
            name: snapshot.node_name,
            client,
        })
    }

    pub async fn restart_node(&self, name: &str) -> Result<(), NodeManagerError> {
        self.restart_node_with(name, StartNodeOptions::default())
            .await
    }

    pub async fn restart_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<(), NodeManagerError> {
        validate_restart_options(&options)?;

        let (index, mut node) = self.take_node(name)?;
        self.mark_node_restarting(name);

        let launch = match build_launch_spec_with_args::<E>(
            node.config(),
            node.working_dir(),
            name,
            &options.args,
        )
        .await
        {
            Ok(launch) => launch,
            Err(source) => {
                self.put_node_back(index, node);
                self.mark_node_stopped(name);
                return Err(NodeManagerError::Config { source });
            }
        };

        if let Err(source) = node.restart_with_launch(launch).await {
            self.put_node_back(index, node);
            self.mark_node_stopped(name);

            return Err(NodeManagerError::Restart {
                source: source.into(),
            });
        }

        self.put_node_back(index, node);
        self.mark_node_running(name);
        self.store_runtime_options(name, options.runtime);

        Ok(())
    }

    pub async fn stop_node(&self, name: &str) -> Result<(), NodeManagerError> {
        let (index, mut node) = self.take_node(name)?;

        node.stop().await;

        self.put_node_back(index, node);
        self.mark_node_stopped(name);

        Ok(())
    }
    async fn spawn_and_register_node(
        &self,
        node_name: &str,
        network_port: u16,
        config: <E as Application>::NodeConfig,
        runtime: NodeRuntimeOptions,
        persist_dir: Option<&std::path::Path>,
        snapshot_dir: Option<&std::path::Path>,
        extra_args: &[String],
    ) -> Result<E::NodeClient, NodeManagerError> {
        let node = spawn_node_from_config::<E>(
            node_name.to_string(),
            config,
            self.keep_tempdir,
            persist_dir,
            snapshot_dir,
            extra_args,
        )
        .await
        .map_err(|source| NodeManagerError::Spawn {
            source: source.into(),
        })?;
        let client = node.client();

        self.node_clients.add_node(client.clone());

        let mut state = self.lock_state();
        if state.template_config.is_none() && snapshot_dir.is_some() {
            state.template_config = Some(node.config().clone());
        }

        state.register_node(node_name, network_port, client.clone(), runtime, node);

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

    fn store_runtime_options(&self, name: &str, runtime: NodeRuntimeOptions) {
        let mut state = self.lock_state();
        state.runtime_by_name.insert(name.to_string(), runtime);
    }

    fn mark_node_stopped(&self, name: &str) {
        let mut state = self.lock_state();
        state.stopped_names.insert(name.to_string());
        state.restarting_names.remove(name);
    }

    fn mark_node_running(&self, name: &str) {
        let mut state = self.lock_state();
        state.stopped_names.remove(name);
        state.restarting_names.remove(name);
    }

    fn mark_node_restarting(&self, name: &str) {
        let mut state = self.lock_state();
        state.restarting_names.insert(name.to_string());
    }

    fn readiness_target(&self, name: &str) -> Result<NodeReadinessTarget, NodeManagerError> {
        let state = self.lock_state();
        let index = node_index(&state, name)?;
        let port = node_api_port(&state, index, name)?;
        let runtime = node_runtime_options(&state, name);

        Ok(NodeReadinessTarget { port, runtime })
    }

    fn start_snapshot(
        &self,
        requested_name: &str,
    ) -> Result<NodeStartSnapshot<E::NodeConfig>, NodeManagerError> {
        let state = self.lock_state();
        let index = state.node_count;
        let node_name = validate_new_node_name::<E>(state.node_count, &state, requested_name)?;

        Ok(NodeStartSnapshot {
            peer_ports: state.peer_ports.clone(),
            peer_ports_by_name: state.peer_ports_by_name.clone(),
            node_name,
            index,
            template_config: state.template_config.clone(),
        })
    }

    fn lock_state(&self) -> MutexGuard<'_, LocalNodeManagerState<E>> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn running_probe_ports_in<E: LocalDeployerEnv>(state: &LocalNodeManagerState<E>) -> Vec<u16> {
    state
        .indices_by_name
        .iter()
        .filter(|(name, _)| !state.stopped_names.contains(name.as_str()))
        .filter_map(|(_, index)| state.nodes.get(*index).and_then(Option::as_ref))
        .map(|node| node.endpoints().api.port())
        .collect()
}

fn clear_registered_nodes<E: LocalDeployerEnv>(state: &mut LocalNodeManagerState<E>) {
    state.nodes.clear();
    state.peer_ports.clear();
    state.peer_ports_by_name.clear();
    state.clients_by_name.clear();
    state.indices_by_name.clear();
    state.runtime_by_name.clear();
    state.stopped_names.clear();
    state.restarting_names.clear();
    state.node_count = 0;
    state.template_config = None;
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

fn validate_restart_options<E: LocalDeployerEnv>(
    options: &StartNodeOptions<E>,
) -> Result<(), NodeManagerError> {
    if options.peers.is_some() {
        return Err(unsupported_restart_override("peer selection"));
    }

    if options.config_override.is_some() {
        return Err(unsupported_restart_override("config override"));
    }

    if options.config_patch.is_some() {
        return Err(unsupported_restart_override("config patch"));
    }

    if options.persist_dir.is_some() {
        return Err(unsupported_restart_override("persist dir"));
    }

    if options.snapshot_dir.is_some() {
        return Err(unsupported_restart_override("snapshot dir"));
    }

    Ok(())
}

fn unsupported_restart_override(field: &str) -> NodeManagerError {
    NodeManagerError::InvalidArgument {
        message: format!("restart_node_with does not support {field} overrides"),
    }
}

fn node_index<E: LocalDeployerEnv>(
    state: &LocalNodeManagerState<E>,
    name: &str,
) -> Result<usize, NodeManagerError> {
    state
        .indices_by_name
        .get(name)
        .copied()
        .ok_or_else(|| NodeManagerError::NodeName {
            name: name.to_string(),
        })
}

fn node_api_port<E: LocalDeployerEnv>(
    state: &LocalNodeManagerState<E>,
    index: usize,
    name: &str,
) -> Result<u16, NodeManagerError> {
    state
        .nodes
        .get(index)
        .and_then(Option::as_ref)
        .map(|node| node.endpoints().api.port())
        .ok_or_else(|| NodeManagerError::NodeName {
            name: name.to_string(),
        })
}

fn node_runtime_options<E: LocalDeployerEnv>(
    state: &LocalNodeManagerState<E>,
    name: &str,
) -> NodeRuntimeOptions {
    state.runtime_by_name.get(name).copied().unwrap_or_default()
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

    let Some(node) = state.nodes.get_mut(index).and_then(Option::take) else {
        return Err(NodeManagerError::NodeName {
            name: name.to_string(),
        });
    };

    Ok((index, node))
}

fn reinsert_node_at<E: LocalDeployerEnv>(
    state: &mut LocalNodeManagerState<E>,
    index: usize,
    node: Node<E>,
) {
    if index < state.nodes.len() {
        state.nodes[index] = Some(node);
    } else {
        state.nodes.push(Some(node));
    }
}

#[async_trait::async_trait]
impl<E: LocalDeployerEnv> NodeControlHandle<E> for NodeManager<E> {
    async fn restart_node(&self, name: &str) -> Result<(), DynError> {
        self.restart_node(name).await.map_err(|err| err.into())
    }

    async fn restart_node_with(
        &self,
        name: &str,
        options: StartNodeOptions<E>,
    ) -> Result<(), DynError> {
        self.restart_node_with(name, options)
            .await
            .map_err(|err| err.into())
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

    async fn wait_node_ready(&self, name: &str) -> Result<(), DynError> {
        self.wait_node_ready(name).await.map_err(|err| err.into())
    }

    fn node_client(&self, name: &str) -> Option<E::NodeClient> {
        self.node_client(name)
    }

    fn node_names(&self) -> Vec<String> {
        self.node_names()
    }

    fn node_pid(&self, name: &str) -> Option<u32> {
        self.node_pid(name)
    }
}

#[cfg(test)]
mod tests {
    use std::{net::TcpListener, path::Path, time::Duration};

    use testing_framework_core::{
        scenario::{Application, DynError, NodeClients},
        topology::DeploymentDescriptor,
    };

    use super::NodeManager;
    use crate::{
        LaunchSpec, NodeEndpoints,
        env::{LocalDeployerEnv, LocalReadinessProbe, spawn_node_from_config},
    };

    #[derive(Clone)]
    struct SleepConfig {
        api_port: u16,
    }

    #[derive(Clone)]
    struct SleepTopology;

    impl DeploymentDescriptor for SleepTopology {
        fn node_count(&self) -> usize {
            2
        }
    }

    struct SleepEnv;

    #[async_trait::async_trait]
    impl Application for SleepEnv {
        type Deployment = SleepTopology;
        type NodeClient = ();
        type NodeConfig = SleepConfig;
    }

    #[async_trait::async_trait]
    impl LocalDeployerEnv for SleepEnv {
        async fn build_launch_spec(
            _config: &SleepConfig,
            _dir: &Path,
            _label: &str,
        ) -> Result<LaunchSpec, DynError> {
            Ok(LaunchSpec {
                binary: "/bin/sleep".into(),
                files: Vec::new(),
                args: vec!["300".into()],
                env: Vec::new(),
            })
        }

        fn node_endpoints(config: &SleepConfig) -> Result<NodeEndpoints, DynError> {
            Ok(NodeEndpoints::from_api_port(config.api_port))
        }

        fn node_client(_endpoints: &NodeEndpoints) -> Result<(), DynError> {
            Ok(())
        }

        fn readiness_probe() -> LocalReadinessProbe {
            LocalReadinessProbe::Tcp
        }
    }

    fn reserve_unbound_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind probe port");
        let port = listener.local_addr().expect("probe addr").port();
        drop(listener);
        port
    }

    async fn manager_with_two_nodes(ports: [u16; 2]) -> NodeManager<SleepEnv> {
        let manager = NodeManager::new(SleepTopology, NodeClients::default());
        let mut nodes = Vec::new();
        for port in ports {
            nodes.push(
                spawn_node_from_config::<SleepEnv>(
                    "node".to_string(),
                    SleepConfig { api_port: port },
                    false,
                    None,
                    None,
                    &[],
                )
                .await
                .expect("spawn sleep node"),
            );
        }
        manager.initialize_with_nodes(nodes);
        manager
    }

    #[tokio::test]
    async fn wait_network_ready_skips_stopped_nodes() {
        let live = TcpListener::bind("127.0.0.1:0").expect("bind live port");
        let live_port = live.local_addr().expect("live addr").port();
        let dead_port = reserve_unbound_port();
        let manager = manager_with_two_nodes([live_port, dead_port]).await;

        manager.stop_node("node-1").await.expect("stop node-1");

        tokio::time::timeout(Duration::from_secs(10), manager.wait_network_ready())
            .await
            .expect("readiness must not probe the stopped node")
            .expect("surviving node should be ready");
    }

    #[tokio::test]
    async fn sibling_lookups_stay_valid_while_a_node_is_taken_for_restart() {
        let port_a = reserve_unbound_port();
        let live = TcpListener::bind("127.0.0.1:0").expect("bind live port");
        let port_b = live.local_addr().expect("live addr").port();
        let manager = manager_with_two_nodes([port_a, port_b]).await;

        let (index, node) = manager.take_node("node-0").expect("take node-0");

        assert_eq!(manager.running_probe_ports(), vec![port_b]);
        let target = manager
            .readiness_target("node-1")
            .expect("sibling readiness target must resolve during a restart window");
        assert_eq!(target.port, port_b);

        manager.put_node_back(index, node);
        let mut ports = manager.running_probe_ports();
        ports.sort_unstable();
        let mut expected = vec![port_a, port_b];
        expected.sort_unstable();
        assert_eq!(ports, expected);
    }

    #[tokio::test]
    async fn wait_network_ready_waits_out_a_restart_window() {
        let live = TcpListener::bind("127.0.0.1:0").expect("bind live port");
        let port = live.local_addr().expect("live addr").port();
        let manager = std::sync::Arc::new(manager_with_two_nodes([port, port]).await);

        let (index_0, node_0) = manager.take_node("node-0").expect("take node-0");
        let (index_1, node_1) = manager.take_node("node-1").expect("take node-1");
        manager.mark_node_restarting("node-0");
        manager.mark_node_restarting("node-1");

        let restorer = std::sync::Arc::clone(&manager);
        let returner = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            restorer.put_node_back(index_0, node_0);
            restorer.mark_node_running("node-0");
            restorer.put_node_back(index_1, node_1);
            restorer.mark_node_running("node-1");
        });

        tokio::time::timeout(Duration::from_secs(10), manager.wait_network_ready())
            .await
            .expect("wait must complete once the restart window closes")
            .expect("restarted nodes should be awaited, not failed");
        returner.await.expect("restore task");
    }

    #[tokio::test]
    async fn wait_network_ready_fails_when_every_node_is_stopped() {
        let manager =
            manager_with_two_nodes([reserve_unbound_port(), reserve_unbound_port()]).await;

        manager.stop_node("node-0").await.expect("stop node-0");
        manager.stop_node("node-1").await.expect("stop node-1");

        let error = manager
            .wait_network_ready()
            .await
            .expect_err("readiness over zero running nodes must fail");
        assert!(matches!(
            error,
            testing_framework_core::scenario::ReadinessError::ProbeTimeout { .. }
        ));
    }

    #[tokio::test]
    async fn restart_returns_node_to_probe_set() {
        let port_a = reserve_unbound_port();
        let port_b = reserve_unbound_port();
        let manager = manager_with_two_nodes([port_a, port_b]).await;

        manager.stop_node("node-1").await.expect("stop node-1");
        assert_eq!(manager.running_probe_ports(), vec![port_a]);

        manager
            .restart_node("node-1")
            .await
            .expect("restart node-1");

        let mut ports = manager.running_probe_ports();
        ports.sort_unstable();
        let mut expected = vec![port_a, port_b];
        expected.sort_unstable();
        assert_eq!(ports, expected);
    }

    #[tokio::test]
    async fn node_names_reports_registered_names_in_index_order() {
        let manager =
            manager_with_two_nodes([reserve_unbound_port(), reserve_unbound_port()]).await;

        assert_eq!(manager.node_names(), vec!["node-0", "node-1"]);
    }
}

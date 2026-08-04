use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use testing_framework_core::scenario::{
    Application, DynError, HttpReadinessRequirement, ReadinessError, StartNodeOptions,
    wait_for_http_ports_with_requirement_and_timeout,
};

use crate::{
    LaunchSpec, NodeEndpoints,
    process::{ProcessNode, ProcessSpawnError},
};

mod helpers;
#[cfg(test)]
mod tests;

pub use helpers::{
    BuiltNodeConfig, LocalConfigArgMode, LocalNodePorts, LocalPeerNode, LocalProcessSpec,
    NodeConfigEntry, build_indexed_http_peers, build_indexed_node_configs,
    build_launch_spec_with_args, build_local_cluster_node_config, build_local_peer_nodes,
    default_yaml_launch_spec, discovered_node_access, preallocate_ports, reserve_local_node_ports,
    single_http_node_endpoints, text_config_launch_spec, text_node_config, yaml_config_launch_spec,
    yaml_node_config,
};

const TCP_READINESS_TIMEOUT: Duration = Duration::from_secs(60);
const TCP_READINESS_POLL_INTERVAL: Duration = Duration::from_millis(200);
const TCP_CONNECT_TIMEOUT: Duration = Duration::from_millis(100);

/// Readiness probe shape used by the local process deployer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalReadinessProbe {
    /// Probe `http://127.0.0.1:<api-port>/<path>` with GET.
    HttpGet { path: &'static str },
    /// Probe that the API port accepts TCP connections.
    Tcp,
}

/// Context passed while building a local node config.
pub struct LocalBuildContext<'a, E: Application> {
    /// Full deployment topology for the current scenario.
    pub topology: &'a E::Deployment,
    /// Zero-based node index being built.
    pub index: usize,
    /// Reserved local ports assigned to this node.
    pub ports: &'a LocalNodePorts,
    /// Peer nodes visible to this node after excluding `index`.
    pub peers: &'a [LocalPeerNode],
    /// Peer network ports for the current node view.
    pub peer_ports: &'a [u16],
    /// Peer ports keyed by application-defined port name.
    pub peer_ports_by_name: &'a HashMap<String, u16>,
    /// Start-time options for the node being built.
    pub options: &'a StartNodeOptions<E>,
    /// Optional template config to derive from during manual restart flows.
    pub template_config: Option<&'a E::NodeConfig>,
}

/// Spawned local process node for a concrete application environment.
pub type Node<E> = ProcessNode<<E as Application>::NodeConfig, <E as Application>::NodeClient>;

/// Advanced local deployer integration.
///
/// This is the full-control path. It exposes runner-facing hooks directly and
/// is intended for applications that need custom startup, endpoint discovery,
/// or lifecycle behavior.
#[async_trait]
pub trait LocalDeployerEnv: Application + Sized
where
    <Self as Application>::NodeConfig: Clone + Send + Sync + 'static,
{
    /// Prepares application-specific resources associated with a local cluster.
    fn prepare_local_cluster(_deployment: &Self::Deployment) {}

    /// Releases application-specific resources associated with a local cluster.
    fn cleanup_local_cluster(_deployment: &Self::Deployment) {}

    /// Returns named ports that should be reserved in addition to the main
    /// network port for each node.
    fn local_port_names() -> &'static [&'static str] {
        Self::initial_local_port_names()
    }

    /// Builds a node config and reserves any local ports needed for a node
    /// started after initial cluster creation.
    fn build_node_config(
        topology: &Self::Deployment,
        index: usize,
        peer_ports_by_name: &HashMap<String, u16>,
        options: &StartNodeOptions<Self>,
        peer_ports: &[u16],
    ) -> Result<BuiltNodeConfig<<Self as Application>::NodeConfig>, DynError> {
        Self::build_node_config_from_template(
            topology,
            index,
            peer_ports_by_name,
            options,
            peer_ports,
            None,
        )
    }

    /// Builds a node config from an optional template during restart or
    /// manual-cluster flows.
    fn build_node_config_from_template(
        topology: &Self::Deployment,
        index: usize,
        peer_ports_by_name: &HashMap<String, u16>,
        options: &StartNodeOptions<Self>,
        peer_ports: &[u16],
        template_config: Option<&<Self as Application>::NodeConfig>,
    ) -> Result<BuiltNodeConfig<<Self as Application>::NodeConfig>, DynError> {
        let mut reserved = reserve_local_node_ports(1, Self::local_port_names(), "node")
            .map_err(|source| -> DynError { source.into() })?;
        let ports = reserved
            .pop()
            .ok_or_else(|| std::io::Error::other("failed to reserve local node ports"))?;
        let network_port = ports.network_port();
        let config = Self::build_local_node_config(
            topology,
            index,
            &ports,
            peer_ports_by_name,
            options,
            peer_ports,
            template_config,
        )?;

        Ok(BuiltNodeConfig {
            config,
            network_port,
        })
    }

    /// Builds the initial local configs for every node in the deployment.
    fn build_initial_node_configs(
        topology: &Self::Deployment,
    ) -> Result<Vec<NodeConfigEntry<<Self as Application>::NodeConfig>>, ProcessSpawnError> {
        helpers::build_generated_initial_nodes::<Self>(
            topology,
            Self::initial_node_name_prefix(),
            Self::initial_local_port_names(),
            |context| {
                let config = Self::build_local_node_config_with_peers(
                    context.topology,
                    context.index,
                    context.ports,
                    context.peers,
                    context.peer_ports_by_name,
                    context.options,
                    context.template_config,
                )?;

                Ok(BuiltNodeConfig {
                    config,
                    network_port: context.ports.network_port(),
                })
            },
        )
    }

    /// Prefix used for generated node names such as `node-0`.
    fn initial_node_name_prefix() -> &'static str {
        "node"
    }

    /// Additional named ports to reserve for each initial node.
    fn initial_local_port_names() -> &'static [&'static str] {
        &[]
    }

    /// Builds one initial node config from already reserved ports.
    fn build_initial_node_config(
        topology: &Self::Deployment,
        index: usize,
        ports: &LocalNodePorts,
        peer_ports: &[u16],
    ) -> Result<<Self as Application>::NodeConfig, DynError> {
        let peer_ports = helpers::compact_peer_ports(peer_ports, index);
        let peer_ports_by_name = HashMap::new();
        let options = StartNodeOptions::<Self>::default();
        Self::build_local_node_config(
            topology,
            index,
            ports,
            &peer_ports_by_name,
            &options,
            &peer_ports,
            None,
        )
    }

    /// Builds a local node config from peer port information.
    fn build_local_node_config(
        topology: &Self::Deployment,
        index: usize,
        ports: &LocalNodePorts,
        peer_ports_by_name: &HashMap<String, u16>,
        options: &StartNodeOptions<Self>,
        peer_ports: &[u16],
        template_config: Option<&<Self as Application>::NodeConfig>,
    ) -> Result<<Self as Application>::NodeConfig, DynError> {
        let peers = build_local_peer_nodes(peer_ports, index);
        Self::build_local_node_config_with_peers(
            topology,
            index,
            ports,
            &peers,
            peer_ports_by_name,
            options,
            template_config,
        )
    }

    /// Builds a local node config from full peer node descriptions.
    fn build_local_node_config_with_peers(
        _topology: &Self::Deployment,
        _index: usize,
        _ports: &LocalNodePorts,
        _peers: &[LocalPeerNode],
        _peer_ports_by_name: &HashMap<String, u16>,
        _options: &StartNodeOptions<Self>,
        _template_config: Option<&<Self as Application>::NodeConfig>,
    ) -> Result<<Self as Application>::NodeConfig, DynError> {
        Err(std::io::Error::other(
            "build_local_node_config_with_peers is not implemented for this app",
        )
        .into())
    }

    /// Returns the initial persist directory for a node, if one should be
    /// mounted before startup.
    fn initial_persist_dir(
        _topology: &Self::Deployment,
        _node_name: &str,
        _index: usize,
    ) -> Option<PathBuf> {
        None
    }

    /// Returns the initial snapshot directory for a node, if one should be
    /// mounted before startup.
    fn initial_snapshot_dir(
        _topology: &Self::Deployment,
        _node_name: &str,
        _index: usize,
    ) -> Option<PathBuf> {
        None
    }

    /// Returns the default local process description for this app.
    fn local_process_spec() -> Option<LocalProcessSpec> {
        None
    }

    /// Returns the local process description for this concrete node config.
    ///
    /// Apps that need mixed-version or mixed-binary clusters can override this
    /// hook and choose a different binary provider per node while keeping the
    /// standard launch-spec rendering path.
    fn local_process_spec_for_node(
        _config: &<Self as Application>::NodeConfig,
        _label: &str,
    ) -> Option<LocalProcessSpec> {
        Self::local_process_spec()
    }

    /// Serializes a local node config into the file bytes written next to the
    /// spawned process.
    fn render_local_config(
        _config: &<Self as Application>::NodeConfig,
    ) -> Result<Vec<u8>, DynError> {
        Err(std::io::Error::other("render_local_config is not implemented for this app").into())
    }

    /// Builds the full launch spec for a local node process.
    async fn build_launch_spec(
        config: &<Self as Application>::NodeConfig,
        _dir: &Path,
        label: &str,
    ) -> Result<LaunchSpec, DynError> {
        let spec = Self::local_process_spec_for_node(config, label).ok_or_else(|| {
            std::io::Error::other("build_launch_spec is not implemented for this app")
        })?;
        let rendered = Self::render_local_config(config)?;
        helpers::rendered_config_launch_spec(rendered, &spec).await
    }

    /// Returns the main HTTP API port from a node config when the app follows
    /// the standard single-HTTP-endpoint pattern.
    fn http_api_port(_config: &<Self as Application>::NodeConfig) -> Option<u16> {
        None
    }

    /// Resolves the full local endpoint set exposed by a node.
    fn node_endpoints(
        config: &<Self as Application>::NodeConfig,
    ) -> Result<NodeEndpoints, DynError> {
        if let Some(port) = Self::http_api_port(config) {
            return Ok(NodeEndpoints {
                api: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
                extra_ports: HashMap::new(),
            });
        }

        Err(std::io::Error::other("node_endpoints is not implemented for this app").into())
    }

    /// Resolves the port peers should use for cluster traffic.
    fn node_peer_port(node: &Node<Self>) -> u16 {
        node.endpoints().api.port()
    }

    /// Builds a node client directly from the API endpoint when the default
    /// `NodeAccess`-based path is not suitable.
    fn node_client_from_api_endpoint(_api: SocketAddr) -> Option<Self::NodeClient> {
        None
    }

    /// Builds a node client from discovered local endpoints.
    fn node_client(endpoints: &NodeEndpoints) -> Result<Self::NodeClient, DynError> {
        if let Ok(client) =
            <Self as Application>::build_node_client(&discovered_node_access(endpoints))
        {
            return Ok(client);
        }

        if let Some(client) = Self::node_client_from_api_endpoint(endpoints.api) {
            return Ok(client);
        }

        Err(std::io::Error::other("node_client is not implemented for this app").into())
    }

    /// Returns the readiness endpoint path used for local HTTP probes.
    fn readiness_endpoint_path() -> &'static str {
        <Self as Application>::node_readiness_path()
    }

    /// Returns the readiness probe used for local process startup.
    fn readiness_probe() -> LocalReadinessProbe {
        LocalReadinessProbe::HttpGet {
            path: Self::readiness_endpoint_path(),
        }
    }

    /// Waits for any additional cluster-specific stabilization after the HTTP
    /// readiness probe succeeds.
    async fn wait_readiness_stable(_nodes: &[Node<Self>]) -> Result<(), DynError> {
        Ok(())
    }
}

/// Common local binary-app path.
///
/// This is the compact path for apps that:
/// - launch one local binary per node
/// - materialize one config file per node
/// - expose an HTTP API port used for readiness and discovery
#[async_trait]
pub trait LocalBinaryApp: Application + Sized
where
    <Self as Application>::NodeConfig: Clone + Send + Sync + 'static,
{
    /// Prefix used for generated initial node names such as `node-0`.
    fn initial_node_name_prefix() -> &'static str;

    /// Additional named ports to reserve for each local node.
    fn initial_local_port_names() -> &'static [&'static str] {
        &[]
    }

    /// Builds a local node config from full peer node descriptions.
    fn build_local_node_config_with_peers(
        topology: &Self::Deployment,
        index: usize,
        ports: &LocalNodePorts,
        peers: &[LocalPeerNode],
        peer_ports_by_name: &HashMap<String, u16>,
        options: &StartNodeOptions<Self>,
        template_config: Option<&<Self as Application>::NodeConfig>,
    ) -> Result<<Self as Application>::NodeConfig, DynError>;

    /// Returns the standard process description for launching one local node.
    fn local_process_spec() -> LocalProcessSpec;

    /// Serializes a local node config into the file bytes written next to the
    /// spawned process.
    fn render_local_config(config: &<Self as Application>::NodeConfig)
    -> Result<Vec<u8>, DynError>;

    /// Returns the main HTTP API port used for discovery and readiness.
    fn http_api_port(config: &<Self as Application>::NodeConfig) -> u16;

    /// Returns the readiness endpoint path used for local HTTP probes.
    fn readiness_endpoint_path() -> &'static str {
        <Self as Application>::node_readiness_path()
    }

    /// Returns the readiness probe used for local process startup.
    fn readiness_probe() -> LocalReadinessProbe {
        LocalReadinessProbe::HttpGet {
            path: Self::readiness_endpoint_path(),
        }
    }

    /// Waits for any additional cluster-specific stabilization after the HTTP
    /// readiness probe succeeds.
    async fn wait_readiness_stable(_nodes: &[Node<Self>]) -> Result<(), DynError> {
        Ok(())
    }
}

#[async_trait]
impl<T> LocalDeployerEnv for T
where
    T: LocalBinaryApp,
    <T as Application>::NodeConfig: Clone + Send + Sync + 'static,
{
    fn initial_node_name_prefix() -> &'static str {
        T::initial_node_name_prefix()
    }

    fn initial_local_port_names() -> &'static [&'static str] {
        T::initial_local_port_names()
    }

    fn build_local_node_config_with_peers(
        topology: &Self::Deployment,
        index: usize,
        ports: &LocalNodePorts,
        peers: &[LocalPeerNode],
        peer_ports_by_name: &HashMap<String, u16>,
        options: &StartNodeOptions<Self>,
        template_config: Option<&<Self as Application>::NodeConfig>,
    ) -> Result<<Self as Application>::NodeConfig, DynError> {
        T::build_local_node_config_with_peers(
            topology,
            index,
            ports,
            peers,
            peer_ports_by_name,
            options,
            template_config,
        )
    }

    fn local_process_spec() -> Option<LocalProcessSpec> {
        Some(T::local_process_spec())
    }

    fn render_local_config(
        config: &<Self as Application>::NodeConfig,
    ) -> Result<Vec<u8>, DynError> {
        T::render_local_config(config)
    }

    fn http_api_port(config: &<Self as Application>::NodeConfig) -> Option<u16> {
        Some(T::http_api_port(config))
    }

    fn readiness_endpoint_path() -> &'static str {
        T::readiness_endpoint_path()
    }

    fn readiness_probe() -> LocalReadinessProbe {
        T::readiness_probe()
    }

    async fn wait_readiness_stable(nodes: &[Node<Self>]) -> Result<(), DynError> {
        T::wait_readiness_stable(nodes).await
    }
}

pub(crate) fn build_node_from_template<E: LocalDeployerEnv>(
    topology: &E::Deployment,
    index: usize,
    peer_ports_by_name: &HashMap<String, u16>,
    options: &StartNodeOptions<E>,
    peer_ports: &[u16],
    template_config: Option<&E::NodeConfig>,
) -> Result<BuiltNodeConfig<E::NodeConfig>, DynError> {
    E::build_node_config_from_template(
        topology,
        index,
        peer_ports_by_name,
        options,
        peer_ports,
        template_config,
    )
}

pub(crate) fn build_initial_node_configs<E: LocalDeployerEnv>(
    topology: &E::Deployment,
) -> Result<Vec<NodeConfigEntry<E::NodeConfig>>, ProcessSpawnError> {
    E::build_initial_node_configs(topology)
}

pub(crate) fn initial_persist_dir<E: LocalDeployerEnv>(
    topology: &E::Deployment,
    node_name: &str,
    index: usize,
) -> Option<PathBuf> {
    E::initial_persist_dir(topology, node_name, index)
}

pub(crate) fn initial_snapshot_dir<E: LocalDeployerEnv>(
    topology: &E::Deployment,
    node_name: &str,
    index: usize,
) -> Option<PathBuf> {
    E::initial_snapshot_dir(topology, node_name, index)
}

pub(crate) fn node_client<E: LocalDeployerEnv>(
    endpoints: &NodeEndpoints,
) -> Result<E::NodeClient, DynError> {
    E::node_client(endpoints)
}

pub(crate) fn node_peer_port<E: LocalDeployerEnv>(node: &Node<E>) -> u16 {
    E::node_peer_port(node)
}

/// Waits for local readiness across the provided nodes and then applies
/// any app-specific stabilization hook.
pub async fn wait_local_readiness<E: LocalDeployerEnv>(
    nodes: &[Node<E>],
    requirement: HttpReadinessRequirement,
) -> Result<(), ReadinessError> {
    let ports: Vec<_> = nodes
        .iter()
        .map(|node| node.endpoints().api.port())
        .collect();

    wait_for_local_readiness_ports::<E>(&ports, requirement, None).await?;

    E::wait_readiness_stable(nodes)
        .await
        .map_err(|source| ReadinessError::ClusterStable { source })
}

/// Backward-compatible name for local readiness checks that used to be
/// HTTP-only.
pub async fn wait_local_http_readiness<E: LocalDeployerEnv>(
    nodes: &[Node<E>],
    requirement: HttpReadinessRequirement,
) -> Result<(), ReadinessError> {
    wait_local_readiness::<E>(nodes, requirement).await
}

pub(crate) async fn wait_for_local_readiness_ports<E: LocalDeployerEnv>(
    ports: &[u16],
    requirement: HttpReadinessRequirement,
    timeout: Option<Duration>,
) -> Result<(), ReadinessError> {
    match E::readiness_probe() {
        LocalReadinessProbe::HttpGet { path } => {
            wait_for_http_ports_with_requirement_and_timeout(ports, path, requirement, timeout)
                .await
        }
        LocalReadinessProbe::Tcp => {
            wait_for_tcp_ports_with_requirement(ports, requirement, timeout).await
        }
    }
}

async fn wait_for_tcp_ports_with_requirement(
    ports: &[u16],
    requirement: HttpReadinessRequirement,
    timeout: Option<Duration>,
) -> Result<(), ReadinessError> {
    if ports.is_empty() {
        return Ok(());
    }

    let timeout = timeout.unwrap_or(TCP_READINESS_TIMEOUT);
    let deadline = Instant::now() + timeout;

    loop {
        let statuses = collect_tcp_statuses(ports);
        if tcp_requirement_satisfied(&statuses, requirement) {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(ReadinessError::ProbeTimeout {
                message: format_tcp_timeout_message(&statuses, requirement),
            });
        }

        tokio::time::sleep(TCP_READINESS_POLL_INTERVAL).await;
    }
}

fn collect_tcp_statuses(ports: &[u16]) -> Vec<(u16, bool)> {
    ports
        .iter()
        .map(|port| (*port, tcp_port_ready(*port)))
        .collect()
}

fn tcp_port_ready(port: u16) -> bool {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&addr, TCP_CONNECT_TIMEOUT).is_ok()
}

fn tcp_requirement_satisfied(
    statuses: &[(u16, bool)],
    requirement: HttpReadinessRequirement,
) -> bool {
    let ready = tcp_ready_count(statuses);

    match requirement {
        HttpReadinessRequirement::AllNodesReady => ready == statuses.len(),
        HttpReadinessRequirement::AnyNodeReady => ready >= 1,
        HttpReadinessRequirement::AtLeast(min_ready) => ready >= min_ready,
    }
}

fn tcp_ready_count(statuses: &[(u16, bool)]) -> usize {
    statuses.iter().filter(|(_, ready)| *ready).count()
}

fn format_tcp_timeout_message(
    statuses: &[(u16, bool)],
    requirement: HttpReadinessRequirement,
) -> String {
    let ready = tcp_ready_count(statuses);
    let failing = statuses
        .iter()
        .filter_map(|(port, ok)| (!ok).then_some(port.to_string()))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "timed out waiting for TCP readiness {:?}; ready={}, total={}, failing ports: {}",
        requirement,
        ready,
        statuses.len(),
        if failing.is_empty() {
            "<none>"
        } else {
            &failing
        }
    )
}

/// Spawns a local process node from an already prepared config value.
pub async fn spawn_node_from_config<E: LocalDeployerEnv>(
    label: String,
    config: <E as Application>::NodeConfig,
    keep_tempdir: bool,
    persist_dir: Option<&std::path::Path>,
    snapshot_dir: Option<&std::path::Path>,
    extra_args: &[String],
) -> Result<Node<E>, ProcessSpawnError> {
    let extra_args = extra_args.to_vec();

    ProcessNode::spawn(
        &label,
        config,
        move |config, dir, label| {
            let extra_args = extra_args.clone();
            Box::pin(async move {
                build_launch_spec_with_args::<E>(config, dir, label, &extra_args).await
            })
        },
        E::node_endpoints,
        keep_tempdir,
        persist_dir,
        snapshot_dir,
        E::node_client,
    )
    .await
}

use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
};

use serde::Serialize;
use testing_framework_core::{
    scenario::{
        Application, ClusterNodeConfigApplication, ClusterNodeView, ClusterPeerView, DynError,
        HttpReadinessRequirement, NodeAccess, ReadinessError, StartNodeOptions,
        wait_for_http_ports_with_requirement,
    },
    topology::DeploymentDescriptor,
};

use crate::process::{LaunchSpec, NodeEndpointPort, NodeEndpoints, ProcessNode, ProcessSpawnError};

pub type Node<E> = ProcessNode<<E as Application>::NodeConfig, <E as Application>::NodeClient>;

pub struct BuiltNodeConfig<Config> {
    pub config: Config,
    pub network_port: u16,
}

pub struct NodeConfigEntry<NodeConfigValue> {
    pub name: String,
    pub config: NodeConfigValue,
}

pub struct LocalNodePorts {
    network_port: u16,
    named_ports: HashMap<&'static str, u16>,
}

impl LocalNodePorts {
    #[must_use]
    pub fn network_port(&self) -> u16 {
        self.network_port
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<u16> {
        self.named_ports.get(name).copied()
    }

    pub fn require(&self, name: &str) -> Result<u16, DynError> {
        self.get(name)
            .ok_or_else(|| format!("missing reserved local port '{name}'").into())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&'static str, u16)> + '_ {
        self.named_ports.iter().map(|(name, port)| (*name, *port))
    }
}

#[derive(Clone, Debug)]
pub struct LocalPeerNode {
    index: usize,
    network_port: u16,
}

impl LocalPeerNode {
    #[must_use]
    pub fn index(&self) -> usize {
        self.index
    }

    #[must_use]
    pub fn network_port(&self) -> u16 {
        self.network_port
    }

    #[must_use]
    pub fn http_address(&self) -> String {
        format!("127.0.0.1:{}", self.network_port)
    }

    #[must_use]
    pub fn authority(&self) -> String {
        self.http_address()
    }
}

#[derive(Clone, Default)]
pub struct LocalProcessSpec {
    pub binary_env_var: String,
    pub binary_name: String,
    pub config_file_name: String,
    pub config_arg: String,
    pub extra_args: Vec<String>,
    pub env: Vec<crate::process::LaunchEnvVar>,
}

impl LocalProcessSpec {
    #[must_use]
    pub fn new(binary_env_var: &str, binary_name: &str) -> Self {
        Self {
            binary_env_var: binary_env_var.to_owned(),
            binary_name: binary_name.to_owned(),
            config_file_name: "config.yaml".to_owned(),
            config_arg: "--config".to_owned(),
            extra_args: Vec::new(),
            env: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_config_file(mut self, file_name: &str, arg: &str) -> Self {
        self.config_file_name = file_name.to_owned();
        self.config_arg = arg.to_owned();
        self
    }

    #[must_use]
    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.push(crate::process::LaunchEnvVar::new(key, value));
        self
    }

    #[must_use]
    pub fn with_rust_log(self, value: &str) -> Self {
        self.with_env("RUST_LOG", value)
    }

    #[must_use]
    pub fn with_args(mut self, args: impl IntoIterator<Item = String>) -> Self {
        self.extra_args.extend(args);
        self
    }
}

pub fn preallocate_ports(count: usize, label: &str) -> Result<Vec<u16>, ProcessSpawnError> {
    (0..count)
        .map(|_| crate::process::allocate_available_port())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| ProcessSpawnError::Config {
            source: format!("failed to pre-allocate {label} ports: {source}").into(),
        })
}

pub fn build_indexed_node_configs<T>(
    count: usize,
    name_prefix: &str,
    build: impl FnMut(usize) -> T,
) -> Vec<NodeConfigEntry<T>> {
    (0..count)
        .map(build)
        .enumerate()
        .map(|(index, config)| NodeConfigEntry {
            name: format!("{name_prefix}-{index}"),
            config,
        })
        .collect()
}

pub fn reserve_local_node_ports(
    count: usize,
    names: &[&'static str],
    label: &str,
) -> Result<Vec<LocalNodePorts>, ProcessSpawnError> {
    let network_ports = preallocate_ports(count, label)?;
    let mut named_by_role = HashMap::new();
    for name in names {
        named_by_role.insert(*name, preallocate_ports(count, &format!("{label} {name}"))?);
    }

    Ok((0..count)
        .map(|index| LocalNodePorts {
            network_port: network_ports[index],
            named_ports: named_by_role
                .iter()
                .map(|(name, ports)| (*name, ports[index]))
                .collect(),
        })
        .collect())
}

pub fn single_http_node_endpoints(port: u16) -> NodeEndpoints {
    NodeEndpoints::from_api_port(port)
}

pub fn build_local_cluster_node_config<E>(
    index: usize,
    ports: &LocalNodePorts,
    peers: &[LocalPeerNode],
) -> Result<<E as Application>::NodeConfig, DynError>
where
    E: ClusterNodeConfigApplication,
{
    let mut node = ClusterNodeView::new(index, "127.0.0.1", ports.network_port());
    for (name, port) in ports.iter() {
        node = node.with_named_port(name, port);
    }

    let peer_views = peers
        .iter()
        .map(|peer| ClusterPeerView::new(peer.index(), "127.0.0.1", peer.network_port()))
        .collect::<Vec<_>>();

    E::build_cluster_node_config(&node, &peer_views).map_err(Into::into)
}

pub fn discovered_node_access(endpoints: &NodeEndpoints) -> NodeAccess {
    let mut access = NodeAccess::new("127.0.0.1", endpoints.api.port());

    for (key, port) in &endpoints.extra_ports {
        match key {
            NodeEndpointPort::TestingApi => {
                access = access.with_testing_port(*port);
            }
            NodeEndpointPort::Custom(name) => {
                access = access.with_named_port(name.clone(), *port);
            }
            NodeEndpointPort::Network => {}
        }
    }

    access
}

pub fn build_indexed_http_peers<T>(
    node_count: usize,
    self_index: usize,
    peer_ports: &[u16],
    mut build_peer: impl FnMut(usize, String) -> T,
) -> Vec<T> {
    (0..node_count)
        .filter(|&i| i != self_index)
        .map(|i| build_peer(i, format!("127.0.0.1:{}", peer_ports[i])))
        .collect()
}

fn compact_peer_ports(peer_ports: &[u16], self_index: usize) -> Vec<u16> {
    peer_ports
        .iter()
        .enumerate()
        .filter_map(|(index, port)| (index != self_index).then_some(*port))
        .collect()
}

pub fn build_local_peer_nodes(peer_ports: &[u16], self_index: usize) -> Vec<LocalPeerNode> {
    peer_ports
        .iter()
        .enumerate()
        .filter_map(|(index, port)| {
            (index != self_index).then_some(LocalPeerNode {
                index,
                network_port: *port,
            })
        })
        .collect()
}

pub fn yaml_config_launch_spec<T: Serialize>(
    config: &T,
    spec: &LocalProcessSpec,
) -> Result<LaunchSpec, DynError> {
    let config_yaml = serde_yaml::to_string(config)?;
    rendered_config_launch_spec(config_yaml.into_bytes(), spec)
}

pub fn text_config_launch_spec(
    rendered_config: impl Into<Vec<u8>>,
    spec: &LocalProcessSpec,
) -> Result<LaunchSpec, DynError> {
    rendered_config_launch_spec(rendered_config.into(), spec)
}

pub fn default_yaml_launch_spec<T: Serialize>(
    config: &T,
    binary_env_var: &str,
    binary_name: &str,
    rust_log: &str,
) -> Result<LaunchSpec, DynError> {
    yaml_config_launch_spec(
        config,
        &LocalProcessSpec::new(binary_env_var, binary_name).with_rust_log(rust_log),
    )
}

pub fn yaml_node_config<T: Serialize>(config: &T) -> Result<Vec<u8>, DynError> {
    Ok(serde_yaml::to_string(config)?.into_bytes())
}

pub fn text_node_config(rendered_config: impl Into<Vec<u8>>) -> Vec<u8> {
    rendered_config.into()
}

fn rendered_config_launch_spec(
    rendered_config: Vec<u8>,
    spec: &LocalProcessSpec,
) -> Result<LaunchSpec, DynError> {
    let binary = resolve_binary(spec);
    let mut args = vec![spec.config_arg.clone(), spec.config_file_name.clone()];
    args.extend(spec.extra_args.iter().cloned());

    Ok(LaunchSpec {
        binary,
        files: vec![crate::process::LaunchFile {
            relative_path: spec.config_file_name.clone().into(),
            contents: rendered_config,
        }],
        args,
        env: spec.env.clone(),
    })
}

fn resolve_binary(spec: &LocalProcessSpec) -> PathBuf {
    std::env::var(&spec.binary_env_var)
        .map(PathBuf::from)
        .or_else(|_| which::which(&spec.binary_name))
        .unwrap_or_else(|_| {
            let mut path = std::env::current_dir().unwrap_or_default();
            let mut debug = path.clone();
            debug.push(format!("target/debug/{}", spec.binary_name));
            if debug.exists() {
                return debug;
            }

            path.push(format!("target/release/{}", spec.binary_name));
            path
        })
}

#[async_trait::async_trait]
pub trait LocalDeployerEnv: Application + Sized
where
    <Self as Application>::NodeConfig: Clone + Send + Sync + 'static,
{
    fn local_port_names() -> &'static [&'static str] {
        Self::initial_local_port_names()
    }

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

    fn build_initial_node_configs(
        topology: &Self::Deployment,
    ) -> Result<Vec<NodeConfigEntry<<Self as Application>::NodeConfig>>, ProcessSpawnError> {
        let reserved_ports = reserve_local_node_ports(
            topology.node_count(),
            Self::initial_local_port_names(),
            Self::initial_node_name_prefix(),
        )?;
        let peer_ports = reserved_ports
            .iter()
            .map(LocalNodePorts::network_port)
            .collect::<Vec<_>>();

        let mut configs = Vec::with_capacity(topology.node_count());
        for (index, ports) in reserved_ports.iter().enumerate() {
            let config = Self::build_initial_node_config(topology, index, ports, &peer_ports)
                .map_err(|source| ProcessSpawnError::Config { source })?;
            configs.push(NodeConfigEntry {
                name: format!("{}-{index}", Self::initial_node_name_prefix()),
                config,
            });
        }

        Ok(configs)
    }

    fn initial_node_name_prefix() -> &'static str {
        "node"
    }

    fn initial_local_port_names() -> &'static [&'static str] {
        &[]
    }

    fn build_initial_node_config(
        topology: &Self::Deployment,
        index: usize,
        ports: &LocalNodePorts,
        peer_ports: &[u16],
    ) -> Result<<Self as Application>::NodeConfig, DynError> {
        let compact_peer_ports = compact_peer_ports(peer_ports, index);
        let peer_ports_by_name = HashMap::new();
        let options = StartNodeOptions::<Self>::default();
        Self::build_local_node_config(
            topology,
            index,
            ports,
            &peer_ports_by_name,
            &options,
            &compact_peer_ports,
            None,
        )
    }

    fn build_local_node_config(
        _topology: &Self::Deployment,
        _index: usize,
        _ports: &LocalNodePorts,
        _peer_ports_by_name: &HashMap<String, u16>,
        _options: &StartNodeOptions<Self>,
        _peer_ports: &[u16],
        _template_config: Option<&<Self as Application>::NodeConfig>,
    ) -> Result<<Self as Application>::NodeConfig, DynError> {
        let peers = build_local_peer_nodes(_peer_ports, _index);
        Self::build_local_node_config_with_peers(
            _topology,
            _index,
            _ports,
            &peers,
            _peer_ports_by_name,
            _options,
            _template_config,
        )
    }

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

    fn initial_persist_dir(
        _topology: &Self::Deployment,
        _node_name: &str,
        _index: usize,
    ) -> Option<PathBuf> {
        None
    }

    fn initial_snapshot_dir(
        _topology: &Self::Deployment,
        _node_name: &str,
        _index: usize,
    ) -> Option<PathBuf> {
        None
    }

    fn local_process_spec() -> Option<LocalProcessSpec> {
        None
    }

    fn render_local_config(
        _config: &<Self as Application>::NodeConfig,
    ) -> Result<Vec<u8>, DynError> {
        Err(std::io::Error::other("render_local_config is not implemented for this app").into())
    }

    fn build_launch_spec(
        config: &<Self as Application>::NodeConfig,
        _dir: &Path,
        _label: &str,
    ) -> Result<LaunchSpec, DynError> {
        let spec = Self::local_process_spec().ok_or_else(|| {
            std::io::Error::other("build_launch_spec is not implemented for this app")
        })?;
        let rendered = Self::render_local_config(config)?;
        rendered_config_launch_spec(rendered, &spec)
    }

    fn http_api_port(_config: &<Self as Application>::NodeConfig) -> Option<u16> {
        None
    }

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

    fn node_peer_port(node: &Node<Self>) -> u16 {
        node.endpoints().api.port()
    }

    fn node_client_from_api_endpoint(_api: SocketAddr) -> Option<Self::NodeClient> {
        None
    }

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

    fn readiness_endpoint_path() -> &'static str {
        <Self as Application>::node_readiness_path()
    }

    async fn wait_readiness_stable(_nodes: &[Node<Self>]) -> Result<(), DynError> {
        Ok(())
    }
}

pub async fn wait_local_http_readiness<E: LocalDeployerEnv>(
    nodes: &[Node<E>],
    requirement: HttpReadinessRequirement,
) -> Result<(), ReadinessError> {
    let ports: Vec<_> = nodes
        .iter()
        .map(|node| node.endpoints().api.port())
        .collect();
    wait_for_http_ports_with_requirement(&ports, E::readiness_endpoint_path(), requirement).await?;

    E::wait_readiness_stable(nodes)
        .await
        .map_err(|source| ReadinessError::ClusterStable { source })
}

pub async fn spawn_node_from_config<E: LocalDeployerEnv>(
    label: String,
    config: <E as Application>::NodeConfig,
    keep_tempdir: bool,
    persist_dir: Option<&std::path::Path>,
    snapshot_dir: Option<&std::path::Path>,
) -> Result<Node<E>, ProcessSpawnError> {
    ProcessNode::spawn(
        &label,
        config,
        E::build_launch_spec,
        E::node_endpoints,
        keep_tempdir,
        persist_dir,
        snapshot_dir,
        E::node_client,
    )
    .await
}

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use testing_framework_core::{
        scenario::{Application, DynError, Feed, FeedRuntime, NodeClients},
        topology::DeploymentDescriptor,
    };

    use super::*;

    static STABLE_CALLS: AtomicUsize = AtomicUsize::new(0);

    #[derive(Clone, Default)]
    struct DummyFeed;

    impl Feed for DummyFeed {
        type Subscription = ();

        fn subscribe(&self) -> Self::Subscription {}
    }

    #[derive(Default)]
    struct DummyFeedRuntime;

    #[async_trait]
    impl FeedRuntime for DummyFeedRuntime {
        type Feed = DummyFeed;

        async fn run(self: Box<Self>) {}
    }

    #[derive(Clone)]
    struct DummyConfig;

    #[derive(Clone)]
    struct DummyTopology;

    impl DeploymentDescriptor for DummyTopology {
        fn node_count(&self) -> usize {
            0
        }
    }

    struct DummyEnv;

    #[async_trait]
    impl Application for DummyEnv {
        type Deployment = DummyTopology;
        type NodeClient = ();
        type NodeConfig = DummyConfig;
        type FeedRuntime = DummyFeedRuntime;

        async fn prepare_feed(
            _node_clients: NodeClients<Self>,
        ) -> Result<(<Self::FeedRuntime as FeedRuntime>::Feed, Self::FeedRuntime), DynError>
        {
            Ok((DummyFeed, DummyFeedRuntime))
        }
    }

    #[async_trait]
    impl LocalDeployerEnv for DummyEnv {
        fn build_node_config(
            _topology: &Self::Deployment,
            _index: usize,
            _peer_ports_by_name: &HashMap<String, u16>,
            _options: &StartNodeOptions<Self>,
            _peer_ports: &[u16],
        ) -> Result<BuiltNodeConfig<<Self as Application>::NodeConfig>, DynError> {
            unreachable!("not used in this test")
        }

        fn build_initial_node_configs(
            _topology: &Self::Deployment,
        ) -> Result<Vec<NodeConfigEntry<<Self as Application>::NodeConfig>>, ProcessSpawnError>
        {
            unreachable!("not used in this test")
        }

        fn build_launch_spec(
            _config: &<Self as Application>::NodeConfig,
            _dir: &Path,
            _label: &str,
        ) -> Result<LaunchSpec, DynError> {
            Ok(LaunchSpec::default())
        }

        fn node_endpoints(
            _config: &<Self as Application>::NodeConfig,
        ) -> Result<NodeEndpoints, DynError> {
            Ok(NodeEndpoints::default())
        }

        fn node_client(_endpoints: &NodeEndpoints) -> Result<Self::NodeClient, DynError> {
            Ok(())
        }

        async fn wait_readiness_stable(_nodes: &[Node<Self>]) -> Result<(), DynError> {
            STABLE_CALLS.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn empty_cluster_still_runs_stability_hook() {
        STABLE_CALLS.store(0, Ordering::SeqCst);
        let nodes: Vec<Node<DummyEnv>> = Vec::new();
        wait_local_http_readiness::<DummyEnv>(&nodes, HttpReadinessRequirement::AllNodesReady)
            .await
            .expect("empty cluster should be considered ready");
        assert_eq!(STABLE_CALLS.load(Ordering::SeqCst), 1);
    }
}

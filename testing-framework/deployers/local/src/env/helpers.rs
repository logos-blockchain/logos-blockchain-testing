use std::{collections::HashMap, path::PathBuf};

use serde::Serialize;
use testing_framework_core::{
    scenario::{
        Application, ClusterNodeConfigApplication, ClusterNodeView, ClusterPeerView, DynError,
        NodeAccess,
    },
    topology::DeploymentDescriptor,
};

use crate::{
    env::LocalBuildContext,
    process::{LaunchSpec, NodeEndpointPort, NodeEndpoints, ProcessSpawnError},
};

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

pub(crate) fn compact_peer_ports(peer_ports: &[u16], self_index: usize) -> Vec<u16> {
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

pub fn build_generated_initial_nodes<E>(
    topology: &E::Deployment,
    node_name_prefix: &str,
    port_names: &[&'static str],
    build_node: impl Fn(LocalBuildContext<'_, E>) -> Result<BuiltNodeConfig<E::NodeConfig>, DynError>,
) -> Result<Vec<NodeConfigEntry<E::NodeConfig>>, ProcessSpawnError>
where
    E: Application,
{
    let reserved_ports =
        reserve_local_node_ports(topology.node_count(), port_names, node_name_prefix)?;
    let peer_ports = reserved_ports
        .iter()
        .map(LocalNodePorts::network_port)
        .collect::<Vec<_>>();
    let peer_ports_by_name = HashMap::new();
    let options = testing_framework_core::scenario::StartNodeOptions::<E>::default();

    reserved_ports
        .iter()
        .enumerate()
        .map(|(index, ports)| {
            let compact_peer_ports = compact_peer_ports(&peer_ports, index);
            let peers = build_local_peer_nodes(&compact_peer_ports, index);
            let built = build_node(LocalBuildContext {
                topology,
                index,
                ports,
                peers: &peers,
                peer_ports: &compact_peer_ports,
                peer_ports_by_name: &peer_ports_by_name,
                options: &options,
                template_config: None,
            })
            .map_err(|source| ProcessSpawnError::Config { source })?;

            Ok(NodeConfigEntry {
                name: format!("{node_name_prefix}-{index}"),
                config: built.config,
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

pub(crate) fn rendered_config_launch_spec(
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

use std::{collections::HashMap, sync::Arc};

use serde::Serialize;
use testing_framework_core::{
    scenario::{
        Application, ClusterNodeConfigApplication, ClusterNodeView, ClusterPeerView, DynError,
        NodeAccess,
    },
    topology::DeploymentDescriptor,
};

use crate::{
    binary::{BinaryProvider, BinaryProviderRef, EnvBinaryProvider, PathBinaryProvider},
    env::LocalBuildContext,
    process::{
        LaunchSpec, NodeEndpointPort, NodeEndpoints, ProcessSpawnError, allocate_available_port,
    },
};

/// Application config and runtime metadata prepared for one local node.
pub struct PreparedNode<Config> {
    /// Default node name, used unless the caller supplies one.
    pub name: String,
    /// Materialized node config value.
    pub config: Config,
    /// Reserved network port used for peer traffic.
    pub network_port: u16,
}

/// Allocated local ports assigned to one node.
pub struct LocalNodePorts {
    network_port: u16,
    named_ports: HashMap<&'static str, u16>,
}

impl LocalNodePorts {
    /// Returns the allocated network port.
    #[must_use]
    pub fn network_port(&self) -> u16 {
        self.network_port
    }

    /// Allocates a named port on first use, returning the same port on later
    /// calls.
    pub fn allocate(&mut self, name: &'static str) -> Result<u16, DynError> {
        if let Some(port) = self.get(name) {
            return Ok(port);
        }
        let port = allocate_available_port()?;
        self.named_ports.insert(name, port);
        Ok(port)
    }

    /// Returns an allocated named port, if present.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<u16> {
        self.named_ports.get(name).copied()
    }

    /// Returns an allocated named port or an error if it is missing.
    pub fn require(&self, name: &str) -> Result<u16, DynError> {
        self.get(name)
            .ok_or_else(|| format!("missing allocated local port '{name}'").into())
    }

    /// Iterates over all allocated named ports.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, u16)> + '_ {
        self.named_ports.iter().map(|(name, port)| (*name, *port))
    }
}

/// Peer node view used while constructing local configs.
#[derive(Clone, Debug)]
pub struct LocalPeerNode {
    index: usize,
    name: Option<String>,
    network_port: u16,
}

impl LocalPeerNode {
    /// Describes a peer whose name is not yet known during initial preparation.
    #[must_use]
    pub fn new(index: usize, network_port: u16) -> Self {
        Self {
            index,
            name: None,
            network_port,
        }
    }

    /// Adds the registered node name for an existing peer.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// The registered name, absent while initial configs are still being built.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Returns the peer's zero-based node index.
    #[must_use]
    pub fn index(&self) -> usize {
        self.index
    }

    /// Returns the peer's allocated network port.
    #[must_use]
    pub fn network_port(&self) -> u16 {
        self.network_port
    }

    /// Returns the peer's loopback HTTP authority as `127.0.0.1:<port>`.
    #[must_use]
    pub fn http_address(&self) -> String {
        format!("127.0.0.1:{}", self.network_port)
    }

    /// Returns the peer authority used in local configs.
    #[must_use]
    pub fn authority(&self) -> String {
        self.http_address()
    }
}

/// How a rendered local config file path is passed to the child process.
#[derive(Clone, Default)]
pub enum LocalConfigArgMode {
    /// Pass the config file as a flag pair, for example `--config config.yaml`.
    #[default]
    Flag,
    /// Pass the config file path as a positional argument.
    Positional,
}

/// Standard local process description for one node binary plus one config file.
#[derive(Clone)]
pub struct LocalProcessSpec {
    /// Binary preparation and resolution policy.
    pub binary: BinaryProviderRef,
    /// Config file name written into the temp launch directory.
    pub config_file_name: String,
    /// CLI flag used to point the process at `config_file_name`.
    pub config_arg: String,
    /// Controls whether `config_arg` is emitted or the file is positional.
    pub config_arg_mode: LocalConfigArgMode,
    /// Extra CLI arguments passed after the config flag.
    pub extra_args: Vec<String>,
    /// Extra environment variables for the child process.
    pub env: Vec<crate::process::LaunchEnvVar>,
}

impl LocalProcessSpec {
    /// Creates a standard binary+config local process spec.
    #[must_use]
    pub fn new(binary_env_var: &str) -> Self {
        Self {
            binary: Arc::new(EnvBinaryProvider::new(binary_env_var)),
            config_file_name: "config.yaml".to_owned(),
            config_arg: "--config".to_owned(),
            config_arg_mode: LocalConfigArgMode::Flag,
            extra_args: Vec::new(),
            env: Vec::new(),
        }
    }

    /// Sets an explicit binary path for this launch.
    #[must_use]
    pub fn with_binary_path(self, path: impl Into<std::path::PathBuf>) -> Self {
        self.with_binary_provider(PathBinaryProvider::new(path))
    }

    /// Overrides the config file name and CLI flag used to pass it.
    #[must_use]
    pub fn with_config_file(mut self, file_name: &str, arg: &str) -> Self {
        self.config_file_name = file_name.to_owned();
        self.config_arg = arg.to_owned();
        self.config_arg_mode = LocalConfigArgMode::Flag;
        self
    }

    /// Overrides the config file name and passes it as a positional argument.
    #[must_use]
    pub fn with_positional_config_file(mut self, file_name: &str) -> Self {
        self.config_file_name = file_name.to_owned();
        self.config_arg_mode = LocalConfigArgMode::Positional;
        self
    }

    /// Appends one extra environment variable.
    #[must_use]
    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.push(crate::process::LaunchEnvVar::new(key, value));
        self
    }

    /// Convenience helper for setting `RUST_LOG`.
    #[must_use]
    pub fn with_rust_log(self, value: &str) -> Self {
        self.with_env("RUST_LOG", value)
    }

    /// Appends extra CLI arguments after the config flag pair.
    #[must_use]
    pub fn with_args(mut self, args: impl IntoIterator<Item = String>) -> Self {
        self.extra_args.extend(args);
        self
    }

    /// Overrides the binary provider used by this process.
    #[must_use]
    pub fn with_binary_provider(mut self, binary: impl BinaryProvider + 'static) -> Self {
        self.binary = Arc::new(binary);
        self
    }

    /// Overrides the binary provider with an already shared provider handle.
    #[must_use]
    pub fn with_binary_provider_ref(mut self, binary: BinaryProviderRef) -> Self {
        self.binary = binary;
        self
    }
}

/// Selects `count` local TCP/UDP ports for later use without holding sockets.
pub fn preallocate_ports(count: usize, label: &str) -> Result<Vec<u16>, ProcessSpawnError> {
    (0..count)
        .map(|_| allocate_available_port())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| ProcessSpawnError::Config {
            source: format!("failed to pre-allocate {label} ports: {source}").into(),
        })
}

/// Allocates network and named ports for `count` local nodes.
/// Ports are checked for TCP and UDP availability, but are not held until
/// launch.
pub fn allocate_local_node_ports(
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

/// Builds the default single-HTTP-endpoint node access shape.
pub fn single_http_node_endpoints(port: u16) -> NodeEndpoints {
    NodeEndpoints::from_api_port(port)
}

/// Builds a cluster node config for the local loopback environment from the
/// shared cluster-config application model.
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

/// Converts discovered local node endpoints into the generic `NodeAccess`
/// shape used by `Application::build_node_client`.
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

/// Builds peer values from a full indexed port list while skipping
/// `self_index`.
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

/// Builds local peer-node views from a full indexed port list while skipping
/// `self_index`.
pub fn build_local_peer_nodes(peer_ports: &[u16], self_index: usize) -> Vec<LocalPeerNode> {
    peer_ports
        .iter()
        .enumerate()
        .filter_map(|(index, port)| {
            (index != self_index).then_some(LocalPeerNode::new(index, *port))
        })
        .collect()
}

/// Generates the initial local node configs for one deployment.
pub fn build_generated_initial_nodes<E>(
    topology: &E::Deployment,
    build_node: impl Fn(LocalBuildContext<'_, E>) -> Result<PreparedNode<E::NodeConfig>, DynError>,
) -> Result<Vec<PreparedNode<E::NodeConfig>>, ProcessSpawnError>
where
    E: Application,
{
    let mut allocated_ports = allocate_local_node_ports(topology.node_count(), &[], "node")?;
    let peer_ports = allocated_ports
        .iter()
        .map(LocalNodePorts::network_port)
        .collect::<Vec<_>>();
    let options = testing_framework_core::scenario::StartNodeOptions::<E>::default();

    allocated_ports
        .iter_mut()
        .enumerate()
        .map(|(index, ports)| {
            let peers = build_local_peer_nodes(&peer_ports, index);
            let built = build_node(LocalBuildContext {
                topology,
                index,
                ports,
                peers: &peers,
                options: &options,
                template_config: None,
            })
            .map_err(|source| ProcessSpawnError::Config { source })?;

            Ok(built)
        })
        .collect()
}

/// Serializes a config as YAML and builds a launch spec for `spec`.
pub async fn yaml_config_launch_spec<T: Serialize>(
    config: &T,
    spec: &LocalProcessSpec,
) -> Result<LaunchSpec, DynError> {
    let config_yaml = serde_yaml::to_string(config)?;
    rendered_config_launch_spec(config_yaml.into_bytes(), spec).await
}

pub async fn build_launch_spec_with_args<E>(
    config: &<E as Application>::NodeConfig,
    dir: &std::path::Path,
    label: &str,
    extra_args: &[String],
) -> Result<LaunchSpec, DynError>
where
    E: crate::env::LocalDeployerEnv,
{
    let mut launch = E::build_launch_spec(config, dir, label).await?;
    launch.args.extend(extra_args.iter().cloned());
    Ok(launch)
}

/// Uses an already rendered text config to build a launch spec for `spec`.
pub async fn text_config_launch_spec(
    rendered_config: impl Into<Vec<u8>>,
    spec: &LocalProcessSpec,
) -> Result<LaunchSpec, DynError> {
    rendered_config_launch_spec(rendered_config.into(), spec).await
}

/// Uses the standard binary+config launch shape for a YAML-rendered config.
pub async fn default_yaml_launch_spec<T: Serialize>(
    config: &T,
    binary_env_var: &str,
    rust_log: &str,
) -> Result<LaunchSpec, DynError> {
    yaml_config_launch_spec(
        config,
        &LocalProcessSpec::new(binary_env_var).with_rust_log(rust_log),
    )
    .await
}

/// Serializes a node config as YAML bytes.
pub fn yaml_node_config<T: Serialize>(config: &T) -> Result<Vec<u8>, DynError> {
    Ok(serde_yaml::to_string(config)?.into_bytes())
}

/// Converts an already rendered text config into launch-file bytes.
pub fn text_node_config(rendered_config: impl Into<Vec<u8>>) -> Vec<u8> {
    rendered_config.into()
}

pub(crate) async fn rendered_config_launch_spec(
    rendered_config: Vec<u8>,
    spec: &LocalProcessSpec,
) -> Result<LaunchSpec, DynError> {
    let binary = spec.binary.resolve().await?;
    let mut args = config_file_args(spec);
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
fn config_file_args(spec: &LocalProcessSpec) -> Vec<String> {
    match spec.config_arg_mode {
        LocalConfigArgMode::Flag => vec![spec.config_arg.clone(), spec.config_file_name.clone()],
        LocalConfigArgMode::Positional => vec![spec.config_file_name.clone()],
    }
}

#[cfg(test)]
mod tests {
    use super::{LocalProcessSpec, text_config_launch_spec};

    #[tokio::test]
    async fn launch_spec_uses_flag_config_by_default() {
        let temp = tempfile::tempdir().expect("temp dir");
        let binary = temp.path().join("app");
        std::fs::write(&binary, b"binary").expect("test binary");
        let spec = LocalProcessSpec::new("APP_BIN").with_binary_path(binary);
        let launch = text_config_launch_spec("config", &spec)
            .await
            .expect("launch spec");

        assert_eq!(launch.args, ["--config", "config.yaml"]);
    }

    #[tokio::test]
    async fn launch_spec_can_use_positional_config_path() {
        let temp = tempfile::tempdir().expect("temp dir");
        let binary = temp.path().join("app");
        std::fs::write(&binary, b"binary").expect("test binary");
        let spec = LocalProcessSpec::new("APP_BIN")
            .with_binary_path(binary)
            .with_positional_config_file("app.json")
            .with_args(["--port".to_owned(), "8080".to_owned()]);
        let launch = text_config_launch_spec("config", &spec)
            .await
            .expect("launch spec");

        assert_eq!(launch.args, ["app.json", "--port", "8080"]);
    }
}

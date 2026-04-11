use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use testing_framework_core::scenario::{
    Application, DynError, HttpReadinessRequirement, ReadinessError, StartNodeOptions,
    wait_for_http_ports_with_requirement,
};

use crate::{
    LaunchSpec, NodeEndpoints,
    process::{ProcessNode, ProcessSpawnError},
};

mod helpers;
#[cfg(test)]
mod tests;

pub use helpers::{
    BuiltNodeConfig, LocalNodePorts, LocalPeerNode, LocalProcessSpec, NodeConfigEntry,
    build_indexed_http_peers, build_indexed_node_configs, build_local_cluster_node_config,
    build_local_peer_nodes, default_yaml_launch_spec, discovered_node_access, preallocate_ports,
    reserve_local_node_ports, single_http_node_endpoints, text_config_launch_spec,
    text_node_config, yaml_config_launch_spec, yaml_node_config,
};

/// Context passed while building a local node config.
pub struct LocalBuildContext<'a, E: Application> {
    pub topology: &'a E::Deployment,
    pub index: usize,
    pub ports: &'a LocalNodePorts,
    pub peers: &'a [LocalPeerNode],
    pub peer_ports: &'a [u16],
    pub peer_ports_by_name: &'a HashMap<String, u16>,
    pub options: &'a StartNodeOptions<E>,
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
        helpers::build_generated_initial_nodes::<Self>(
            topology,
            Self::initial_node_name_prefix(),
            Self::initial_local_port_names(),
            |context| {
                Self::build_node_config_from_template(
                    context.topology,
                    context.index,
                    context.peer_ports_by_name,
                    context.options,
                    context.peer_ports,
                    context.template_config,
                )
            },
        )
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
        helpers::rendered_config_launch_spec(rendered, &spec)
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
    fn initial_node_name_prefix() -> &'static str;

    fn initial_local_port_names() -> &'static [&'static str] {
        &[]
    }

    fn build_local_node_config_with_peers(
        topology: &Self::Deployment,
        index: usize,
        ports: &LocalNodePorts,
        peers: &[LocalPeerNode],
        peer_ports_by_name: &HashMap<String, u16>,
        options: &StartNodeOptions<Self>,
        template_config: Option<&<Self as Application>::NodeConfig>,
    ) -> Result<<Self as Application>::NodeConfig, DynError>;

    fn local_process_spec() -> LocalProcessSpec;

    fn render_local_config(config: &<Self as Application>::NodeConfig)
    -> Result<Vec<u8>, DynError>;

    fn http_api_port(config: &<Self as Application>::NodeConfig) -> u16;

    fn readiness_endpoint_path() -> &'static str {
        <Self as Application>::node_readiness_path()
    }

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

pub(crate) fn readiness_endpoint_path<E: LocalDeployerEnv>() -> &'static str {
    E::readiness_endpoint_path()
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

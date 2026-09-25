use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    time::Duration,
};

use async_trait::async_trait;
use testing_framework_core::scenario::{
    Application, DEFAULT_READINESS_POLL_INTERVAL, DEFAULT_READINESS_TIMEOUT, DynError,
    ReadinessError, ReadinessRequirement, StartNodeOptions, wait_for_readiness_ports,
};

use crate::{
    LaunchSpec, NodeEndpointPort, NodeEndpoints,
    process::{ProcessNode, ProcessSpawnError},
};

mod helpers;
#[cfg(test)]
mod tests;

pub use helpers::{
    LocalConfigArgMode, LocalNodePorts, LocalPeerNode, LocalProcessSpec, PreparedNode,
    build_indexed_http_peers, build_launch_spec_with_args, build_local_cluster_node_config,
    build_local_peer_nodes, default_yaml_launch_spec, discovered_node_access, preallocate_ports,
    reserve_local_node_ports, single_http_node_endpoints, text_config_launch_spec,
    text_node_config, yaml_config_launch_spec, yaml_node_config,
};

/// Context passed while building a local node config.
pub struct LocalBuildContext<'a, E: Application> {
    /// Full deployment topology for the current scenario.
    pub topology: &'a E::Deployment,
    /// Zero-based node index being built.
    pub index: usize,
    /// Reserved local ports assigned to this node.
    pub ports: &'a mut LocalNodePorts,
    /// Peer nodes visible to this node after excluding `index`.
    pub peers: &'a [LocalPeerNode],
    /// Start-time options for the node being built.
    pub options: &'a StartNodeOptions<E>,
    /// Optional existing config to use as a template when starting a node.
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

    /// Builds one node using the allocated resources and optional template.
    /// Applications with preplanned ports return their actual peer port.
    fn build_node_config(
        context: LocalBuildContext<'_, Self>,
    ) -> Result<PreparedNode<Self::NodeConfig>, DynError>;

    /// Builds the initial local configs for every node in the deployment.
    fn build_initial_node_configs(
        topology: &Self::Deployment,
    ) -> Result<Vec<PreparedNode<<Self as Application>::NodeConfig>>, ProcessSpawnError> {
        helpers::build_generated_initial_nodes::<Self>(topology, Self::build_node_config)
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

    /// Builds the executable, files, and arguments for this node.
    async fn build_launch_spec(
        config: &Self::NodeConfig,
        dir: &Path,
        label: &str,
    ) -> Result<LaunchSpec, DynError>;

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

    /// Builds a node client from discovered local endpoints.
    fn node_client(endpoints: &NodeEndpoints) -> Result<Self::NodeClient, DynError> {
        <Self as Application>::build_node_client(&discovered_node_access(endpoints))
    }

    /// Waits for any additional cluster-specific stabilization after the
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
    /// Builds one node using the allocated resources and optional template.
    fn build_node_config(
        context: LocalBuildContext<'_, Self>,
    ) -> Result<PreparedNode<Self::NodeConfig>, DynError>;

    /// Returns the standard process description for launching one local node.
    fn local_process_spec(config: &Self::NodeConfig) -> LocalProcessSpec;

    /// Serializes a local node config into the file bytes written next to the
    /// spawned process.
    fn render_local_config(config: &<Self as Application>::NodeConfig)
    -> Result<Vec<u8>, DynError>;

    /// Returns the main HTTP API port used for discovery and readiness.
    fn http_api_port(config: &<Self as Application>::NodeConfig) -> u16;

    /// Waits for any additional cluster-specific stabilization after the
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
    fn build_node_config(
        context: LocalBuildContext<'_, Self>,
    ) -> Result<PreparedNode<Self::NodeConfig>, DynError> {
        T::build_node_config(context)
    }

    async fn build_launch_spec(
        config: &Self::NodeConfig,
        _dir: &Path,
        _label: &str,
    ) -> Result<LaunchSpec, DynError> {
        let spec = T::local_process_spec(config);
        let rendered = T::render_local_config(config)?;
        helpers::rendered_config_launch_spec(rendered, &spec).await
    }

    fn http_api_port(config: &<Self as Application>::NodeConfig) -> Option<u16> {
        Some(T::http_api_port(config))
    }

    async fn wait_readiness_stable(nodes: &[Node<Self>]) -> Result<(), DynError> {
        T::wait_readiness_stable(nodes).await
    }
}

pub(crate) fn build_node_from_template<E: LocalDeployerEnv>(
    topology: &E::Deployment,
    index: usize,
    options: &StartNodeOptions<E>,
    peers: &[LocalPeerNode],
    template_config: Option<&E::NodeConfig>,
) -> Result<PreparedNode<E::NodeConfig>, DynError> {
    let mut reserved =
        reserve_local_node_ports(1, &[], "node").map_err(|source| -> DynError { source.into() })?;
    let mut ports = reserved
        .pop()
        .ok_or_else(|| std::io::Error::other("failed to reserve local node ports"))?;
    E::build_node_config(LocalBuildContext {
        topology,
        index,
        ports: &mut ports,
        peers,
        options,
        template_config,
    })
}

pub(crate) fn build_initial_node_configs<E: LocalDeployerEnv>(
    topology: &E::Deployment,
) -> Result<Vec<PreparedNode<E::NodeConfig>>, ProcessSpawnError> {
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

/// Waits for local readiness across the provided nodes and then applies
/// any app-specific stabilization hook.
pub async fn wait_local_readiness<E: LocalDeployerEnv>(
    nodes: &[Node<E>],
    requirement: ReadinessRequirement,
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
    requirement: ReadinessRequirement,
) -> Result<(), ReadinessError> {
    wait_local_readiness::<E>(nodes, requirement).await
}

pub(crate) async fn wait_for_local_readiness_ports<E: LocalDeployerEnv>(
    ports: &[u16],
    requirement: ReadinessRequirement,
    timeout: Option<Duration>,
) -> Result<(), ReadinessError> {
    wait_for_readiness_ports(
        ports,
        "127.0.0.1",
        E::node_readiness_probe(),
        requirement,
        timeout.unwrap_or(DEFAULT_READINESS_TIMEOUT),
        DEFAULT_READINESS_POLL_INTERVAL,
    )
    .await
}

/// Spawns a local process node from an already prepared config value.
pub async fn spawn_node_from_config<E: LocalDeployerEnv>(
    prepared: PreparedNode<E::NodeConfig>,
    keep_tempdir: bool,
    persist_dir: Option<&std::path::Path>,
    snapshot_dir: Option<&std::path::Path>,
    extra_args: &[String],
) -> Result<Node<E>, ProcessSpawnError> {
    let extra_args = extra_args.to_vec();
    let PreparedNode {
        name,
        config,
        network_port,
    } = prepared;

    ProcessNode::spawn(
        &name,
        config,
        move |config, dir, label| {
            let extra_args = extra_args.clone();
            Box::pin(async move {
                build_launch_spec_with_args::<E>(config, dir, label, &extra_args).await
            })
        },
        move |config| {
            let mut endpoints = E::node_endpoints(config)?;
            endpoints.insert_port(NodeEndpointPort::Network, network_port);
            Ok(endpoints)
        },
        keep_tempdir,
        persist_dir,
        snapshot_dir,
        E::node_client,
    )
    .await
}

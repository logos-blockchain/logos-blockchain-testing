use testing_framework_core::scenario::{
    Application, DynError, HttpReadinessRequirement, ReadinessError,
    wait_for_http_ports_with_requirement,
};

use crate::{
    LaunchSpec, NodeEndpoints,
    process::{ProcessNode, ProcessSpawnError},
};

mod helpers;
mod runtime;
#[cfg(test)]
mod tests;

pub use helpers::{
    BuiltNodeConfig, LocalNodePorts, LocalPeerNode, LocalProcessSpec, NodeConfigEntry,
    build_indexed_http_peers, build_indexed_node_configs, build_local_cluster_node_config,
    build_local_peer_nodes, default_yaml_launch_spec, discovered_node_access, preallocate_ports,
    reserve_local_node_ports, single_http_node_endpoints, text_config_launch_spec,
    text_node_config, yaml_config_launch_spec, yaml_node_config,
};
pub use runtime::{
    LocalAccess, LocalBuildContext, LocalLifecycle, LocalProcess, LocalRuntime,
    LocalStableReadinessFuture, cluster_node_config_from_context,
};

pub type Node<E> = ProcessNode<<E as Application>::NodeConfig, <E as Application>::NodeClient>;

pub trait LocalDeployerEnv: Application + Sized
where
    <Self as Application>::NodeConfig: Clone + Send + Sync + 'static,
{
    fn local_runtime() -> LocalRuntime<Self>;
}

pub(crate) fn runtime_for<E: LocalDeployerEnv>() -> LocalRuntime<E> {
    E::local_runtime()
}

pub(crate) fn build_node_from_template<E: LocalDeployerEnv>(
    topology: &E::Deployment,
    index: usize,
    peer_ports_by_name: &std::collections::HashMap<String, u16>,
    options: &testing_framework_core::scenario::StartNodeOptions<E>,
    peer_ports: &[u16],
    template_config: Option<&E::NodeConfig>,
) -> Result<BuiltNodeConfig<E::NodeConfig>, DynError> {
    let runtime = runtime_for::<E>();
    let mut reserved = reserve_local_node_ports(1, runtime.process.port_names, "node")
        .map_err(|source| -> DynError { source.into() })?;
    let ports = reserved
        .pop()
        .ok_or_else(|| std::io::Error::other("failed to reserve local node ports"))?;
    let peers = build_local_peer_nodes(peer_ports, index);

    runtime.process.build_node(LocalBuildContext {
        topology,
        index,
        ports: &ports,
        peers: &peers,
        peer_ports,
        peer_ports_by_name,
        options,
        template_config,
    })
}

pub(crate) fn build_initial_node_configs<E: LocalDeployerEnv>(
    topology: &E::Deployment,
) -> Result<Vec<NodeConfigEntry<E::NodeConfig>>, ProcessSpawnError> {
    runtime_for::<E>().process.build_initial_nodes(topology)
}

pub(crate) fn initial_persist_dir<E: LocalDeployerEnv>(
    topology: &E::Deployment,
    node_name: &str,
    index: usize,
) -> Option<std::path::PathBuf> {
    runtime_for::<E>()
        .lifecycle
        .initial_persist_dir(topology, node_name, index)
}

pub(crate) fn initial_snapshot_dir<E: LocalDeployerEnv>(
    topology: &E::Deployment,
    node_name: &str,
    index: usize,
) -> Option<std::path::PathBuf> {
    runtime_for::<E>()
        .lifecycle
        .initial_snapshot_dir(topology, node_name, index)
}

pub(crate) fn build_launch_spec<E: LocalDeployerEnv>(
    config: &E::NodeConfig,
    dir: &std::path::Path,
    label: &str,
) -> Result<LaunchSpec, DynError> {
    runtime_for::<E>()
        .process
        .build_launch_spec(config, dir, label)
}

pub(crate) fn node_endpoints<E: LocalDeployerEnv>(
    config: &E::NodeConfig,
) -> Result<NodeEndpoints, DynError> {
    runtime_for::<E>().access.node_endpoints(config)
}

pub(crate) fn node_client<E: LocalDeployerEnv>(
    endpoints: &NodeEndpoints,
) -> Result<E::NodeClient, DynError> {
    runtime_for::<E>().access.node_client(endpoints)
}

pub(crate) fn node_peer_port<E: LocalDeployerEnv>(node: &Node<E>) -> u16 {
    runtime_for::<E>()
        .access
        .node_peer_port(node.config(), node.endpoints())
}

pub(crate) fn readiness_endpoint_path<E: LocalDeployerEnv>() -> &'static str {
    runtime_for::<E>().access.readiness_path()
}

pub async fn wait_local_http_readiness<E: LocalDeployerEnv>(
    nodes: &[Node<E>],
    requirement: HttpReadinessRequirement,
) -> Result<(), ReadinessError> {
    let ports: Vec<_> = nodes
        .iter()
        .map(|node| node.endpoints().api.port())
        .collect();

    wait_for_http_ports_with_requirement(&ports, readiness_endpoint_path::<E>(), requirement)
        .await?;

    runtime_for::<E>()
        .lifecycle
        .wait_stable(nodes)
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
        build_launch_spec::<E>,
        node_endpoints::<E>,
        keep_tempdir,
        persist_dir,
        snapshot_dir,
        node_client::<E>,
    )
    .await
}

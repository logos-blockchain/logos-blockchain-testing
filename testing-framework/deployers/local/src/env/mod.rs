use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
};

use testing_framework_core::{
    scenario::{
        Application, DynError, HttpReadinessRequirement, ReadinessError, StartNodeOptions,
        wait_for_http_ports_with_requirement,
    },
    topology::DeploymentDescriptor,
};

use crate::process::{LaunchSpec, NodeEndpoints, ProcessNode, ProcessSpawnError};

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
    LocalAccess, LocalBuildContext, LocalProcess, LocalRuntime, cluster_node_config_from_context,
};

pub type Node<E> = ProcessNode<<E as Application>::NodeConfig, <E as Application>::NodeClient>;

#[async_trait::async_trait]
pub trait LocalDeployerEnv: Application + Sized
where
    <Self as Application>::NodeConfig: Clone + Send + Sync + 'static,
{
    fn local_runtime() -> Option<LocalRuntime<Self>> {
        None
    }

    fn local_port_names() -> &'static [&'static str] {
        Self::local_runtime()
            .map(|runtime| runtime.process.port_names)
            .unwrap_or_else(Self::initial_local_port_names)
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
            Self::local_port_names(),
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
        Self::local_runtime()
            .map(|runtime| runtime.process.node_name_prefix)
            .unwrap_or("node")
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
        let compact_peer_ports = helpers::compact_peer_ports(peer_ports, index);
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
        topology: &Self::Deployment,
        index: usize,
        ports: &LocalNodePorts,
        peers: &[LocalPeerNode],
        peer_ports_by_name: &HashMap<String, u16>,
        options: &StartNodeOptions<Self>,
        template_config: Option<&<Self as Application>::NodeConfig>,
    ) -> Result<<Self as Application>::NodeConfig, DynError> {
        if let Some(runtime) = Self::local_runtime() {
            return (runtime.process.build_config)(LocalBuildContext {
                topology,
                index,
                ports,
                peers,
                peer_ports_by_name,
                options,
                template_config,
            });
        }

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
        Self::local_runtime().map(|runtime| runtime.process.spec)
    }

    fn render_local_config(
        config: &<Self as Application>::NodeConfig,
    ) -> Result<Vec<u8>, DynError> {
        if let Some(runtime) = Self::local_runtime() {
            return (runtime.process.render_config)(config);
        }

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

    fn http_api_port(config: &<Self as Application>::NodeConfig) -> Option<u16> {
        Self::local_runtime()
            .and_then(|runtime| runtime.access.api_port.map(|api_port| api_port(config)))
    }

    fn node_endpoints(
        config: &<Self as Application>::NodeConfig,
    ) -> Result<NodeEndpoints, DynError> {
        if let Some(runtime) = Self::local_runtime() {
            return runtime.access.node_endpoints(config);
        }

        if let Some(port) = Self::http_api_port(config) {
            return Ok(NodeEndpoints {
                api: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
                extra_ports: HashMap::new(),
            });
        }

        Err(std::io::Error::other("node_endpoints is not implemented for this app").into())
    }

    fn node_peer_port(node: &Node<Self>) -> u16 {
        if let Some(runtime) = Self::local_runtime() {
            return runtime
                .access
                .node_peer_port(node.config(), node.endpoints());
        }

        node.endpoints().api.port()
    }

    fn node_client_from_api_endpoint(_api: SocketAddr) -> Option<Self::NodeClient> {
        None
    }

    fn node_client(endpoints: &NodeEndpoints) -> Result<Self::NodeClient, DynError> {
        if let Some(runtime) = Self::local_runtime() {
            return runtime.access.node_client(endpoints);
        }

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
        Self::local_runtime()
            .map(|runtime| runtime.access.readiness_path)
            .unwrap_or_else(<Self as Application>::node_readiness_path)
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

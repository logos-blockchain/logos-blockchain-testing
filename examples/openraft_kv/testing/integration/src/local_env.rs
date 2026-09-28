use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use openraft_kv_node::OpenRaftKvNodeConfig;
use testing_framework_core::{scenario::DynError, topology::DeploymentDescriptor};
use testing_framework_runner_local::{
    BinaryProviderRef, BuildBinaryProvider, BuildCommand, EnvBinaryProvider,
    FallbackBinaryProvider, LaunchSpec, LocalBuildContext, LocalDeployerEnv, LocalNodePorts,
    LocalProcessSpec, PreparedNode, allocate_local_node_ports, yaml_config_launch_spec,
};

use crate::OpenRaftKvEnv;

#[async_trait::async_trait]
impl LocalDeployerEnv for OpenRaftKvEnv {
    fn build_node_config(
        context: LocalBuildContext<'_, Self>,
    ) -> Result<PreparedNode<OpenRaftKvNodeConfig>, DynError> {
        let LocalBuildContext {
            index,
            ports,
            peers,
            template_config,
            ..
        } = context;
        let mut config = template_config
            .cloned()
            .unwrap_or_else(|| local_node_config(index, ports.network_port(), BTreeMap::new()));

        // OpenRaft peer config is index-sensitive, so local restarts must rebuild
        // the full peer map from the current reserved port set.
        let network_port = ports.network_port();
        config.node_id = index as u64;
        config.http_port = network_port;
        config.public_addr = local_addr(network_port);
        config.peer_addrs = peers
            .iter()
            .map(|peer| (peer.index() as u64, local_addr(peer.network_port())))
            .collect();

        Ok(PreparedNode {
            name: format!("node-{}", index),
            config,
            network_port,
        })
    }

    fn build_initial_node_configs(
        topology: &Self::Deployment,
    ) -> Result<
        Vec<PreparedNode<OpenRaftKvNodeConfig>>,
        testing_framework_runner_local::process::ProcessSpawnError,
    > {
        let allocated_ports = allocate_local_node_ports(topology.node_count(), &[], "node")?;

        let peer_ports = allocated_ports
            .iter()
            .map(LocalNodePorts::network_port)
            .collect::<Vec<_>>();

        // Build every node from the same reserved port view so the initial
        // cluster starts with a consistent peer list on all nodes.
        Ok(allocated_ports
            .iter()
            .enumerate()
            .map(|(index, ports)| PreparedNode {
                name: format!("node-{index}"),
                network_port: ports.network_port(),
                config: local_node_config(
                    index,
                    ports.network_port(),
                    peer_addrs_from_ports(&peer_ports, index),
                ),
            })
            .collect())
    }

    async fn build_launch_spec(
        config: &OpenRaftKvNodeConfig,
        _dir: &Path,
        _label: &str,
    ) -> Result<LaunchSpec, DynError> {
        let spec = LocalProcessSpec::new("OPENRAFT_KV_NODE_BIN")
            .with_binary_provider(openraft_binary_provider())
            .with_rust_log("info");
        yaml_config_launch_spec(config, &spec).await
    }

    fn http_api_port(config: &OpenRaftKvNodeConfig) -> Option<u16> {
        Some(config.http_port)
    }
}

fn openraft_binary_provider() -> FallbackBinaryProvider {
    let workspace = workspace_root();
    let providers: [BinaryProviderRef; 2] = [
        Arc::new(EnvBinaryProvider::new("OPENRAFT_KV_NODE_BIN")),
        Arc::new(BuildBinaryProvider {
            command: BuildCommand::new("cargo").with_args([
                "build",
                "-p",
                "openraft-kv-node",
                "--bin",
                "openraft-kv-node",
            ]),
            output_path: PathBuf::from(format!(
                "target/debug/openraft-kv-node{}",
                std::env::consts::EXE_SUFFIX
            )),
            working_dir: Some(workspace),
            lock_dir: None,
        }),
    ];

    FallbackBinaryProvider::new(providers)
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..")
}

fn local_node_config(
    index: usize,
    network_port: u16,
    peer_addrs: BTreeMap<u64, String>,
) -> OpenRaftKvNodeConfig {
    OpenRaftKvNodeConfig {
        node_id: index as u64,
        http_port: network_port,
        public_addr: local_addr(network_port),
        peer_addrs,

        heartbeat_interval_ms: 500,
        election_timeout_min_ms: 1_500,
        election_timeout_max_ms: 3_000,
    }
}

fn peer_addrs_from_ports(peer_ports: &[u16], local_index: usize) -> BTreeMap<u64, String> {
    peer_ports
        .iter()
        .enumerate()
        .filter(|(peer_index, _)| *peer_index != local_index)
        .map(|(peer_index, peer_port)| (peer_index as u64, local_addr(*peer_port)))
        .collect()
}

fn local_addr(port: u16) -> String {
    format!("127.0.0.1:{port}")
}

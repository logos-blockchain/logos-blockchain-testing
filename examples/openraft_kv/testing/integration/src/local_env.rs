use std::collections::{BTreeMap, HashMap};

use openraft_kv_node::OpenRaftKvNodeConfig;
use testing_framework_core::{
    scenario::{DynError, StartNodeOptions},
    topology::DeploymentDescriptor,
};
use testing_framework_runner_local::{
    BuiltNodeConfig, LocalDeployerEnv, LocalNodePorts, LocalProcessSpec, NodeConfigEntry,
    reserve_local_node_ports, yaml_node_config,
};

use crate::OpenRaftKvEnv;

impl LocalDeployerEnv for OpenRaftKvEnv {
    fn build_node_config_from_template(
        _topology: &Self::Deployment,
        index: usize,
        _peer_ports_by_name: &HashMap<String, u16>,
        _options: &StartNodeOptions<Self>,
        peer_ports: &[u16],
        template_config: Option<&OpenRaftKvNodeConfig>,
    ) -> Result<BuiltNodeConfig<OpenRaftKvNodeConfig>, DynError> {
        let mut reserved = reserve_local_node_ports(1, &[], "node")
            .map_err(|source| -> DynError { source.into() })?;

        let ports = reserved
            .pop()
            .ok_or_else(|| std::io::Error::other("failed to reserve local node ports"))?;

        let mut config = template_config
            .cloned()
            .unwrap_or_else(|| local_node_config(index, ports.network_port(), BTreeMap::new()));

        // OpenRaft peer config is index-sensitive, so local restarts must rebuild
        // the full peer map from the current reserved port set.
        let network_port = ports.network_port();
        config.node_id = index as u64;
        config.http_port = network_port;
        config.public_addr = local_addr(network_port);
        config.peer_addrs = peer_addrs_from_ports(peer_ports, index);

        Ok(BuiltNodeConfig {
            config,
            network_port,
        })
    }

    fn build_initial_node_configs(
        topology: &Self::Deployment,
    ) -> Result<
        Vec<NodeConfigEntry<OpenRaftKvNodeConfig>>,
        testing_framework_runner_local::process::ProcessSpawnError,
    > {
        let reserved_ports = reserve_local_node_ports(topology.node_count(), &[], "node")?;

        let peer_ports = reserved_ports
            .iter()
            .map(LocalNodePorts::network_port)
            .collect::<Vec<_>>();

        // Build every node from the same reserved port view so the initial
        // cluster starts with a consistent peer list on all nodes.
        Ok(reserved_ports
            .iter()
            .enumerate()
            .map(|(index, ports)| NodeConfigEntry {
                name: format!("node-{index}"),
                config: local_node_config(
                    index,
                    ports.network_port(),
                    peer_addrs_from_ports(&peer_ports, index),
                ),
            })
            .collect())
    }

    fn initial_node_name_prefix() -> &'static str {
        "node"
    }

    fn local_process_spec() -> Option<LocalProcessSpec> {
        Some(
            LocalProcessSpec::new("OPENRAFT_KV_NODE_BIN", "openraft-kv-node").with_rust_log("info"),
        )
    }

    fn render_local_config(config: &OpenRaftKvNodeConfig) -> Result<Vec<u8>, DynError> {
        yaml_node_config(config)
    }

    fn http_api_port(config: &OpenRaftKvNodeConfig) -> Option<u16> {
        Some(config.http_port)
    }
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

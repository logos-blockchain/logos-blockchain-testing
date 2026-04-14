use std::io::Error;

use openraft_kv_node::{OpenRaftKvClient, OpenRaftKvNodeConfig};
use testing_framework_core::scenario::{
    Application, ClusterNodeConfigApplication, ClusterNodeView, ClusterPeerView, DynError,
    NodeAccess, serialize_cluster_yaml_config,
};

/// Three-node topology used by the OpenRaft example scenarios.
pub type OpenRaftKvTopology = testing_framework_core::topology::ClusterTopology;

/// Application environment wiring for the OpenRaft-backed key-value example.
pub struct OpenRaftKvEnv;

impl Application for OpenRaftKvEnv {
    type Deployment = OpenRaftKvTopology;
    type NodeClient = OpenRaftKvClient;
    type NodeConfig = OpenRaftKvNodeConfig;

    fn build_node_client(access: &NodeAccess) -> Result<Self::NodeClient, DynError> {
        Ok(OpenRaftKvClient::new(access.api_base_url()?))
    }

    fn node_readiness_path() -> &'static str {
        "/healthz"
    }
}

impl ClusterNodeConfigApplication for OpenRaftKvEnv {
    type ConfigError = Error;

    fn static_network_port() -> u16 {
        8080
    }

    fn build_cluster_node_config(
        node: &ClusterNodeView,
        peers: &[ClusterPeerView],
    ) -> Result<Self::NodeConfig, Self::ConfigError> {
        Ok(OpenRaftKvNodeConfig {
            node_id: node.index() as u64,
            http_port: node.network_port(),
            public_addr: node.authority(),
            peer_addrs: peers
                .iter()
                .map(|peer| (peer.index() as u64, peer.authority()))
                .collect(),
            heartbeat_interval_ms: 500,
            election_timeout_min_ms: 1_500,
            election_timeout_max_ms: 3_000,
        })
    }

    fn serialize_cluster_node_config(
        config: &Self::NodeConfig,
    ) -> Result<String, Self::ConfigError> {
        serialize_cluster_yaml_config(config).map_err(Error::other)
    }
}

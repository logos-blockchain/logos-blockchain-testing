use std::io::Error;

use async_trait::async_trait;
use pubsub_node::PubSubClient;
use serde::{Deserialize, Serialize};
use testing_framework_core::scenario::{
    Application, ClusterNodeConfigApplication, ClusterNodeView, ClusterPeerView, DynError,
    NodeAccess, serialize_cluster_yaml_config,
};

pub type PubSubTopology = testing_framework_core::topology::ClusterTopology;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PubSubPeerInfo {
    pub node_id: u64,
    pub http_address: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PubSubNodeConfig {
    pub node_id: u64,
    pub http_port: u16,
    pub peers: Vec<PubSubPeerInfo>,
    pub sync_interval_ms: u64,
}

pub struct PubSubEnv;

#[async_trait]
impl Application for PubSubEnv {
    type Deployment = PubSubTopology;
    type NodeClient = PubSubClient;
    type NodeConfig = PubSubNodeConfig;
    fn build_node_client(access: &NodeAccess) -> Result<Self::NodeClient, DynError> {
        Ok(PubSubClient::new(access.api_base_url()?))
    }

    fn node_readiness_path() -> &'static str {
        "/health/ready"
    }
}

impl ClusterNodeConfigApplication for PubSubEnv {
    type ConfigError = Error;

    fn static_network_port() -> u16 {
        8080
    }

    fn build_cluster_node_config(
        node: &ClusterNodeView,
        peers: &[ClusterPeerView],
    ) -> Result<Self::NodeConfig, Self::ConfigError> {
        let peers = peers
            .iter()
            .map(|peer| PubSubPeerInfo {
                node_id: peer.index() as u64,
                http_address: peer.authority(),
            })
            .collect::<Vec<_>>();

        Ok(PubSubNodeConfig {
            node_id: node.index() as u64,
            http_port: node.network_port(),
            peers,
            sync_interval_ms: 500,
        })
    }

    fn serialize_cluster_node_config(
        config: &Self::NodeConfig,
    ) -> Result<String, Self::ConfigError> {
        serialize_cluster_yaml_config(config).map_err(Error::other)
    }
}

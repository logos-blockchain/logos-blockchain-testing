use std::io::Error;

use async_trait::async_trait;
use queue_node::QueueHttpClient;
use serde::{Deserialize, Serialize};
use testing_framework_app::{AppDeployment, AppHostEnv, DeployContext, LocalAppCluster};
use testing_framework_core::scenario::{
    Application, ClusterNodeConfigApplication, ClusterNodeView, ClusterPeerView, DynError,
    NodeAccess, serialize_cluster_yaml_config,
};

pub type QueueTopology = testing_framework_core::topology::ClusterTopology;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueuePeerInfo {
    pub node_id: u64,
    pub http_address: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueueNodeConfig {
    pub node_id: u64,
    pub http_port: u16,
    pub peers: Vec<QueuePeerInfo>,
    pub sync_interval_ms: u64,
}

pub struct QueueEnv;

#[async_trait]
impl Application for QueueEnv {
    type Deployment = QueueTopology;
    type NodeClient = QueueHttpClient;
    type NodeConfig = QueueNodeConfig;
    fn build_node_client(access: &NodeAccess) -> Result<Self::NodeClient, DynError> {
        Ok(QueueHttpClient::new(access.api_base_url()?))
    }

    fn node_readiness_path() -> &'static str {
        "/health/ready"
    }
}

#[derive(Clone)]
pub struct QueueLocalApp {
    deployment: QueueTopology,
}

impl QueueLocalApp {
    #[must_use]
    pub fn nodes(nodes: usize) -> Self {
        Self {
            deployment: QueueTopology::new(nodes),
        }
    }

    #[must_use]
    pub fn deployment(&self) -> QueueTopology {
        self.deployment.clone()
    }
}

#[async_trait]
impl AppDeployment<AppHostEnv> for QueueLocalApp {
    type Handle = LocalAppCluster<QueueEnv>;

    async fn deploy(self, ctx: &mut DeployContext<AppHostEnv>) -> Result<Self::Handle, DynError> {
        ctx.deploy_local_cluster::<QueueEnv>(self.deployment).await
    }
}

impl ClusterNodeConfigApplication for QueueEnv {
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
            .map(|peer| QueuePeerInfo {
                node_id: peer.index() as u64,
                http_address: peer.authority(),
            })
            .collect::<Vec<_>>();

        Ok(QueueNodeConfig {
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

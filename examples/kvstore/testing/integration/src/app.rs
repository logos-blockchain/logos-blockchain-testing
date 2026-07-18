use std::io::Error;

use async_trait::async_trait;
use kvstore_node::KvHttpClient;
use serde::{Deserialize, Serialize};
use testing_framework_app::{AppDeployment, AppHostEnv, DeployContext, LocalAppCluster};
use testing_framework_core::{
    scenario::{
        Application, ClusterNodeConfigApplication, ClusterNodeView, ClusterPeerView, DynError,
        NodeAccess, NodeClients, serialize_cluster_yaml_config,
    },
    topology::DeploymentDescriptor,
};

pub type KvTopology = testing_framework_core::topology::ClusterTopology;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KvPeerInfo {
    pub node_id: u64,
    pub http_address: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KvNodeConfig {
    pub node_id: u64,
    pub http_port: u16,
    pub peers: Vec<KvPeerInfo>,
    pub sync_interval_ms: u64,
}

pub struct KvEnv;

#[async_trait]
impl Application for KvEnv {
    type Deployment = KvTopology;
    type NodeClient = KvHttpClient;
    type NodeConfig = KvNodeConfig;
    fn build_node_client(access: &NodeAccess) -> Result<Self::NodeClient, DynError> {
        Ok(KvHttpClient::new(access.api_base_url()?))
    }

    fn node_readiness_path() -> &'static str {
        "/health/ready"
    }
}

#[derive(Clone)]
pub struct KvStoreCluster {
    deployment: KvTopology,
    node_clients: NodeClients<KvEnv>,
}

impl KvStoreCluster {
    #[must_use]
    pub const fn new(deployment: KvTopology, node_clients: NodeClients<KvEnv>) -> Self {
        Self {
            deployment,
            node_clients,
        }
    }

    #[must_use]
    pub fn topology(&self) -> &KvTopology {
        &self.deployment
    }

    #[must_use]
    pub fn node_count(&self) -> usize {
        self.deployment.node_count()
    }

    #[must_use]
    pub fn clients(&self) -> Vec<KvHttpClient> {
        self.node_clients.snapshot()
    }

    #[must_use]
    pub fn first_client(&self) -> Option<KvHttpClient> {
        self.node_clients
            .with_clients(|clients| clients.first().cloned())
    }
}

#[derive(Clone)]
pub struct KvExistingClusterApp {
    topology: KvTopology,
}

impl KvExistingClusterApp {
    #[must_use]
    pub fn nodes(nodes: usize) -> Self {
        Self {
            topology: KvTopology::new(nodes),
        }
    }

    #[must_use]
    pub fn topology(&self) -> KvTopology {
        self.topology.clone()
    }
}

#[async_trait]
impl AppDeployment<KvEnv> for KvExistingClusterApp {
    type Handle = KvStoreCluster;

    async fn deploy(self, ctx: &mut DeployContext<KvEnv>) -> Result<Self::Handle, DynError> {
        Ok(KvStoreCluster::new(
            ctx.deployment().clone(),
            ctx.node_clients().clone(),
        ))
    }
}

#[derive(Clone)]
pub struct KvLocalApp {
    deployment: KvTopology,
}

impl KvLocalApp {
    #[must_use]
    pub fn nodes(nodes: usize) -> Self {
        Self {
            deployment: KvTopology::new(nodes),
        }
    }

    #[must_use]
    pub fn deployment(&self) -> KvTopology {
        self.deployment.clone()
    }
}

#[async_trait]
impl AppDeployment<AppHostEnv> for KvLocalApp {
    type Handle = LocalAppCluster<KvEnv>;

    async fn deploy(self, ctx: &mut DeployContext<AppHostEnv>) -> Result<Self::Handle, DynError> {
        ctx.deploy_local_cluster::<KvEnv>(self.deployment).await
    }
}

impl ClusterNodeConfigApplication for KvEnv {
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
            .map(|peer| KvPeerInfo {
                node_id: peer.index() as u64,
                http_address: peer.authority(),
            })
            .collect::<Vec<_>>();

        Ok(KvNodeConfig {
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

use std::io::Error;

use async_trait::async_trait;
use openraft_kv_node::{OpenRaftKvClient, OpenRaftKvNodeConfig};
use testing_framework_app::{AppDeployment, AppHostEnv, DeployContext, LocalAppCluster};
use testing_framework_core::{
    scenario::{
        Application, ClusterNodeConfigApplication, ClusterNodeView, ClusterPeerView, DynError,
        NodeAccess, NodeClients, serialize_cluster_yaml_config,
    },
    topology::DeploymentDescriptor,
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

/// Runtime handle exposed by the OpenRaft app deployment.
#[derive(Clone)]
pub struct OpenRaftKvCluster {
    deployment: OpenRaftKvTopology,
    node_clients: NodeClients<OpenRaftKvEnv>,
}

impl OpenRaftKvCluster {
    #[must_use]
    pub const fn new(
        deployment: OpenRaftKvTopology,
        node_clients: NodeClients<OpenRaftKvEnv>,
    ) -> Self {
        Self {
            deployment,
            node_clients,
        }
    }

    #[must_use]
    pub fn topology(&self) -> &OpenRaftKvTopology {
        &self.deployment
    }

    #[must_use]
    pub fn node_count(&self) -> usize {
        self.deployment.node_count()
    }

    #[must_use]
    pub fn clients(&self) -> Vec<OpenRaftKvClient> {
        self.node_clients.snapshot()
    }

    pub async fn states(&self) -> Result<Vec<openraft_kv_node::OpenRaftKvState>, DynError> {
        let clients = self.clients();
        let mut states = Vec::with_capacity(clients.len());

        for client in clients {
            states.push(client.state().await?);
        }

        states.sort_by_key(|state| state.node_id);

        Ok(states)
    }
}

/// App preset for the OpenRaft key-value example.
#[derive(Clone)]
pub struct OpenRaftKvExistingClusterApp {
    topology: OpenRaftKvTopology,
}

impl OpenRaftKvExistingClusterApp {
    #[must_use]
    pub fn nodes(nodes: usize) -> Self {
        Self {
            topology: OpenRaftKvTopology::new(nodes),
        }
    }

    #[must_use]
    pub fn topology(&self) -> OpenRaftKvTopology {
        self.topology.clone()
    }
}

#[async_trait]
impl AppDeployment<OpenRaftKvEnv> for OpenRaftKvExistingClusterApp {
    type Handle = OpenRaftKvCluster;

    async fn deploy(
        self,
        ctx: &mut DeployContext<OpenRaftKvEnv>,
    ) -> Result<Self::Handle, DynError> {
        Ok(OpenRaftKvCluster::new(
            ctx.deployment().clone(),
            ctx.node_clients().clone(),
        ))
    }
}

/// Local app preset that starts its own OpenRaft cluster.
#[derive(Clone)]
pub struct OpenRaftKvLocalApp {
    deployment: OpenRaftKvTopology,
}

impl OpenRaftKvLocalApp {
    #[must_use]
    pub fn nodes(nodes: usize) -> Self {
        Self {
            deployment: OpenRaftKvTopology::new(nodes),
        }
    }

    #[must_use]
    pub fn deployment(&self) -> OpenRaftKvTopology {
        self.deployment.clone()
    }
}

#[async_trait]
impl AppDeployment<AppHostEnv> for OpenRaftKvLocalApp {
    type Handle = LocalAppCluster<OpenRaftKvEnv>;

    async fn deploy(self, ctx: &mut DeployContext<AppHostEnv>) -> Result<Self::Handle, DynError> {
        ctx.deploy_local_cluster::<OpenRaftKvEnv>(self.deployment)
            .await
    }
}

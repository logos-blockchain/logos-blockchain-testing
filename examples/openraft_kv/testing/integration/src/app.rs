use std::io::Error;

use async_trait::async_trait;
use openraft_kv_node::{OpenRaftKvClient, OpenRaftKvNodeConfig};
use testing_framework_app::{AppDeployment, AppHostEnv, ClusterApp, DeployContext};
use testing_framework_core::{
    observation::ObservationRuntime,
    scenario::{
        Application, CleanupGuard, ClusterHandle, ClusterNodeConfigApplication, ClusterNodeView,
        ClusterPeerView, ClusterProvisioner, DynError, NodeAccess, ReadinessProbe,
        serialize_cluster_yaml_config,
    },
};
use tokio::task::JoinHandle;

use crate::{OpenRaftClusterObserver, OpenRaftNodeClientsSourceProvider};

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

    fn node_readiness_probe() -> ReadinessProbe {
        ReadinessProbe::Http { path: "/healthz" }
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

/// App preset that deploys an OpenRaft cluster together with its cluster
/// observer.
///
/// The cluster is exposed as [`ClusterHandle<OpenRaftKvEnv>`] and the observer
/// as `ObservationHandle<OpenRaftClusterObserver>`, both retrievable through
/// `AppRunContextExt::require_app`.
#[derive(Clone)]
pub struct OpenRaftKvClusterApp {
    topology: OpenRaftKvTopology,
}

impl OpenRaftKvClusterApp {
    /// Creates the composed app for a cluster of the given size.
    #[must_use]
    pub fn nodes(nodes: usize) -> Self {
        Self {
            topology: OpenRaftKvTopology::new(nodes),
        }
    }

    /// Returns the requested cluster topology.
    #[must_use]
    pub fn topology(&self) -> OpenRaftKvTopology {
        self.topology.clone()
    }
}

#[async_trait]
impl<P> AppDeployment<AppHostEnv, P> for OpenRaftKvClusterApp
where
    P: ClusterProvisioner<OpenRaftKvEnv>,
{
    type Handle = ClusterHandle<OpenRaftKvEnv>;

    async fn deploy(
        self,
        ctx: &mut DeployContext<AppHostEnv, P>,
    ) -> Result<Self::Handle, DynError> {
        let cluster = ctx
            .deploy(ClusterApp::<OpenRaftKvEnv>::new(self.topology))
            .await?;

        ctx.expose(cluster.clone())?;

        let provider = OpenRaftNodeClientsSourceProvider::new(cluster.node_clients());
        let runtime = ObservationRuntime::start(
            provider,
            OpenRaftClusterObserver,
            OpenRaftClusterObserver::config(),
        )
        .await?;
        let (observer, task) = runtime.into_parts();

        ctx.defer_cleanup(Box::new(ObserverTaskGuard { task }));
        ctx.expose(observer)?;

        Ok(cluster)
    }
}

struct ObserverTaskGuard {
    task: JoinHandle<()>,
}

impl CleanupGuard for ObserverTaskGuard {
    fn cleanup(self: Box<Self>) {
        self.task.abort();
    }
}

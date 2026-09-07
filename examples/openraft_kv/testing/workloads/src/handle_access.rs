use async_trait::async_trait;
use openraft_kv_runtime_ext::OpenRaftKvEnv;
use testing_framework_app::{AppHostEnv, AppRunContextExt as _};
use testing_framework_core::scenario::{ClusterHandle, DynError, RunContext, Workload};
use tracing::info;

#[derive(Clone)]
pub struct OpenRaftKvClusterAccessible {
    expected_nodes: usize,
}

impl OpenRaftKvClusterAccessible {
    #[must_use]
    pub const fn new(expected_nodes: usize) -> Self {
        Self { expected_nodes }
    }
}

#[async_trait]
impl Workload<AppHostEnv> for OpenRaftKvClusterAccessible {
    fn name(&self) -> &str {
        "openraft_kv_cluster_accessible"
    }

    async fn start(&self, ctx: &RunContext<AppHostEnv>) -> Result<(), DynError> {
        let cluster = ctx.require_app::<ClusterHandle<OpenRaftKvEnv>>()?;
        let clients = cluster.clients();

        if cluster.node_count() != self.expected_nodes {
            return Err(format!(
                "openraft app topology has {} nodes, expected {}",
                cluster.node_count(),
                self.expected_nodes
            )
            .into());
        }

        if clients.len() != self.expected_nodes {
            return Err(format!(
                "openraft app handle has {} clients, expected {}",
                clients.len(),
                self.expected_nodes
            )
            .into());
        }

        let mut states = Vec::with_capacity(clients.len());
        for client in &clients {
            states.push(client.state().await?);
        }

        if states.len() != self.expected_nodes {
            return Err(format!(
                "openraft app handle read {} node states, expected {}",
                states.len(),
                self.expected_nodes
            )
            .into());
        }

        info!(
            nodes = self.expected_nodes,
            "openraft app handle is accessible from workload"
        );

        Ok(())
    }
}

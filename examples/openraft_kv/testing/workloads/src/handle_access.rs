use async_trait::async_trait;
use openraft_kv_runtime_ext::{OpenRaftKvCluster, OpenRaftKvEnv};
use testing_framework_app::AppRunContextExt;
use testing_framework_core::scenario::{DynError, RunContext, Workload};
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
impl Workload<OpenRaftKvEnv> for OpenRaftKvClusterAccessible {
    fn name(&self) -> &str {
        "openraft_kv_cluster_accessible"
    }

    async fn start(&self, ctx: &RunContext<OpenRaftKvEnv>) -> Result<(), DynError> {
        let cluster = ctx.require_app::<OpenRaftKvCluster>()?;
        let states = cluster.states().await?;
        let client_count = cluster.clients().len();

        if cluster.node_count() != self.expected_nodes {
            return Err(format!(
                "openraft app topology has {} nodes, expected {}",
                cluster.node_count(),
                self.expected_nodes
            )
            .into());
        }

        if client_count != self.expected_nodes {
            return Err(format!(
                "openraft app handle has {client_count} clients, expected {}",
                self.expected_nodes
            )
            .into());
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

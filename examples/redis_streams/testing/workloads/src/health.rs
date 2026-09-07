use std::time::Duration;

use async_trait::async_trait;
use redis_streams_runtime_ext::{RedisStreamsClient, RedisStreamsEnv};
use testing_framework_app::{AppHostEnv, AppRunContextExt as _};
use testing_framework_core::scenario::{ClusterHandle, DynError, Expectation, RunContext};
use tokio::time::Instant;
use tracing::info;

#[derive(Clone)]
pub struct RedisStreamsClusterHealthy {
    timeout: Duration,
    poll_interval: Duration,
}

impl RedisStreamsClusterHealthy {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            timeout: Duration::from_secs(20),
            poll_interval: Duration::from_millis(500),
        }
    }

    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl Default for RedisStreamsClusterHealthy {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Expectation<AppHostEnv> for RedisStreamsClusterHealthy {
    fn name(&self) -> &str {
        "redis_streams_cluster_healthy"
    }

    async fn evaluate(&mut self, ctx: &RunContext<AppHostEnv>) -> Result<(), DynError> {
        let cluster = ctx.require_app::<ClusterHandle<RedisStreamsEnv>>()?;
        if cluster.node_count() == 0 {
            return Err("redis streams cluster has no nodes".into());
        }

        let clients = cluster.clients();
        if clients.is_empty() {
            return Err("no redis streams node clients available".into());
        }

        let deadline = Instant::now() + self.timeout;
        while Instant::now() < deadline {
            if all_nodes_healthy(&clients).await? {
                info!(nodes = clients.len(), "redis streams cluster healthy");
                return Ok(());
            }

            tokio::time::sleep(self.poll_interval).await;
        }

        Err(format!(
            "redis streams cluster not healthy within {:?}",
            self.timeout
        )
        .into())
    }
}

async fn all_nodes_healthy(clients: &[RedisStreamsClient]) -> Result<bool, DynError> {
    for client in clients {
        if client.ping().await.is_err() {
            return Ok(false);
        }
    }
    Ok(true)
}

use std::time::Duration;

use async_trait::async_trait;
use nats_runtime_ext::{NatsClient, NatsEnv};
use testing_framework_core::scenario::{DynError, Expectation, RunContext};
use tokio::time::Instant;
use tracing::info;

#[derive(Clone)]
pub struct NatsClusterHealthy {
    timeout: Duration,
    poll_interval: Duration,
}

impl NatsClusterHealthy {
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

impl Default for NatsClusterHealthy {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Expectation<NatsEnv> for NatsClusterHealthy {
    fn name(&self) -> &str {
        "nats_cluster_healthy"
    }

    async fn evaluate(&mut self, ctx: &RunContext<NatsEnv>) -> Result<(), DynError> {
        let clients = ctx.node_clients().snapshot();
        if clients.is_empty() {
            return Err("no nats node clients available".into());
        }

        let deadline = Instant::now() + self.timeout;
        while Instant::now() < deadline {
            if all_nodes_healthy(&clients).await? {
                info!(nodes = clients.len(), "nats cluster healthy");
                return Ok(());
            }

            tokio::time::sleep(self.poll_interval).await;
        }

        Err(format!("nats cluster not healthy within {:?}", self.timeout).into())
    }
}

async fn all_nodes_healthy(clients: &[NatsClient]) -> Result<bool, DynError> {
    for client in clients {
        if !client.is_healthy().await? {
            return Ok(false);
        }
    }
    Ok(true)
}

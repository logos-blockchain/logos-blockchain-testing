use std::time::Duration;

use async_trait::async_trait;
use queue_runtime_ext::QueueEnv;
use serde::Deserialize;
use testing_framework_core::scenario::{DynError, Expectation, RunContext};
use tracing::info;

#[derive(Clone)]
pub struct QueueConverges {
    min_queue_len: usize,
    timeout: Duration,
    poll_interval: Duration,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct QueueRevision {
    version: u64,
    origin: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct QueueStateResponse {
    revision: QueueRevision,
    queue_len: usize,
    head_id: Option<u64>,
    tail_id: Option<u64>,
}

impl QueueConverges {
    #[must_use]
    pub fn new(min_queue_len: usize) -> Self {
        Self {
            min_queue_len,
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

#[async_trait]
impl Expectation<QueueEnv> for QueueConverges {
    fn name(&self) -> &str {
        "queue_converges"
    }

    async fn evaluate(&mut self, ctx: &RunContext<QueueEnv>) -> Result<(), DynError> {
        let clients = ctx.node_clients().snapshot();
        if clients.is_empty() {
            return Err("no queue node clients available".into());
        }

        let deadline = tokio::time::Instant::now() + self.timeout;
        while tokio::time::Instant::now() < deadline {
            if self.is_converged(&clients).await? {
                info!(
                    min_queue_len = self.min_queue_len,
                    "queue convergence reached"
                );
                return Ok(());
            }
            tokio::time::sleep(self.poll_interval).await;
        }

        Err(format!(
            "queue convergence not reached within {:?} (min_queue_len={})",
            self.timeout, self.min_queue_len
        )
        .into())
    }
}

impl QueueConverges {
    async fn is_converged(
        &self,
        clients: &[queue_node::QueueHttpClient],
    ) -> Result<bool, DynError> {
        let Some((first, rest)) = clients.split_first() else {
            return Ok(false);
        };

        let baseline = read_state(first).await?;
        if baseline.queue_len < self.min_queue_len {
            return Ok(false);
        }

        for client in rest {
            let current = read_state(client).await?;
            if current != baseline {
                return Ok(false);
            }
        }

        Ok(true)
    }
}

async fn read_state(client: &queue_node::QueueHttpClient) -> Result<QueueStateResponse, DynError> {
    Ok(client.get("/queue/state").await?)
}

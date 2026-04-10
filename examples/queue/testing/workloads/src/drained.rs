use std::time::Duration;

use async_trait::async_trait;
use queue_runtime_ext::QueueEnv;
use serde::Deserialize;
use testing_framework_core::scenario::{DynError, Expectation, RunContext};
use tracing::info;

#[derive(Clone)]
pub struct QueueDrained {
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

impl QueueDrained {
    #[must_use]
    pub fn new() -> Self {
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

impl Default for QueueDrained {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Expectation<QueueEnv> for QueueDrained {
    fn name(&self) -> &str {
        "queue_drained"
    }

    async fn evaluate(&mut self, ctx: &RunContext<QueueEnv>) -> Result<(), DynError> {
        let clients = ctx.node_clients().snapshot();
        if clients.is_empty() {
            return Err("no queue node clients available".into());
        }

        let deadline = tokio::time::Instant::now() + self.timeout;
        while tokio::time::Instant::now() < deadline {
            if is_drained_and_converged(&clients).await? {
                info!("queue drained and converged");
                return Ok(());
            }
            tokio::time::sleep(self.poll_interval).await;
        }

        Err(format!("queue not drained within {:?}", self.timeout).into())
    }
}

async fn is_drained_and_converged(
    clients: &[queue_node::QueueHttpClient],
) -> Result<bool, DynError> {
    let Some((first, rest)) = clients.split_first() else {
        return Ok(false);
    };

    let baseline = read_state(first).await?;
    if !is_drained(&baseline) {
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

fn is_drained(state: &QueueStateResponse) -> bool {
    state.queue_len == 0 && state.head_id.is_none() && state.tail_id.is_none()
}

async fn read_state(client: &queue_node::QueueHttpClient) -> Result<QueueStateResponse, DynError> {
    Ok(client.get("/queue/state").await?)
}

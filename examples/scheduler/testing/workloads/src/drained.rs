use std::time::Duration;

use async_trait::async_trait;
use scheduler_runtime_ext::SchedulerEnv;
use serde::Deserialize;
use testing_framework_core::scenario::{DynError, Expectation, RunContext};
use tracing::info;

#[derive(Clone)]
pub struct SchedulerDrained {
    min_done: usize,
    timeout: Duration,
    poll_interval: Duration,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct Revision {
    version: u64,
    origin: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct StateResponse {
    revision: Revision,
    pending: usize,
    leased: usize,
    done: usize,
}

impl SchedulerDrained {
    #[must_use]
    pub fn new(min_done: usize) -> Self {
        Self {
            min_done,
            timeout: Duration::from_secs(30),
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
impl Expectation<SchedulerEnv> for SchedulerDrained {
    fn name(&self) -> &str {
        "scheduler_drained"
    }

    async fn evaluate(&mut self, ctx: &RunContext<SchedulerEnv>) -> Result<(), DynError> {
        let clients = ctx.node_clients().snapshot();
        if clients.is_empty() {
            return Err("no scheduler node clients available".into());
        }

        let deadline = tokio::time::Instant::now() + self.timeout;
        while tokio::time::Instant::now() < deadline {
            if is_drained_and_converged(&clients, self.min_done).await? {
                info!(min_done = self.min_done, "scheduler drained and converged");
                return Ok(());
            }
            tokio::time::sleep(self.poll_interval).await;
        }

        Err(format!("scheduler not drained within {:?}", self.timeout).into())
    }
}

async fn is_drained_and_converged(
    clients: &[scheduler_node::SchedulerHttpClient],
    min_done: usize,
) -> Result<bool, DynError> {
    let Some((first, rest)) = clients.split_first() else {
        return Ok(false);
    };

    let baseline = read_state(first).await?;
    if baseline.pending != 0 || baseline.leased != 0 || baseline.done < min_done {
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

async fn read_state(
    client: &scheduler_node::SchedulerHttpClient,
) -> Result<StateResponse, DynError> {
    Ok(client.get("/jobs/state").await?)
}

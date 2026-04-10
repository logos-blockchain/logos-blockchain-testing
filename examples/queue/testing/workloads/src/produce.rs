use std::time::Duration;

use async_trait::async_trait;
use queue_runtime_ext::QueueEnv;
use serde::{Deserialize, Serialize};
use testing_framework_core::scenario::{DynError, RunContext, Workload};
use tracing::info;

#[derive(Clone)]
pub struct QueueProduceWorkload {
    operations: usize,
    rate_per_sec: Option<usize>,
    payload_prefix: String,
}

#[derive(Serialize)]
struct EnqueueRequest {
    payload: String,
}

#[derive(Deserialize)]
struct EnqueueResponse {
    accepted: bool,
    id: u64,
    queue_len: usize,
}

impl QueueProduceWorkload {
    #[must_use]
    pub fn new() -> Self {
        Self {
            operations: 200,
            rate_per_sec: Some(25),
            payload_prefix: "queue-demo".to_owned(),
        }
    }

    #[must_use]
    pub const fn operations(mut self, value: usize) -> Self {
        self.operations = value;
        self
    }

    #[must_use]
    pub const fn rate_per_sec(mut self, value: usize) -> Self {
        self.rate_per_sec = Some(value);
        self
    }

    #[must_use]
    pub fn payload_prefix(mut self, value: impl Into<String>) -> Self {
        self.payload_prefix = value.into();
        self
    }
}

impl Default for QueueProduceWorkload {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Workload<QueueEnv> for QueueProduceWorkload {
    fn name(&self) -> &str {
        "queue_produce_workload"
    }

    async fn start(&self, ctx: &RunContext<QueueEnv>) -> Result<(), DynError> {
        let clients = ctx.node_clients().snapshot();
        let Some(producer) = clients.first() else {
            return Err("no queue node clients available".into());
        };

        let interval = self.rate_per_sec.and_then(compute_interval);
        info!(
            operations = self.operations,
            rate_per_sec = ?self.rate_per_sec,
            "starting queue produce workload"
        );

        for idx in 0..self.operations {
            let payload = format!("{}-{idx}", self.payload_prefix);
            let response: EnqueueResponse = producer
                .post("/queue/enqueue", &EnqueueRequest { payload })
                .await?;

            if !response.accepted {
                return Err(format!("node rejected enqueue at operation {idx}").into());
            }

            if (idx + 1) % 25 == 0 {
                info!(
                    completed = idx + 1,
                    last_id = response.id,
                    queue_len = response.queue_len,
                    "queue produce progress"
                );
            }

            if let Some(delay) = interval {
                tokio::time::sleep(delay).await;
            }
        }

        Ok(())
    }
}

fn compute_interval(rate_per_sec: usize) -> Option<Duration> {
    if rate_per_sec == 0 {
        return None;
    }

    Some(Duration::from_millis((1000 / rate_per_sec as u64).max(1)))
}

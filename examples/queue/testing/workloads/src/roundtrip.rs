use std::{collections::HashSet, time::Duration};

use async_trait::async_trait;
use queue_runtime_ext::QueueEnv;
use serde::{Deserialize, Serialize};
use testing_framework_core::scenario::{DynError, RunContext, Workload};
use tokio::time::{Instant, sleep};
use tracing::info;

#[derive(Clone)]
pub struct QueueRoundTripWorkload {
    operations: usize,
    rate_per_sec: Option<usize>,
    payload_prefix: String,
    drain_timeout: Duration,
    empty_poll_interval: Duration,
}

#[derive(Serialize)]
struct EnqueueRequest {
    payload: String,
}

#[derive(Deserialize)]
struct EnqueueResponse {
    accepted: bool,
    id: u64,
}

#[derive(Serialize)]
struct DequeueRequest {}

#[derive(Deserialize)]
struct QueueMessage {
    id: u64,
    payload: String,
}

#[derive(Deserialize)]
struct DequeueResponse {
    message: Option<QueueMessage>,
}

impl QueueRoundTripWorkload {
    #[must_use]
    pub fn new() -> Self {
        Self {
            operations: 200,
            rate_per_sec: Some(25),
            payload_prefix: "queue-roundtrip".to_owned(),
            drain_timeout: Duration::from_secs(20),
            empty_poll_interval: Duration::from_millis(100),
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

    #[must_use]
    pub const fn drain_timeout(mut self, value: Duration) -> Self {
        self.drain_timeout = value;
        self
    }
}

impl Default for QueueRoundTripWorkload {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Workload<QueueEnv> for QueueRoundTripWorkload {
    fn name(&self) -> &str {
        "queue_roundtrip_workload"
    }

    async fn start(&self, ctx: &RunContext<QueueEnv>) -> Result<(), DynError> {
        let clients = ctx.node_clients().snapshot();
        let Some(driver) = clients.first() else {
            return Err("no queue node clients available".into());
        };

        let interval = self.rate_per_sec.and_then(compute_interval);
        let mut produced_ids = HashSet::with_capacity(self.operations);

        info!(
            operations = self.operations,
            "queue roundtrip: produce phase"
        );
        for idx in 0..self.operations {
            let payload = format!("{}-{idx}", self.payload_prefix);
            let response: EnqueueResponse = driver
                .post("/queue/enqueue", &EnqueueRequest { payload })
                .await?;

            if !response.accepted {
                return Err(format!("enqueue rejected at operation {idx}").into());
            }

            if !produced_ids.insert(response.id) {
                return Err(format!("duplicate enqueue id observed: {}", response.id).into());
            }

            if let Some(delay) = interval {
                sleep(delay).await;
            }
        }

        info!(
            operations = self.operations,
            "queue roundtrip: consume phase"
        );
        let mut consumed = 0usize;
        let deadline = Instant::now() + self.drain_timeout;

        while consumed < self.operations && Instant::now() < deadline {
            let response: DequeueResponse =
                driver.post("/queue/dequeue", &DequeueRequest {}).await?;

            match response.message {
                Some(message) => {
                    if !message.payload.starts_with(&self.payload_prefix) {
                        return Err(format!("unexpected payload: {}", message.payload).into());
                    }
                    if !produced_ids.remove(&message.id) {
                        return Err(
                            format!("unknown or duplicate dequeue id: {}", message.id).into()
                        );
                    }
                    consumed += 1;
                }
                None => sleep(self.empty_poll_interval).await,
            }
        }

        if consumed != self.operations {
            return Err(format!(
                "queue roundtrip timed out: consumed {consumed}/{} messages",
                self.operations
            )
            .into());
        }

        if !produced_ids.is_empty() {
            return Err(format!(
                "queue roundtrip ended with {} undrained produced ids",
                produced_ids.len()
            )
            .into());
        }

        info!(operations = self.operations, "queue roundtrip finished");
        Ok(())
    }
}

fn compute_interval(rate_per_sec: usize) -> Option<Duration> {
    if rate_per_sec == 0 {
        return None;
    }

    Some(Duration::from_millis((1000 / rate_per_sec as u64).max(1)))
}

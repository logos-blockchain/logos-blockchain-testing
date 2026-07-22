use std::time::Duration;

use async_trait::async_trait;
use queue_node::QueueHttpClient;
use queue_runtime_ext::QueueEnv;
use serde::{Deserialize, Serialize};
use testing_framework_core::scenario::{DynError, RunContext, Workload};
use tracing::{info, warn};

const REQUEST_RETRY_INTERVAL: Duration = Duration::from_millis(250);
const REQUEST_RETRY_WINDOW: Duration = Duration::from_secs(30);
const ENSURE_PRODUCED_WINDOW: Duration = Duration::from_secs(60);
const ENSURE_STABILITY_DELAY: Duration = Duration::from_secs(1);

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

#[derive(Deserialize)]
struct ProducerStateResponse {
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
            let response = enqueue_with_retry(producer, payload, idx).await?;

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

        self.ensure_produced(producer, interval).await
    }
}

impl QueueProduceWorkload {
    /// Top up the queue until the produced count is durably visible.
    ///
    /// A node restart wipes its in-memory queue; ops accepted but not yet
    /// pulled by a peer (or enqueued before the restarted node re-adopted the
    /// cluster state) are lost. Re-reads the producer state and enqueues the
    /// deficit until the target sticks across a sync interval.
    async fn ensure_produced(
        &self,
        producer: &QueueHttpClient,
        interval: Option<Duration>,
    ) -> Result<(), DynError> {
        let deadline = tokio::time::Instant::now() + ENSURE_PRODUCED_WINDOW;
        let mut extra_index = 0_usize;

        loop {
            let observed = producer_queue_len(producer).await?;

            if observed >= self.operations {
                tokio::time::sleep(ENSURE_STABILITY_DELAY).await;
                if producer_queue_len(producer).await? >= self.operations {
                    if extra_index > 0 {
                        info!(
                            topped_up = extra_index,
                            target = self.operations,
                            "queue produce recovered lost operations"
                        );
                    }
                    return Ok(());
                }
                continue;
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "queue produce could not reach {} durable operations (observed {observed})",
                    self.operations
                )
                .into());
            }

            warn!(
                observed,
                target = self.operations,
                "queue produce detected lost operations; topping up"
            );

            for _ in observed..self.operations {
                let payload = format!("{}-extra-{extra_index}", self.payload_prefix);
                enqueue_with_retry(producer, payload, self.operations + extra_index).await?;
                extra_index += 1;

                if let Some(delay) = interval {
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
}

async fn enqueue_with_retry(
    producer: &QueueHttpClient,
    payload: String,
    operation: usize,
) -> Result<EnqueueResponse, DynError> {
    let deadline = tokio::time::Instant::now() + REQUEST_RETRY_WINDOW;

    loop {
        match producer
            .post(
                "/queue/enqueue",
                &EnqueueRequest {
                    payload: payload.clone(),
                },
            )
            .await
        {
            Ok(response) => {
                let response: EnqueueResponse = response;
                if !response.accepted {
                    return Err(format!("node rejected enqueue at operation {operation}").into());
                }
                return Ok(response);
            }
            Err(error) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(format!(
                        "queue enqueue kept failing at operation {operation}: {error}"
                    )
                    .into());
                }
                tokio::time::sleep(REQUEST_RETRY_INTERVAL).await;
            }
        }
    }
}

async fn producer_queue_len(producer: &QueueHttpClient) -> Result<usize, DynError> {
    let deadline = tokio::time::Instant::now() + REQUEST_RETRY_WINDOW;

    loop {
        match producer.get::<ProducerStateResponse>("/queue/state").await {
            Ok(state) => return Ok(state.queue_len),
            Err(error) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(
                        format!("queue state kept failing during produce top-up: {error}").into(),
                    );
                }
                tokio::time::sleep(REQUEST_RETRY_INTERVAL).await;
            }
        }
    }
}

fn compute_interval(rate_per_sec: usize) -> Option<Duration> {
    if rate_per_sec == 0 {
        return None;
    }

    Some(Duration::from_millis((1000 / rate_per_sec as u64).max(1)))
}

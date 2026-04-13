use std::time::Duration;

use async_trait::async_trait;
use metrics_counter_runtime_ext::MetricsCounterEnv;
use serde::Serialize;
use testing_framework_core::scenario::{DynError, RunContext, Workload};
use tracing::info;

#[derive(Clone)]
pub struct CounterIncrementWorkload {
    operations: usize,
    rate_per_sec: Option<usize>,
}

#[derive(Serialize)]
struct IncrementRequest {}

impl CounterIncrementWorkload {
    #[must_use]
    pub fn new() -> Self {
        Self {
            operations: 200,
            rate_per_sec: Some(25),
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
}

impl Default for CounterIncrementWorkload {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Workload<MetricsCounterEnv> for CounterIncrementWorkload {
    fn name(&self) -> &str {
        "counter_increment_workload"
    }

    async fn start(&self, ctx: &RunContext<MetricsCounterEnv>) -> Result<(), DynError> {
        let clients = ctx.node_clients().snapshot();
        if clients.is_empty() {
            return Err("no metrics-counter node clients available".into());
        }

        let interval = self.rate_per_sec.and_then(compute_interval);
        info!(operations = self.operations, rate_per_sec = ?self.rate_per_sec, "starting counter increment workload");

        for idx in 0..self.operations {
            let client = &clients[idx % clients.len()];
            let _: serde_json::Value = client.post("/counter/inc", &IncrementRequest {}).await?;

            if (idx + 1) % 50 == 0 {
                info!(completed = idx + 1, "counter increment progress");
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

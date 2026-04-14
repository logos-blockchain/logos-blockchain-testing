use std::time::Duration;

use async_trait::async_trait;
use kvstore_runtime_ext::KvEnv;
use serde::{Deserialize, Serialize};
use testing_framework_core::scenario::{DynError, RunContext, Workload};
use tracing::info;

#[derive(Clone)]
pub struct KvWriteWorkload {
    operations: usize,
    key_count: usize,
    rate_per_sec: Option<usize>,
    key_prefix: String,
}

#[derive(Serialize)]
struct PutRequest {
    value: String,
    expected_version: Option<u64>,
}

#[derive(Deserialize)]
struct PutResponse {
    applied: bool,
    version: u64,
}

impl KvWriteWorkload {
    #[must_use]
    pub fn new() -> Self {
        Self {
            operations: 200,
            key_count: 20,
            rate_per_sec: Some(25),
            key_prefix: "kv-demo".to_owned(),
        }
    }

    #[must_use]
    pub const fn operations(mut self, value: usize) -> Self {
        self.operations = value;
        self
    }

    #[must_use]
    pub const fn key_count(mut self, value: usize) -> Self {
        self.key_count = value;
        self
    }

    #[must_use]
    pub const fn rate_per_sec(mut self, value: usize) -> Self {
        self.rate_per_sec = Some(value);
        self
    }

    #[must_use]
    pub fn key_prefix(mut self, value: impl Into<String>) -> Self {
        self.key_prefix = value.into();
        self
    }
}

impl Default for KvWriteWorkload {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Workload<KvEnv> for KvWriteWorkload {
    fn name(&self) -> &str {
        "kv_write_workload"
    }

    async fn start(&self, ctx: &RunContext<KvEnv>) -> Result<(), DynError> {
        let clients = ctx.node_clients().snapshot();
        let Some(leader) = clients.first() else {
            return Err("no kv node clients available".into());
        };

        if self.key_count == 0 {
            return Err("kv workload key_count must be > 0".into());
        }

        let interval = self.rate_per_sec.and_then(compute_interval);
        info!(
            operations = self.operations,
            key_count = self.key_count,
            rate_per_sec = ?self.rate_per_sec,
            "starting kv write workload"
        );

        for idx in 0..self.operations {
            let key = format!("{}-{}", self.key_prefix, idx % self.key_count);
            let value = format!("value-{idx}");
            let response: PutResponse = leader
                .put(
                    &format!("/kv/{key}"),
                    &PutRequest {
                        value,
                        expected_version: None,
                    },
                )
                .await?;

            if !response.applied {
                return Err(format!("leader rejected write for key {key}").into());
            }

            if (idx + 1) % 25 == 0 {
                info!(
                    completed = idx + 1,
                    version = response.version,
                    "kv write progress"
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

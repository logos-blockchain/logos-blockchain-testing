use std::time::Duration;

use async_trait::async_trait;
use kvstore_runtime_ext::KvEnv;
use serde::Deserialize;
use testing_framework_core::scenario::{DynError, Expectation, RunContext};
use tracing::info;

#[derive(Clone)]
pub struct KvConverges {
    key_prefix: String,
    key_count: usize,
    timeout: Duration,
    poll_interval: Duration,
}

#[derive(Deserialize, Clone, Debug, Eq, PartialEq)]
struct ValueRecord {
    value: String,
    version: u64,
    origin: u64,
}

#[derive(Deserialize)]
struct GetResponse {
    record: Option<ValueRecord>,
}

impl KvConverges {
    #[must_use]
    pub fn new(key_prefix: impl Into<String>, key_count: usize) -> Self {
        Self {
            key_prefix: key_prefix.into(),
            key_count,
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
impl Expectation<KvEnv> for KvConverges {
    fn name(&self) -> &str {
        "kv_converges"
    }

    async fn evaluate(&mut self, ctx: &RunContext<KvEnv>) -> Result<(), DynError> {
        let clients = ctx.node_clients().snapshot();
        if clients.is_empty() {
            return Err("no kv node clients available".into());
        }

        let deadline = tokio::time::Instant::now() + self.timeout;
        while tokio::time::Instant::now() < deadline {
            if self.is_converged(&clients).await? {
                info!(key_count = self.key_count, "kv convergence reached");
                return Ok(());
            }
            tokio::time::sleep(self.poll_interval).await;
        }

        Err(format!(
            "kv convergence not reached within {:?} for {} keys",
            self.timeout, self.key_count
        )
        .into())
    }
}

impl KvConverges {
    async fn is_converged(&self, clients: &[kvstore_node::KvHttpClient]) -> Result<bool, DynError> {
        for key_idx in 0..self.key_count {
            let key = format!("{}-{key_idx}", self.key_prefix);
            let first = read_key(clients, &key, 0).await?;
            for node_idx in 1..clients.len() {
                let current = read_key(clients, &key, node_idx).await?;
                if current != first {
                    return Ok(false);
                }
            }
        }

        Ok(true)
    }
}

async fn read_key(
    clients: &[kvstore_node::KvHttpClient],
    key: &str,
    index: usize,
) -> Result<Option<ValueRecord>, DynError> {
    let response: GetResponse = clients[index].get(&format!("/kv/{key}")).await?;
    Ok(response.record)
}

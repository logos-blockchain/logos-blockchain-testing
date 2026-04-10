use std::time::Duration;

use async_trait::async_trait;
use redis_streams_runtime_ext::{RedisStreamsClient, RedisStreamsEnv};
use testing_framework_core::scenario::{DynError, RunContext, Workload};
use tokio::time::Instant;
use tracing::info;

#[derive(Clone)]
pub struct RedisStreamsReclaimFailoverWorkload {
    stream: String,
    group: String,
    producer_consumer: String,
    failover_consumer: String,
    messages: usize,
    batch: usize,
    timeout: Duration,
}

impl RedisStreamsReclaimFailoverWorkload {
    #[must_use]
    pub fn new(stream: impl Into<String>, group: impl Into<String>) -> Self {
        Self {
            stream: stream.into(),
            group: group.into(),
            producer_consumer: "worker-a".to_owned(),
            failover_consumer: "worker-b".to_owned(),
            messages: 200,
            batch: 64,
            timeout: Duration::from_secs(20),
        }
    }

    #[must_use]
    pub const fn messages(mut self, value: usize) -> Self {
        self.messages = value;
        self
    }

    #[must_use]
    pub const fn batch(mut self, value: usize) -> Self {
        self.batch = value;
        self
    }

    #[must_use]
    pub fn producer_consumer(mut self, value: impl Into<String>) -> Self {
        self.producer_consumer = value.into();
        self
    }

    #[must_use]
    pub fn failover_consumer(mut self, value: impl Into<String>) -> Self {
        self.failover_consumer = value.into();
        self
    }

    #[must_use]
    pub const fn timeout(mut self, value: Duration) -> Self {
        self.timeout = value;
        self
    }
}

#[async_trait]
impl Workload<RedisStreamsEnv> for RedisStreamsReclaimFailoverWorkload {
    fn name(&self) -> &str {
        "redis_streams_reclaim_failover_workload"
    }

    async fn start(&self, ctx: &RunContext<RedisStreamsEnv>) -> Result<(), DynError> {
        let clients = ctx.node_clients().snapshot();
        if clients.is_empty() {
            return Err("redis streams failover workload requires at least 1 node client".into());
        }

        let driver = &clients[0];
        driver.ensure_group(&self.stream, &self.group).await?;

        info!(messages = self.messages, stream = %self.stream, group = %self.group, "redis streams failover: produce phase");
        produce_messages(driver, &self.stream, self.messages).await?;

        info!(messages = self.messages, consumer = %self.producer_consumer, "redis streams failover: create pending phase");
        let pending_count = create_pending_messages(
            driver,
            &self.stream,
            &self.group,
            &self.producer_consumer,
            self.messages,
            self.batch,
            self.timeout,
        )
        .await?;

        info!(pending = pending_count, from = %self.producer_consumer, to = %self.failover_consumer, "redis streams failover: reclaim+ack phase");
        reclaim_and_ack_pending(
            driver,
            &self.stream,
            &self.group,
            &self.failover_consumer,
            pending_count,
            self.batch,
            self.timeout,
        )
        .await?;

        let pending = driver.pending_count(&self.stream, &self.group).await?;
        if pending != 0 {
            return Err(
                format!("redis streams pending entries remain after reclaim: {pending}").into(),
            );
        }

        info!(
            pending = pending_count,
            "redis streams failover reclaim complete"
        );
        Ok(())
    }
}

async fn produce_messages(
    client: &RedisStreamsClient,
    stream: &str,
    messages: usize,
) -> Result<(), DynError> {
    for idx in 0..messages {
        let payload = format!("msg-{idx}");
        client.append_message(stream, &payload).await?;
    }

    Ok(())
}

async fn create_pending_messages(
    client: &RedisStreamsClient,
    stream: &str,
    group: &str,
    consumer: &str,
    expected: usize,
    batch: usize,
    timeout: Duration,
) -> Result<usize, DynError> {
    let mut claimed = 0usize;
    let deadline = Instant::now() + timeout;

    while claimed < expected && Instant::now() < deadline {
        let ids = client
            .read_group_batch(stream, group, consumer, batch, 500)
            .await?;

        if ids.is_empty() {
            continue;
        }

        claimed += ids.len();
    }

    if claimed == expected {
        return Ok(claimed);
    }

    Err(format!("redis streams pending creation timed out: claimed {claimed}/{expected}").into())
}

async fn reclaim_and_ack_pending(
    client: &RedisStreamsClient,
    stream: &str,
    group: &str,
    failover_consumer: &str,
    expected: usize,
    batch: usize,
    timeout: Duration,
) -> Result<(), DynError> {
    let mut reclaimed = 0usize;
    let mut cursor = "0-0".to_owned();
    let deadline = Instant::now() + timeout;

    while reclaimed < expected && Instant::now() < deadline {
        let (next_cursor, ids) = client
            .autoclaim_batch(stream, group, failover_consumer, 0, &cursor, batch)
            .await?;

        if ids.is_empty() {
            if next_cursor == "0-0" {
                break;
            }
            cursor = next_cursor;
            continue;
        }

        let acked = client.ack_messages(stream, group, &ids).await? as usize;
        reclaimed += acked;
        cursor = next_cursor;
    }

    if reclaimed == expected {
        return Ok(());
    }

    Err(format!("redis streams reclaim timed out: acked {reclaimed}/{expected}").into())
}

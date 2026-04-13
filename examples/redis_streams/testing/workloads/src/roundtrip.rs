use std::time::Duration;

use async_trait::async_trait;
use redis_streams_runtime_ext::RedisStreamsEnv;
use testing_framework_core::scenario::{DynError, RunContext, Workload};
use tokio::time::Instant;
use tracing::info;

#[derive(Clone)]
pub struct RedisStreamsRoundTripWorkload {
    stream: String,
    group: String,
    consumer: String,
    messages: usize,
    read_batch: usize,
    timeout: Duration,
}

impl RedisStreamsRoundTripWorkload {
    #[must_use]
    pub fn new(stream: impl Into<String>, group: impl Into<String>) -> Self {
        Self {
            stream: stream.into(),
            group: group.into(),
            consumer: "worker-1".to_owned(),
            messages: 200,
            read_batch: 32,
            timeout: Duration::from_secs(20),
        }
    }

    #[must_use]
    pub const fn messages(mut self, value: usize) -> Self {
        self.messages = value;
        self
    }

    #[must_use]
    pub const fn read_batch(mut self, value: usize) -> Self {
        self.read_batch = value;
        self
    }

    #[must_use]
    pub fn consumer(mut self, value: impl Into<String>) -> Self {
        self.consumer = value.into();
        self
    }

    #[must_use]
    pub const fn timeout(mut self, value: Duration) -> Self {
        self.timeout = value;
        self
    }
}

#[async_trait]
impl Workload<RedisStreamsEnv> for RedisStreamsRoundTripWorkload {
    fn name(&self) -> &str {
        "redis_streams_roundtrip_workload"
    }

    async fn start(&self, ctx: &RunContext<RedisStreamsEnv>) -> Result<(), DynError> {
        let clients = ctx.node_clients().snapshot();
        if clients.is_empty() {
            return Err("redis streams workload requires at least 1 node".into());
        }

        let driver = &clients[0];

        driver.ensure_group(&self.stream, &self.group).await?;

        info!(messages = self.messages, stream = %self.stream, group = %self.group, "redis streams produce phase");
        for idx in 0..self.messages {
            let payload = format!("msg-{idx}");
            driver.append_message(&self.stream, &payload).await?;
        }

        info!(messages = self.messages, stream = %self.stream, group = %self.group, "redis streams consume+ack phase");
        consume_and_ack(
            driver,
            &self.stream,
            &self.group,
            &self.consumer,
            self.messages,
            self.read_batch,
            self.timeout,
        )
        .await?;

        let pending = driver.pending_count(&self.stream, &self.group).await?;
        if pending != 0 {
            return Err(format!("redis streams pending entries remain: {pending}").into());
        }

        info!(messages = self.messages, stream = %self.stream, group = %self.group, "redis streams roundtrip finished");
        Ok(())
    }
}

async fn consume_and_ack(
    client: &redis_streams_runtime_ext::RedisStreamsClient,
    stream: &str,
    group: &str,
    consumer: &str,
    expected: usize,
    batch: usize,
    timeout: Duration,
) -> Result<(), DynError> {
    let mut acked_total = 0usize;
    let deadline = Instant::now() + timeout;

    while acked_total < expected && Instant::now() < deadline {
        let ids = client
            .read_group_batch(stream, group, consumer, batch, 500)
            .await?;

        if ids.is_empty() {
            continue;
        }

        let acked = client.ack_messages(stream, group, &ids).await? as usize;
        acked_total += acked;
    }

    if acked_total == expected {
        return Ok(());
    }

    Err(format!("redis streams timed out: acked {acked_total}/{expected} messages").into())
}

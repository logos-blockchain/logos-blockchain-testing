use std::{collections::HashSet, time::Duration};

use async_trait::async_trait;
use futures_util::StreamExt;
use nats_runtime_ext::NatsEnv;
use testing_framework_core::scenario::{DynError, RunContext, Workload};
use tokio::time::Instant;
use tracing::info;

#[derive(Clone)]
pub struct NatsRoundTripWorkload {
    subject: String,
    messages: usize,
    publish_rate_per_sec: Option<usize>,
    timeout: Duration,
}

impl NatsRoundTripWorkload {
    #[must_use]
    pub fn new(subject: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            messages: 200,
            publish_rate_per_sec: Some(50),
            timeout: Duration::from_secs(20),
        }
    }

    #[must_use]
    pub const fn messages(mut self, value: usize) -> Self {
        self.messages = value;
        self
    }

    #[must_use]
    pub const fn publish_rate_per_sec(mut self, value: usize) -> Self {
        self.publish_rate_per_sec = Some(value);
        self
    }

    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

#[async_trait]
impl Workload<NatsEnv> for NatsRoundTripWorkload {
    fn name(&self) -> &str {
        "nats_roundtrip_workload"
    }

    async fn start(&self, ctx: &RunContext<NatsEnv>) -> Result<(), DynError> {
        let clients = ctx.node_clients().snapshot();
        if clients.len() < 2 {
            return Err("nats roundtrip workload requires at least 2 nodes".into());
        }

        let subscriber_client = clients[1].connect().await?;
        let mut subscription = subscriber_client.subscribe(self.subject.clone()).await?;

        let publisher = clients[0].connect().await?;
        let interval = self.publish_rate_per_sec.and_then(compute_interval);

        info!(messages = self.messages, subject = %self.subject, "nats publish phase");
        for idx in 0..self.messages {
            let payload = format!("msg-{idx}");
            publisher
                .publish(self.subject.clone(), payload.into())
                .await?;

            if let Some(delay) = interval {
                tokio::time::sleep(delay).await;
            }
        }
        publisher.flush().await?;

        info!(messages = self.messages, subject = %self.subject, "nats consume phase");
        let mut expected = (0..self.messages)
            .map(|idx| format!("msg-{idx}"))
            .collect::<HashSet<_>>();
        let deadline = Instant::now() + self.timeout;

        while !expected.is_empty() && Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let Some(message) = tokio::time::timeout(remaining, subscription.next()).await? else {
                break;
            };

            let payload = String::from_utf8(message.payload.to_vec())?;
            expected.remove(&payload);
        }

        if expected.is_empty() {
            info!(messages = self.messages, "nats roundtrip finished");
            return Ok(());
        }

        Err(format!(
            "nats roundtrip timed out: missing {} messages",
            expected.len()
        )
        .into())
    }
}

fn compute_interval(rate_per_sec: usize) -> Option<Duration> {
    if rate_per_sec == 0 {
        return None;
    }

    Some(Duration::from_millis((1000 / rate_per_sec as u64).max(1)))
}

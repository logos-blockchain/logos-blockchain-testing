use std::{collections::HashSet, time::Duration};

use async_trait::async_trait;
use pubsub_node::PubSubSession;
use pubsub_runtime_ext::PubSubEnv;
use testing_framework_core::scenario::{DynError, RunContext, Workload};
use tokio::time::Instant;
use tracing::info;

#[derive(Clone)]
pub struct PubSubWsReconnectWorkload {
    topic: String,
    phase_one_messages: usize,
    disconnected_messages: usize,
    phase_two_messages: usize,
    publish_rate_per_sec: Option<usize>,
    timeout: Duration,
}

impl PubSubWsReconnectWorkload {
    #[must_use]
    pub fn new(topic: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            phase_one_messages: 40,
            disconnected_messages: 20,
            phase_two_messages: 40,
            publish_rate_per_sec: Some(20),
            timeout: Duration::from_secs(20),
        }
    }

    #[must_use]
    pub const fn phase_one_messages(mut self, value: usize) -> Self {
        self.phase_one_messages = value;
        self
    }

    #[must_use]
    pub const fn disconnected_messages(mut self, value: usize) -> Self {
        self.disconnected_messages = value;
        self
    }

    #[must_use]
    pub const fn phase_two_messages(mut self, value: usize) -> Self {
        self.phase_two_messages = value;
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

    #[must_use]
    pub const fn total_messages(&self) -> usize {
        self.phase_one_messages + self.disconnected_messages + self.phase_two_messages
    }
}

impl Default for PubSubWsReconnectWorkload {
    fn default() -> Self {
        Self::new("demo.topic")
    }
}

#[async_trait]
impl Workload<PubSubEnv> for PubSubWsReconnectWorkload {
    fn name(&self) -> &str {
        "pubsub_ws_reconnect_workload"
    }

    async fn start(&self, ctx: &RunContext<PubSubEnv>) -> Result<(), DynError> {
        let clients = ctx.node_clients().snapshot();
        if clients.len() < 2 {
            return Err("pubsub reconnect workload requires at least 2 nodes".into());
        }

        let delay = self.publish_rate_per_sec.and_then(compute_interval);

        let mut subscriber = clients[1].connect().await?;
        subscriber.subscribe(&self.topic).await?;

        let mut publisher = clients[0].connect().await?;

        info!(topic = %self.topic, "pubsub reconnect phase 1: subscriber connected");
        publish_batch(
            &mut publisher,
            &self.topic,
            "phase1",
            self.phase_one_messages,
            delay,
        )
        .await?;

        subscriber.close().await?;

        info!(topic = %self.topic, "pubsub reconnect phase 2: subscriber disconnected");
        publish_batch(
            &mut publisher,
            &self.topic,
            "phase_disconnected",
            self.disconnected_messages,
            delay,
        )
        .await?;

        let mut subscriber = clients[1].connect().await?;
        subscriber.subscribe(&self.topic).await?;

        info!(topic = %self.topic, "pubsub reconnect phase 3: subscriber reconnected");
        publish_batch(
            &mut publisher,
            &self.topic,
            "phase2",
            self.phase_two_messages,
            delay,
        )
        .await?;

        let received = collect_prefixed_events(
            &mut subscriber,
            "phase2-",
            self.phase_two_messages,
            self.timeout,
        )
        .await?;

        if received != self.phase_two_messages {
            return Err(format!(
                "reconnected subscriber saw {received}/{} phase2 messages",
                self.phase_two_messages
            )
            .into());
        }

        Ok(())
    }
}

async fn publish_batch(
    publisher: &mut PubSubSession,
    topic: &str,
    prefix: &str,
    count: usize,
    delay: Option<Duration>,
) -> Result<(), DynError> {
    for i in 0..count {
        publisher.publish(topic, format!("{prefix}-{i}")).await?;

        if let Some(interval) = delay {
            tokio::time::sleep(interval).await;
        }
    }

    Ok(())
}

async fn collect_prefixed_events(
    subscriber: &mut PubSubSession,
    payload_prefix: &str,
    expected: usize,
    timeout: Duration,
) -> Result<usize, DynError> {
    let mut seen_ids = HashSet::new();
    let deadline = Instant::now() + timeout;

    while Instant::now() < deadline && seen_ids.len() < expected {
        let Some(event) = subscriber
            .next_event_timeout(Duration::from_millis(100))
            .await?
        else {
            continue;
        };

        if !event.payload.starts_with(payload_prefix) {
            continue;
        }

        if !seen_ids.insert(event.id) {
            return Err("duplicate phase2 event id observed".into());
        }
    }

    Ok(seen_ids.len())
}

fn compute_interval(rate_per_sec: usize) -> Option<Duration> {
    if rate_per_sec == 0 {
        return None;
    }

    Some(Duration::from_millis((1000 / rate_per_sec as u64).max(1)))
}

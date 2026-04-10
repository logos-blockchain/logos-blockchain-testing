use std::time::Duration;

use async_trait::async_trait;
use pubsub_runtime_ext::PubSubEnv;
use testing_framework_core::scenario::{DynError, RunContext, Workload};
use tracing::info;

#[derive(Clone)]
pub struct PubSubWsRoundTripWorkload {
    topic: String,
    messages: usize,
    publish_rate_per_sec: Option<usize>,
}

impl PubSubWsRoundTripWorkload {
    #[must_use]
    pub fn new(topic: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            messages: 100,
            publish_rate_per_sec: Some(20),
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
}

#[async_trait]
impl Workload<PubSubEnv> for PubSubWsRoundTripWorkload {
    fn name(&self) -> &str {
        "pubsub_ws_roundtrip_workload"
    }

    async fn start(&self, ctx: &RunContext<PubSubEnv>) -> Result<(), DynError> {
        let clients = ctx.node_clients().snapshot();
        if clients.is_empty() {
            return Err("pubsub workload requires at least 1 node".into());
        }

        let mut publisher = clients[0].connect().await?;
        let delay = self.publish_rate_per_sec.and_then(compute_interval);

        info!(messages = self.messages, topic = %self.topic, "pubsub ws roundtrip publish phase");
        for i in 0..self.messages {
            publisher.publish(&self.topic, format!("msg-{i}")).await?;

            if let Some(interval) = delay {
                tokio::time::sleep(interval).await;
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

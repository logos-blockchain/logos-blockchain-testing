use std::{collections::BTreeMap, time::Duration};

use async_trait::async_trait;
use pubsub_node::PubSubClient;
use pubsub_runtime_ext::PubSubEnv;
use serde::Deserialize;
use testing_framework_core::scenario::{DynError, Expectation, RunContext};
use tokio::time::Instant;
use tracing::info;

#[derive(Clone)]
pub struct PubSubConverges {
    topic: String,
    min_messages: usize,
    timeout: Duration,
    poll_interval: Duration,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct Revision {
    version: u64,
    origin: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct TopicsState {
    revision: Revision,
    total_events: usize,
    topic_counts: BTreeMap<String, usize>,
}

impl PubSubConverges {
    #[must_use]
    pub fn new(topic: impl Into<String>, min_messages: usize) -> Self {
        Self {
            topic: topic.into(),
            min_messages,
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
impl Expectation<PubSubEnv> for PubSubConverges {
    fn name(&self) -> &str {
        "pubsub_converges"
    }

    async fn evaluate(&mut self, ctx: &RunContext<PubSubEnv>) -> Result<(), DynError> {
        let clients = ctx.node_clients().snapshot();
        if clients.is_empty() {
            return Err("no pubsub node clients available".into());
        }

        let deadline = Instant::now() + self.timeout;
        while Instant::now() < deadline {
            if self.is_converged(&clients).await? {
                info!(topic = %self.topic, min_messages = self.min_messages, "pubsub converged");
                return Ok(());
            }
            tokio::time::sleep(self.poll_interval).await;
        }

        Err(format!("pubsub did not converge within {:?}", self.timeout).into())
    }
}

impl PubSubConverges {
    async fn is_converged(&self, clients: &[PubSubClient]) -> Result<bool, DynError> {
        let Some((first, rest)) = clients.split_first() else {
            return Ok(false);
        };

        let baseline = read_state(first).await?;
        if baseline
            .topic_counts
            .get(&self.topic)
            .copied()
            .unwrap_or_default()
            < self.min_messages
        {
            return Ok(false);
        }

        for client in rest {
            let current = read_state(client).await?;
            if current != baseline {
                return Ok(false);
            }
        }

        Ok(true)
    }
}

async fn read_state(client: &PubSubClient) -> Result<TopicsState, DynError> {
    Ok(client.get("/topics/state").await?)
}

use std::time::Duration;

use async_trait::async_trait;
use pubsub_runtime_ext::{PubSubEnv, PubSubTopicFeed};
use testing_framework_core::scenario::{DynError, Expectation, RunContext};
use tokio::time::Instant;
use tracing::info;

#[derive(Clone)]
pub struct PubSubFeedDelivers {
    topic: String,
    expected_messages: usize,
    timeout: Duration,
    poll_interval: Duration,
}

impl PubSubFeedDelivers {
    #[must_use]
    pub fn new(topic: impl Into<String>, expected_messages: usize) -> Self {
        Self {
            topic: topic.into(),
            expected_messages,
            timeout: Duration::from_secs(20),
            poll_interval: Duration::from_millis(200),
        }
    }

    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

#[async_trait]
impl Expectation<PubSubEnv> for PubSubFeedDelivers {
    fn name(&self) -> &str {
        "pubsub_feed_delivers"
    }

    async fn evaluate(&mut self, ctx: &RunContext<PubSubEnv>) -> Result<(), DynError> {
        let feed = ctx.require_extension::<PubSubTopicFeed>()?;
        if feed.topic() != self.topic {
            return Err(format!(
                "pubsub topic feed is configured for '{}' but expectation expects '{}'",
                feed.topic(),
                self.topic
            )
            .into());
        }

        let deadline = Instant::now() + self.timeout;
        while Instant::now() < deadline {
            let snapshot = feed.snapshot().await;
            if snapshot.ensure_consistent(self.expected_messages)? {
                info!(
                    topic = %self.topic,
                    expected_messages = self.expected_messages,
                    subscribers = snapshot.subscriber_count(),
                    "pubsub feed delivered consistent topic events"
                );
                return Ok(());
            }

            tokio::time::sleep(self.poll_interval).await;
        }

        let snapshot = feed.snapshot().await;
        let counts = (0..snapshot.subscriber_count())
            .map(|index| {
                format!(
                    "{index}:{}",
                    snapshot.subscriber_message_count(index).unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join(", ");

        Err(format!(
            "pubsub feed did not observe consistent delivery within {:?} (counts: {counts})",
            self.timeout
        )
        .into())
    }
}

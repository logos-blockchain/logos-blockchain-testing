use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use pubsub_node::{PubSubClient, PubSubEventId, PubSubSession};
use testing_framework_app::{AppDeployment, AppHostEnv, ClusterApp, DeployContext};
use testing_framework_core::scenario::{CleanupGuard, ClusterProvisioner, DynError};
use tokio::{sync::Mutex, task::JoinHandle};

use crate::{PubSubEnv, PubSubTopology};

#[derive(Clone)]
pub struct PubSubTopicFeed {
    state: Arc<PubSubTopicFeedState>,
}

struct PubSubTopicFeedState {
    topic: String,
    deliveries: Mutex<Vec<HashMap<PubSubEventId, String>>>,
}

#[derive(Clone, Default)]
pub struct PubSubTopicFeedSnapshot {
    deliveries: Vec<HashMap<PubSubEventId, String>>,
}

impl PubSubTopicFeed {
    fn new(topic: String, subscribers: usize) -> Self {
        Self {
            state: Arc::new(PubSubTopicFeedState {
                topic,
                deliveries: Mutex::new(vec![HashMap::new(); subscribers]),
            }),
        }
    }

    #[must_use]
    pub fn topic(&self) -> &str {
        self.state.topic.as_str()
    }

    pub async fn snapshot(&self) -> PubSubTopicFeedSnapshot {
        PubSubTopicFeedSnapshot {
            deliveries: self.state.deliveries.lock().await.clone(),
        }
    }

    async fn record_delivery(&self, subscriber_index: usize, id: PubSubEventId, payload: String) {
        let mut deliveries = self.state.deliveries.lock().await;
        deliveries[subscriber_index].entry(id).or_insert(payload);
    }
}

impl PubSubTopicFeedSnapshot {
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.deliveries.len()
    }

    #[must_use]
    pub fn subscriber_message_count(&self, subscriber_index: usize) -> Option<usize> {
        self.deliveries.get(subscriber_index).map(HashMap::len)
    }

    pub fn ensure_consistent(&self, expected_messages: usize) -> Result<bool, DynError> {
        for (index, seen) in self.deliveries.iter().enumerate() {
            if seen.len() > expected_messages {
                return Err(format!(
                    "subscriber {index} saw {}/{} expected messages",
                    seen.len(),
                    expected_messages
                )
                .into());
            }

            if seen.len() < expected_messages {
                return Ok(false);
            }
        }

        if let Some((baseline, rest)) = self.deliveries.split_first() {
            for seen in rest {
                if seen != baseline {
                    return Err("subscriber deliveries diverged".into());
                }
            }
        }

        Ok(true)
    }
}

#[derive(Clone)]
pub struct PubSubStackApp {
    deployment: PubSubTopology,
    topic: String,
}

impl PubSubStackApp {
    #[must_use]
    pub fn new(deployment: PubSubTopology, topic: impl Into<String>) -> Self {
        Self {
            deployment,
            topic: topic.into(),
        }
    }
}

#[async_trait]
impl<P> AppDeployment<AppHostEnv, P> for PubSubStackApp
where
    P: ClusterProvisioner<PubSubEnv>,
{
    type Handle = PubSubTopicFeed;

    async fn deploy(
        self,
        ctx: &mut DeployContext<AppHostEnv, P>,
    ) -> Result<Self::Handle, DynError> {
        let cluster = ctx
            .deploy_and_expose(ClusterApp::<PubSubEnv>::new(self.deployment))
            .await?;

        let clients = cluster.clients();
        if clients.len() < 2 {
            return Err("pubsub topic feed requires at least 2 node clients".into());
        }

        let feed = PubSubTopicFeed::new(self.topic.clone(), clients.len());
        let sessions = connect_subscribers(&clients, &self.topic).await?;
        let collector = tokio::spawn(run_collector(feed.clone(), sessions));
        ctx.defer_cleanup(Box::new(CollectorTaskGuard { task: collector }));
        ctx.expose(feed.clone())?;

        Ok(feed)
    }
}

struct CollectorTaskGuard {
    task: JoinHandle<()>,
}

impl CleanupGuard for CollectorTaskGuard {
    fn cleanup(self: Box<Self>) {
        self.task.abort();
    }
}

async fn connect_subscribers(
    clients: &[PubSubClient],
    topic: &str,
) -> Result<Vec<PubSubSession>, DynError> {
    let mut sessions = Vec::with_capacity(clients.len());

    for client in clients {
        let mut session = client
            .connect()
            .await
            .map_err(|error| -> DynError { error.into() })?;
        session
            .subscribe(topic)
            .await
            .map_err(|error| -> DynError { error.into() })?;
        sessions.push(session);
    }

    Ok(sessions)
}

async fn run_collector(feed: PubSubTopicFeed, mut sessions: Vec<PubSubSession>) {
    loop {
        for (index, session) in sessions.iter_mut().enumerate() {
            match session.next_event_timeout(Duration::from_millis(100)).await {
                Ok(Some(event)) if event.topic == feed.topic() => {
                    feed.record_delivery(index, event.id, event.payload).await;
                }
                Ok(Some(_)) | Ok(None) => {}
                Err(_error) => {
                    return;
                }
            }
        }

        tokio::task::yield_now().await;
    }
}

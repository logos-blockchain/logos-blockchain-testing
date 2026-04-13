use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, broadcast};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Revision {
    pub version: u64,
    pub origin: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct EventId {
    pub origin: u64,
    pub seq: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TopicEvent {
    pub id: EventId,
    pub topic: String,
    pub payload: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub node_id: u64,
    pub revision: Revision,
    pub next_seq: u64,
    pub events: Vec<TopicEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TopicsStateView {
    pub revision: Revision,
    pub total_events: usize,
    pub topic_counts: BTreeMap<String, usize>,
}

#[derive(Debug)]
struct Data {
    revision: Revision,
    next_seq: u64,
    events: Vec<TopicEvent>,
    seen_ids: HashSet<EventId>,
}

impl Default for Data {
    fn default() -> Self {
        Self {
            revision: Revision::default(),
            next_seq: 1,
            events: Vec::new(),
            seen_ids: HashSet::new(),
        }
    }
}

#[derive(Clone)]
pub struct PubSubState {
    node_id: u64,
    ready: Arc<RwLock<bool>>,
    data: Arc<RwLock<Data>>,
    event_tx: broadcast::Sender<TopicEvent>,
}

impl PubSubState {
    pub fn new(node_id: u64) -> Self {
        let (event_tx, _) = broadcast::channel(2048);

        Self {
            node_id,
            ready: Arc::new(RwLock::new(false)),
            data: Arc::new(RwLock::new(Data::default())),
            event_tx,
        }
    }

    pub const fn node_id(&self) -> u64 {
        self.node_id
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<TopicEvent> {
        self.event_tx.subscribe()
    }

    pub async fn set_ready(&self, value: bool) {
        *self.ready.write().await = value;
    }

    pub async fn is_ready(&self) -> bool {
        *self.ready.read().await
    }

    pub async fn publish_local(&self, topic: String, payload: String) -> TopicEvent {
        let event = {
            let mut data = self.data.write().await;
            let event = TopicEvent {
                id: EventId {
                    origin: self.node_id,
                    seq: data.next_seq,
                },
                topic,
                payload,
            };

            data.next_seq = data.next_seq.saturating_add(1);
            data.seen_ids.insert(event.id.clone());
            data.events.push(event.clone());
            bump_revision(&mut data.revision, self.node_id);
            event
        };

        let _ = self.event_tx.send(event.clone());
        event
    }

    pub async fn merge_snapshot(&self, snapshot: Snapshot) {
        let merged_events = {
            let mut data = self.data.write().await;
            let mut added = Vec::new();

            for event in snapshot.events {
                if data.seen_ids.insert(event.id.clone()) {
                    data.events.push(event.clone());
                    added.push(event);
                }
            }

            data.events
                .sort_by_key(|event| (event.id.origin, event.id.seq));
            if is_newer_revision(snapshot.revision, data.revision) {
                data.revision = snapshot.revision;
            }
            data.next_seq = data.next_seq.max(snapshot.next_seq);
            added
        };

        for event in merged_events {
            let _ = self.event_tx.send(event);
        }
    }

    pub async fn snapshot(&self) -> Snapshot {
        let data = self.data.read().await;
        Snapshot {
            node_id: self.node_id,
            revision: data.revision,
            next_seq: data.next_seq,
            events: data.events.clone(),
        }
    }

    pub async fn topics_state(&self) -> TopicsStateView {
        let data = self.data.read().await;
        let mut topic_counts = BTreeMap::new();
        for event in &data.events {
            *topic_counts.entry(event.topic.clone()).or_insert(0) += 1;
        }

        TopicsStateView {
            revision: data.revision,
            total_events: data.events.len(),
            topic_counts,
        }
    }
}

fn bump_revision(revision: &mut Revision, node_id: u64) {
    revision.version = revision.version.saturating_add(1);
    revision.origin = node_id;
}

fn is_newer_revision(candidate: Revision, existing: Revision) -> bool {
    (candidate.version, candidate.origin) > (existing.version, existing.origin)
}

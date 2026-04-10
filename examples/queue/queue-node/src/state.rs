use std::{collections::VecDeque, sync::Arc};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct QueueRevision {
    pub version: u64,
    pub origin: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QueueMessage {
    pub id: u64,
    pub payload: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub node_id: u64,
    pub revision: QueueRevision,
    pub messages: Vec<QueueMessage>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QueueStateView {
    pub revision: QueueRevision,
    pub queue_len: usize,
    pub head_id: Option<u64>,
    pub tail_id: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct EnqueueOutcome {
    pub accepted: bool,
    pub id: u64,
    pub queue_len: usize,
    pub revision: QueueRevision,
}

#[derive(Clone, Debug)]
pub struct DequeueOutcome {
    pub message: Option<QueueMessage>,
    pub queue_len: usize,
    pub revision: QueueRevision,
}

#[derive(Debug, Default)]
struct QueueData {
    revision: QueueRevision,
    messages: VecDeque<QueueMessage>,
}

#[derive(Clone)]
pub struct QueueState {
    node_id: u64,
    ready: Arc<RwLock<bool>>,
    data: Arc<RwLock<QueueData>>,
}

impl QueueState {
    pub fn new(node_id: u64) -> Self {
        Self {
            node_id,
            ready: Arc::new(RwLock::new(false)),
            data: Arc::new(RwLock::new(QueueData::default())),
        }
    }

    pub const fn node_id(&self) -> u64 {
        self.node_id
    }

    pub async fn set_ready(&self, value: bool) {
        *self.ready.write().await = value;
    }

    pub async fn is_ready(&self) -> bool {
        *self.ready.read().await
    }

    pub async fn enqueue_local(&self, payload: String) -> EnqueueOutcome {
        let mut data = self.data.write().await;
        let id = next_message_id(&data.messages);
        data.messages.push_back(QueueMessage { id, payload });
        bump_revision(&mut data.revision, self.node_id);

        EnqueueOutcome {
            accepted: true,
            id,
            queue_len: data.messages.len(),
            revision: data.revision,
        }
    }

    pub async fn dequeue_local(&self) -> DequeueOutcome {
        let mut data = self.data.write().await;
        let message = data.messages.pop_front();
        if message.is_some() {
            bump_revision(&mut data.revision, self.node_id);
        }

        DequeueOutcome {
            message,
            queue_len: data.messages.len(),
            revision: data.revision,
        }
    }

    pub async fn queue_state(&self) -> QueueStateView {
        let data = self.data.read().await;
        QueueStateView {
            revision: data.revision,
            queue_len: data.messages.len(),
            head_id: data.messages.front().map(|message| message.id),
            tail_id: data.messages.back().map(|message| message.id),
        }
    }

    pub async fn merge_snapshot(&self, snapshot: Snapshot) {
        let mut data = self.data.write().await;
        if is_newer_revision(snapshot.revision, data.revision) {
            data.revision = snapshot.revision;
            data.messages = snapshot.messages.into();
        }
    }

    pub async fn snapshot(&self) -> Snapshot {
        let data = self.data.read().await;
        Snapshot {
            node_id: self.node_id,
            revision: data.revision,
            messages: data.messages.iter().cloned().collect(),
        }
    }
}

fn next_message_id(messages: &VecDeque<QueueMessage>) -> u64 {
    messages
        .back()
        .map_or(1, |message| message.id.saturating_add(1))
}

fn bump_revision(revision: &mut QueueRevision, node_id: u64) {
    revision.version = revision.version.saturating_add(1);
    revision.origin = node_id;
}

fn is_newer_revision(candidate: QueueRevision, existing: QueueRevision) -> bool {
    (candidate.version, candidate.origin) > (existing.version, existing.origin)
}

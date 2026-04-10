use std::{collections::HashMap, sync::Arc};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct ValueRecord {
    pub value: String,
    pub version: u64,
    pub origin: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub node_id: u64,
    pub entries: HashMap<String, ValueRecord>,
}

#[derive(Clone, Debug)]
pub struct PutOutcome {
    pub applied: bool,
    pub current_version: u64,
}

#[derive(Clone)]
pub struct KvState {
    node_id: u64,
    ready: Arc<RwLock<bool>>,
    entries: Arc<RwLock<HashMap<String, ValueRecord>>>,
}

impl KvState {
    pub fn new(node_id: u64) -> Self {
        Self {
            node_id,
            ready: Arc::new(RwLock::new(false)),
            entries: Arc::new(RwLock::new(HashMap::new())),
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

    pub async fn get(&self, key: &str) -> Option<ValueRecord> {
        self.entries.read().await.get(key).cloned()
    }

    pub async fn put_local(
        &self,
        key: String,
        value: String,
        expected_version: Option<u64>,
    ) -> PutOutcome {
        let mut entries = self.entries.write().await;
        let current_version = entries.get(&key).map_or(0, |record| record.version);

        if expected_version.is_some_and(|expected| expected != current_version) {
            return PutOutcome {
                applied: false,
                current_version,
            };
        }

        let next_version = current_version.saturating_add(1);
        entries.insert(
            key,
            ValueRecord {
                value,
                version: next_version,
                origin: self.node_id,
            },
        );

        PutOutcome {
            applied: true,
            current_version: next_version,
        }
    }

    pub async fn merge_snapshot(&self, snapshot: Snapshot) {
        let mut local = self.entries.write().await;
        for (key, incoming) in snapshot.entries {
            match local.get(&key) {
                Some(existing) if !is_newer_record(&incoming, existing) => {}
                _ => {
                    local.insert(key, incoming);
                }
            }
        }
    }

    pub async fn snapshot(&self) -> Snapshot {
        Snapshot {
            node_id: self.node_id,
            entries: self.entries.read().await.clone(),
        }
    }
}

fn is_newer_record(candidate: &ValueRecord, existing: &ValueRecord) -> bool {
    (candidate.version, candidate.origin) > (existing.version, existing.origin)
}

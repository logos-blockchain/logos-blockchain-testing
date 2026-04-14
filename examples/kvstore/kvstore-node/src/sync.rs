use std::{collections::HashMap, sync::Arc, time::Duration};

use reqwest::Client;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::{
    config::KvConfig,
    state::{KvState, Snapshot},
};

const WARN_AFTER_CONSECUTIVE_FAILURES: u32 = 5;

#[derive(Clone)]
pub struct SyncService {
    config: Arc<KvConfig>,
    state: KvState,
    client: Client,
    failures_by_peer: Arc<Mutex<HashMap<String, u32>>>,
}

impl SyncService {
    pub fn new(config: KvConfig, state: KvState) -> Self {
        Self {
            config: Arc::new(config),
            state,
            client: Client::new(),
            failures_by_peer: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn start(&self) {
        let service = self.clone();
        tokio::spawn(async move {
            service.run().await;
        });
    }

    async fn run(self) {
        let interval = Duration::from_millis(self.config.sync_interval_ms.max(100));
        loop {
            self.sync_once().await;
            tokio::time::sleep(interval).await;
        }
    }

    async fn sync_once(&self) {
        for peer in &self.config.peers {
            match self.fetch_snapshot(&peer.http_address).await {
                Ok(snapshot) => {
                    self.state.merge_snapshot(snapshot).await;
                    self.clear_failure_counter(&peer.http_address).await;
                }
                Err(error) => {
                    self.record_sync_failure(&peer.http_address, &error).await;
                }
            }
        }
    }

    async fn fetch_snapshot(&self, peer_address: &str) -> anyhow::Result<Snapshot> {
        let url = format!("http://{peer_address}/internal/snapshot");
        let snapshot = self
            .client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(snapshot)
    }

    async fn clear_failure_counter(&self, peer_address: &str) {
        let mut failures = self.failures_by_peer.lock().await;
        failures.remove(peer_address);
    }

    async fn record_sync_failure(&self, peer_address: &str, error: &anyhow::Error) {
        let consecutive_failures = {
            let mut failures = self.failures_by_peer.lock().await;
            let entry = failures.entry(peer_address.to_owned()).or_insert(0);
            *entry += 1;
            *entry
        };

        if consecutive_failures >= WARN_AFTER_CONSECUTIVE_FAILURES {
            warn!(
                peer = %peer_address,
                %error,
                consecutive_failures,
                "kv sync repeatedly failing"
            );
        } else {
            debug!(
                peer = %peer_address,
                %error,
                consecutive_failures,
                "kv sync failed"
            );
        }
    }
}

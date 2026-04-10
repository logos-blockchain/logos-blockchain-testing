use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerInfo {
    pub node_id: u64,
    pub http_address: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueueConfig {
    pub node_id: u64,
    pub http_port: u16,
    pub peers: Vec<PeerInfo>,
    #[serde(default = "default_sync_interval_ms")]
    pub sync_interval_ms: u64,
}

impl QueueConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw = fs::read_to_string(path)?;
        Ok(serde_yaml::from_str(&raw)?)
    }
}

const fn default_sync_interval_ms() -> u64 {
    1000
}

use std::{collections::BTreeMap, fs, path::Path};

use serde::{Deserialize, Serialize};

/// Static node config written by TF for one OpenRaft node process.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenRaftKvNodeConfig {
    /// Stable OpenRaft node identifier.
    pub node_id: u64,
    /// HTTP port bound by the node process.
    pub http_port: u16,
    /// Advertised Raft address for this node.
    pub public_addr: String,
    /// Advertised Raft addresses for the other known nodes.
    #[serde(default)]
    pub peer_addrs: BTreeMap<u64, String>,
    /// Heartbeat interval passed to the OpenRaft config.
    #[serde(default = "default_heartbeat_interval_ms")]
    pub heartbeat_interval_ms: u64,
    /// Lower election timeout bound passed to OpenRaft.
    #[serde(default = "default_election_timeout_min_ms")]
    pub election_timeout_min_ms: u64,
    /// Upper election timeout bound passed to OpenRaft.
    #[serde(default = "default_election_timeout_max_ms")]
    pub election_timeout_max_ms: u64,
}

impl OpenRaftKvNodeConfig {
    /// Loads one node config from YAML on disk.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw = fs::read_to_string(path)?;
        Ok(serde_yaml::from_str(&raw)?)
    }
}

const fn default_heartbeat_interval_ms() -> u64 {
    500
}

const fn default_election_timeout_min_ms() -> u64 {
    1_500
}

const fn default_election_timeout_max_ms() -> u64 {
    3_000
}

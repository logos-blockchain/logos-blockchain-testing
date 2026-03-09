use serde::{Deserialize, Serialize};

use crate::CfgSyncFile;

/// Top-level cfgsync bundle containing per-node file payloads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgSyncBundle {
    pub nodes: Vec<CfgSyncBundleNode>,
}

impl CfgSyncBundle {
    #[must_use]
    pub fn new(nodes: Vec<CfgSyncBundleNode>) -> Self {
        Self { nodes }
    }
}

/// Artifact set for a single node resolved by identifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgSyncBundleNode {
    /// Stable node identifier used by cfgsync lookup.
    pub identifier: String,
    /// Files that should be materialized for the node.
    #[serde(default)]
    pub files: Vec<CfgSyncFile>,
}

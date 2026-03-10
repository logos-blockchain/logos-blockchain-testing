use serde::{Deserialize, Serialize};

use crate::NodeArtifactFile;

/// Top-level cfgsync bundle containing per-node file payloads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeArtifactsBundle {
    pub nodes: Vec<NodeArtifactsBundleEntry>,
}

impl NodeArtifactsBundle {
    #[must_use]
    pub fn new(nodes: Vec<NodeArtifactsBundleEntry>) -> Self {
        Self { nodes }
    }
}

/// Artifact set for a single node resolved by identifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeArtifactsBundleEntry {
    /// Stable node identifier used by cfgsync lookup.
    pub identifier: String,
    /// Files that should be materialized for the node.
    #[serde(default)]
    pub files: Vec<NodeArtifactFile>,
}

#[doc(hidden)]
pub type CfgSyncBundle = NodeArtifactsBundle;

#[doc(hidden)]
pub type CfgSyncBundleNode = NodeArtifactsBundleEntry;

use serde::{Deserialize, Serialize};

use crate::NodeArtifactFile;

/// Static cfgsync artifact bundle.
///
/// This is the bundle-oriented source format used when all artifacts are known
/// ahead of time and no registration-time materialization is required.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeArtifactsBundle {
    /// Per-node artifact entries keyed by identifier.
    pub nodes: Vec<NodeArtifactsBundleEntry>,
    /// Files that should be served alongside every node-specific entry.
    #[serde(default)]
    pub shared_files: Vec<NodeArtifactFile>,
}

impl NodeArtifactsBundle {
    /// Creates a bundle with node-specific entries only.
    #[must_use]
    pub fn new(nodes: Vec<NodeArtifactsBundleEntry>) -> Self {
        Self {
            nodes,
            shared_files: Vec::new(),
        }
    }

    /// Attaches files that should be served alongside every node entry.
    #[must_use]
    pub fn with_shared_files(mut self, shared_files: Vec<NodeArtifactFile>) -> Self {
        self.shared_files = shared_files;
        self
    }
}

/// One node entry inside a static cfgsync bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeArtifactsBundleEntry {
    /// Stable node identifier used by cfgsync lookup.
    pub identifier: String,
    /// Files that should be materialized for the node.
    #[serde(default)]
    pub files: Vec<NodeArtifactFile>,
}

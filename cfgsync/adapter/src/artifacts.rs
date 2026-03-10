use std::collections::HashMap;

use cfgsync_artifacts::ArtifactFile;
use serde::{Deserialize, Serialize};

/// Per-node artifact payload served by cfgsync for one registered node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeArtifacts {
    /// Stable node identifier resolved by the adapter.
    pub identifier: String,
    /// Files served to the node after cfgsync registration.
    pub files: Vec<ArtifactFile>,
}

/// Materialized artifact files for a single registered node.
#[derive(Debug, Clone, Default)]
pub struct ArtifactSet {
    files: Vec<ArtifactFile>,
}

impl ArtifactSet {
    #[must_use]
    pub fn new(files: Vec<ArtifactFile>) -> Self {
        Self { files }
    }

    #[must_use]
    pub fn files(&self) -> &[ArtifactFile] {
        &self.files
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// Artifact payloads indexed by stable node identifier.
#[derive(Debug, Clone, Default)]
pub struct NodeArtifactsCatalog {
    nodes: HashMap<String, NodeArtifacts>,
}

impl NodeArtifactsCatalog {
    #[must_use]
    pub fn new(nodes: Vec<NodeArtifacts>) -> Self {
        let nodes = nodes
            .into_iter()
            .map(|node| (node.identifier.clone(), node))
            .collect();

        Self { nodes }
    }

    #[must_use]
    pub fn resolve(&self, identifier: &str) -> Option<&NodeArtifacts> {
        self.nodes.get(identifier)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    #[must_use]
    pub fn into_nodes(self) -> Vec<NodeArtifacts> {
        self.nodes.into_values().collect()
    }
}

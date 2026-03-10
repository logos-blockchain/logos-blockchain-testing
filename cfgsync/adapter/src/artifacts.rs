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

    #[must_use]
    pub fn into_files(self) -> Vec<ArtifactFile> {
        self.files
    }
}

/// Resolved artifact payload for one node, including any shared files that
/// should be delivered alongside the node-local files.
#[derive(Debug, Clone, Default)]
pub struct ResolvedNodeArtifacts {
    node: ArtifactSet,
    shared: ArtifactSet,
}

impl ResolvedNodeArtifacts {
    #[must_use]
    pub fn new(node: ArtifactSet, shared: ArtifactSet) -> Self {
        Self { node, shared }
    }

    #[must_use]
    pub fn node(&self) -> &ArtifactSet {
        &self.node
    }

    #[must_use]
    pub fn shared(&self) -> &ArtifactSet {
        &self.shared
    }

    #[must_use]
    pub fn files(&self) -> Vec<ArtifactFile> {
        let mut files = self.node.files().to_vec();
        files.extend_from_slice(self.shared.files());
        files
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

/// Materialized cfgsync output for a whole registration set.
#[derive(Debug, Clone, Default)]
pub struct MaterializedArtifacts {
    nodes: NodeArtifactsCatalog,
    shared: ArtifactSet,
}

impl MaterializedArtifacts {
    #[must_use]
    pub fn new(nodes: NodeArtifactsCatalog, shared: ArtifactSet) -> Self {
        Self { nodes, shared }
    }

    #[must_use]
    pub fn from_catalog(nodes: NodeArtifactsCatalog) -> Self {
        Self::new(nodes, ArtifactSet::default())
    }

    #[must_use]
    pub fn nodes(&self) -> &NodeArtifactsCatalog {
        &self.nodes
    }

    #[must_use]
    pub fn shared(&self) -> &ArtifactSet {
        &self.shared
    }

    #[must_use]
    pub fn resolve(&self, identifier: &str) -> Option<ResolvedNodeArtifacts> {
        self.nodes.resolve(identifier).map(|node| {
            ResolvedNodeArtifacts::new(ArtifactSet::new(node.files.clone()), self.shared.clone())
        })
    }
}

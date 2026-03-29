use std::collections::HashMap;

use cfgsync_artifacts::{ArtifactFile, ArtifactSet};
use serde::{Deserialize, Serialize};

/// Fully materialized cfgsync artifacts for a registration set.
///
/// `nodes` holds the node-local files keyed by stable node identifier.
/// `shared` holds files that should be delivered alongside every node.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MaterializedArtifacts {
    nodes: HashMap<String, ArtifactSet>,
    shared: ArtifactSet,
}

impl MaterializedArtifacts {
    /// Creates materialized artifacts from node-local artifact sets.
    #[must_use]
    pub fn from_nodes<I>(nodes: I) -> Self
    where
        I: IntoIterator<Item = (String, ArtifactSet)>,
    {
        Self {
            nodes: nodes.into_iter().collect(),
            shared: ArtifactSet::default(),
        }
    }

    /// Attaches shared files delivered alongside every node.
    #[must_use]
    pub fn with_shared(mut self, shared: ArtifactSet) -> Self {
        self.shared = shared;
        self
    }

    /// Returns the node-local artifact set for one identifier.
    #[must_use]
    pub fn node(&self, identifier: &str) -> Option<&ArtifactSet> {
        self.nodes.get(identifier)
    }

    /// Inserts or replaces the node-local artifact set for one identifier.
    pub fn set_node(&mut self, identifier: impl Into<String>, artifacts: ArtifactSet) {
        self.nodes.insert(identifier.into(), artifacts);
    }

    /// Returns the shared artifact set.
    #[must_use]
    pub fn shared(&self) -> &ArtifactSet {
        &self.shared
    }

    /// Returns the number of node-local artifact sets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Returns `true` when no node-local artifact sets are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Resolves the full file set that should be written for one node.
    #[must_use]
    pub fn resolve(&self, identifier: &str) -> Option<ArtifactSet> {
        let node = self.node(identifier)?;
        let mut files: Vec<ArtifactFile> = node.files.clone();
        files.extend(self.shared.files.iter().cloned());
        Some(ArtifactSet::new(files))
    }

    /// Iterates node-local artifact sets by stable identifier.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ArtifactSet)> {
        self.nodes
            .iter()
            .map(|(identifier, artifacts)| (identifier.as_str(), artifacts))
    }
}

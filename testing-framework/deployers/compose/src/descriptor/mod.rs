use serde::Serialize;

mod node;

pub use node::{EnvEntry, NodeDescriptor};

/// Top-level docker-compose descriptor built from an environment-specific
/// topology.
#[derive(Clone, Debug, Serialize)]
pub struct ComposeDescriptor {
    nodes: Vec<NodeDescriptor>,
}

impl ComposeDescriptor {
    #[must_use]
    pub fn new(nodes: Vec<NodeDescriptor>) -> Self {
        Self { nodes }
    }

    #[must_use]
    pub fn nodes(&self) -> &[NodeDescriptor] {
        &self.nodes
    }

    #[cfg(test)]
    pub fn test_nodes(&self) -> &[NodeDescriptor] {
        self.nodes()
    }
}

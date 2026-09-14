use serde::Serialize;

mod node;

pub use node::{
    BinaryConfigNodeSpec, EnvEntry, LoopbackNodeRuntimeSpec, NodeDescriptor,
    binary_config_node_runtime_spec, build_binary_config_node_descriptors,
    build_binary_config_node_descriptors_with_file_name, build_loopback_node_descriptors,
};

/// Top-level docker-compose descriptor built from an environment-specific
/// topology.
#[derive(Clone, Debug, Serialize)]
pub struct ComposeDescriptor {
    nodes: Vec<NodeDescriptor>,
    external_network: Option<String>,
}

impl ComposeDescriptor {
    #[must_use]
    pub fn new(nodes: Vec<NodeDescriptor>) -> Self {
        Self {
            nodes,
            external_network: None,
        }
    }

    /// Attaches every service to the given pre-existing Docker network in
    /// addition to the project's default network, so services from separate
    /// compose projects reach each other by service name.
    #[must_use]
    pub fn with_external_network(mut self, name: impl Into<String>) -> Self {
        self.external_network = Some(name.into());
        self
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

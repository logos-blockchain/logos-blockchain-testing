use super::DeploymentDescriptor;

#[derive(Clone, Debug)]
pub struct ClusterTopology {
    pub node_count: usize,
    node_indices: Vec<usize>,
}

impl ClusterTopology {
    #[must_use]
    pub fn new(node_count: usize) -> Self {
        Self {
            node_count,
            node_indices: (0..node_count).collect(),
        }
    }

    #[must_use]
    pub fn node_indices(&self) -> &[usize] {
        &self.node_indices
    }
}

impl DeploymentDescriptor for ClusterTopology {
    fn node_count(&self) -> usize {
        self.node_count
    }
}

#[doc(hidden)]
pub type NodeCountTopology = ClusterTopology;

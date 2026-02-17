use std::sync::Arc;

use super::DeploymentDescriptor;

#[derive(Clone)]
pub struct NodePlan<NodeConfig> {
    pub index: usize,
    pub id: [u8; 32],
    pub general: NodeConfig,
}

impl<NodeConfig> NodePlan<NodeConfig> {
    #[must_use]
    pub const fn index(&self) -> usize {
        self.index
    }
}

#[derive(Clone)]
pub struct DeploymentPlan<TopologyShape, NodeConfig> {
    pub config: TopologyShape,
    pub plans: Vec<NodePlan<NodeConfig>>,
}

impl<TopologyShape, NodeConfig> DeploymentPlan<TopologyShape, NodeConfig> {
    #[must_use]
    pub fn new(config: TopologyShape, plans: Vec<NodePlan<NodeConfig>>) -> Self {
        Self { config, plans }
    }

    #[must_use]
    pub const fn config(&self) -> &TopologyShape {
        &self.config
    }

    #[must_use]
    pub fn nodes(&self) -> &[NodePlan<NodeConfig>] {
        &self.plans
    }

    pub fn iter(&self) -> impl Iterator<Item = &NodePlan<NodeConfig>> {
        self.plans.iter()
    }
}

impl<TopologyShape, NodeConfig> DeploymentDescriptor for DeploymentPlan<TopologyShape, NodeConfig>
where
    TopologyShape: Send + Sync + 'static,
    NodeConfig: Send + Sync + 'static,
{
    fn node_count(&self) -> usize {
        self.plans.len()
    }
}

pub struct RuntimeTopology<Node> {
    nodes: Vec<Node>,
}

impl<Node> RuntimeTopology<Node> {
    #[must_use]
    pub fn from_nodes(nodes: Vec<Node>) -> Self {
        Self { nodes }
    }

    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    #[must_use]
    pub fn into_nodes(self) -> Vec<Node> {
        self.nodes
    }
}

impl<Node> Clone for RuntimeTopology<Node>
where
    Node: Clone,
{
    fn clone(&self) -> Self {
        Self {
            nodes: self.nodes.clone(),
        }
    }
}

impl<Node> From<Vec<Node>> for RuntimeTopology<Node> {
    fn from(nodes: Vec<Node>) -> Self {
        Self::from_nodes(nodes)
    }
}

pub type SharedTopology<T> = Arc<T>;

use std::error::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeploymentSeed([u8; 32]);

impl DeploymentSeed {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

pub type DynTopologyError = Box<dyn Error + Send + Sync + 'static>;

pub mod generated;
pub mod shape;
pub mod simple;
pub use generated::{DeploymentPlan, RuntimeTopology, SharedTopology};
pub use shape::TopologyShapeBuilder;
pub use simple::{ClusterTopology, NodeCountTopology};

pub trait DeploymentDescriptor: Send + Sync {
    fn node_count(&self) -> usize;
}

pub trait DeploymentProvider<D>: Send + Sync
where
    D: DeploymentDescriptor,
{
    fn build(&self, seed: Option<&DeploymentSeed>) -> Result<D, DynTopologyError>;
}

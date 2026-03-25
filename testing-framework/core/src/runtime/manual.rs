use async_trait::async_trait;

use crate::scenario::{Application, ClusterControlProfile, ClusterWaitHandle, NodeControlHandle};

/// Interface for imperative, deployer-backed manual clusters.
#[async_trait]
pub trait ManualClusterHandle<E: Application>: NodeControlHandle<E> + ClusterWaitHandle<E> {
    fn cluster_control_profile(&self) -> ClusterControlProfile {
        ClusterControlProfile::ManualControlled
    }
}

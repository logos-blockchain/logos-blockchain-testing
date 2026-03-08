use async_trait::async_trait;

use crate::scenario::{Application, ClusterWaitHandle, NodeControlHandle};

/// Interface for imperative, deployer-backed manual clusters.
#[async_trait]
pub trait ManualClusterHandle<E: Application>: NodeControlHandle<E> + ClusterWaitHandle<E> {}

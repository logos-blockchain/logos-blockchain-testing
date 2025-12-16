use async_trait::async_trait;
use reqwest::Url;

use super::DynError;

/// Marker type used by scenario builders to request node control support.
#[derive(Clone, Copy, Debug, Default)]
pub struct NodeControlCapability;

/// Optional observability settings attached to a scenario.
///
/// Runners may use this to decide whether to provision in-cluster Prometheus or
/// reuse an existing endpoint.
#[derive(Clone, Debug, Default)]
pub struct ObservabilityCapability {
    pub external_prometheus: Option<Url>,
}

/// Trait implemented by scenario capability markers to signal whether node
/// control is required.
pub trait RequiresNodeControl {
    const REQUIRED: bool;
}

impl RequiresNodeControl for () {
    const REQUIRED: bool = false;
}

impl RequiresNodeControl for NodeControlCapability {
    const REQUIRED: bool = true;
}

impl RequiresNodeControl for ObservabilityCapability {
    const REQUIRED: bool = false;
}

/// Interface exposed by runners that can restart nodes at runtime.
#[async_trait]
pub trait NodeControlHandle: Send + Sync {
    async fn restart_validator(&self, index: usize) -> Result<(), DynError>;

    async fn restart_executor(&self, index: usize) -> Result<(), DynError>;
}

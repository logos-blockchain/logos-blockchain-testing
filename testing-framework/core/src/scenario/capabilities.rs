use async_trait::async_trait;
use reqwest::Url;

use super::DynError;
use crate::{nodes::ApiClient, topology::generation::NodeRole};

/// Marker type used by scenario builders to request node control support.
#[derive(Clone, Copy, Debug, Default)]
pub struct NodeControlCapability;

/// Optional observability settings attached to a scenario.
#[derive(Clone, Debug, Default)]
pub struct ObservabilityCapability {
    /// Prometheus-compatible base URL used by the *runner process* to query
    /// metrics (commonly a localhost port-forward, but can be any reachable
    /// endpoint).
    pub metrics_query_url: Option<Url>,
    /// Full OTLP HTTP metrics ingest endpoint used by *nodes* to export metrics
    /// (backend-specific host and path).
    pub metrics_otlp_ingest_url: Option<Url>,
    /// Optional Grafana base URL for printing/logging (human access).
    pub grafana_url: Option<Url>,
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

    async fn start_validator(&self, _name: &str) -> Result<StartedNode, DynError> {
        Err("start_validator not supported by this deployer".into())
    }

    async fn start_executor(&self, _name: &str) -> Result<StartedNode, DynError> {
        Err("start_executor not supported by this deployer".into())
    }
}

#[derive(Clone)]
pub struct StartedNode {
    pub name: String,
    pub role: NodeRole,
    pub api: ApiClient,
}

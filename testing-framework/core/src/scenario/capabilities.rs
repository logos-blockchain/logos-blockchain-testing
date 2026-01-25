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

/// Peer selection strategy for dynamically started nodes.
#[derive(Clone, Debug)]
pub enum PeerSelection {
    /// Use the topology default (star/chain/full).
    DefaultLayout,
    /// Start without any initial peers.
    None,
    /// Connect to the named peers.
    Named(Vec<String>),
}

/// Options for dynamically starting a node.
#[derive(Clone, Debug)]
pub struct StartNodeOptions {
    /// How to select initial peers on startup.
    pub peers: PeerSelection,
}

impl Default for StartNodeOptions {
    fn default() -> Self {
        Self {
            peers: PeerSelection::DefaultLayout,
        }
    }
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

    async fn start_validator(&self, _name: &str) -> Result<StartedNode, DynError> {
        Err("start_validator not supported by this deployer".into())
    }

    async fn start_validator_with(
        &self,
        _name: &str,
        _options: StartNodeOptions,
    ) -> Result<StartedNode, DynError> {
        Err("start_validator_with not supported by this deployer".into())
    }

    fn node_client(&self, _name: &str) -> Option<ApiClient> {
        None
    }
}

#[derive(Clone)]
pub struct StartedNode {
    pub name: String,
    pub role: NodeRole,
    pub api: ApiClient,
}

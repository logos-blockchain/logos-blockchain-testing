use std::sync::Arc;

use reqwest::Url;

use super::DynError;
use crate::{nodes::ApiClient, topology::config::NodeConfigPatch};

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
#[derive(Clone)]
pub struct StartNodeOptions {
    /// How to select initial peers on startup.
    pub peers: PeerSelection,
    /// Optional node config patch applied before spawn.
    pub config_patch: Option<NodeConfigPatch>,
}

impl Default for StartNodeOptions {
    fn default() -> Self {
        Self {
            peers: PeerSelection::DefaultLayout,
            config_patch: None,
        }
    }
}

impl StartNodeOptions {
    pub fn create_patch<F>(mut self, f: F) -> Self
    where
        F: Fn(nomos_node::Config) -> Result<nomos_node::Config, DynError> + Send + Sync + 'static,
    {
        self.config_patch = Some(Arc::new(f));
        self
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

#[derive(Clone)]
pub struct StartedNode {
    pub name: String,
    pub api: ApiClient,
}

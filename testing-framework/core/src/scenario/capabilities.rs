use std::{fmt, marker::PhantomData, path::PathBuf, sync::Arc};

use reqwest::Url;

use super::{Application, DynError};

/// Marker enabling node control support.
#[derive(Clone, Copy, Debug, Default)]
pub struct NodeControlCapability;

/// Observability settings attached to a scenario.
#[derive(Clone, Debug, Default)]
pub struct ObservabilityCapability {
    /// Base URL used by the runner to query Prometheus metrics.
    pub metrics_query_url: Option<Url>,
    /// OTLP HTTP endpoint used by nodes to export metrics.
    pub metrics_otlp_ingest_url: Option<Url>,
    /// Optional Grafana URL for logs/output.
    pub grafana_url: Option<Url>,
}

/// Peer selection strategy for dynamically started nodes.
#[derive(Clone, Debug)]
pub enum PeerSelection {
    /// Use topology defaults.
    DefaultLayout,
    /// Start without initial peers.
    None,
    /// Connect to named peers.
    Named(Vec<String>),
}

/// Options for dynamically starting a node.
#[derive(Clone)]
pub struct StartNodeOptions<E: Application> {
    /// How to select initial peers on startup.
    pub peers: PeerSelection,
    /// Optional backend-specific initial config override.
    pub config_override: Option<E::NodeConfig>,
    /// Optional patch callback applied to generated node config before spawn.
    pub config_patch:
        Option<Arc<dyn Fn(E::NodeConfig) -> Result<E::NodeConfig, DynError> + Send + Sync>>,
    /// Optional persistent working directory for this node process.
    pub persist_dir: Option<PathBuf>,
    _phantom: PhantomData<E>,
}

impl<E: Application> fmt::Debug for StartNodeOptions<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StartNodeOptions")
            .field("peers", &self.peers)
            .field("config_override", &self.config_override.is_some())
            .field("config_patch", &self.config_patch.is_some())
            .field("persist_dir", &self.persist_dir)
            .finish()
    }
}

impl<E: Application> Default for StartNodeOptions<E> {
    fn default() -> Self {
        Self {
            peers: PeerSelection::DefaultLayout,
            config_override: None,
            config_patch: None,
            persist_dir: None,
            _phantom: PhantomData,
        }
    }
}

impl<E: Application> StartNodeOptions<E> {
    #[must_use]
    pub fn with_peers(mut self, peers: PeerSelection) -> Self {
        self.peers = peers;
        self
    }

    #[must_use]
    pub fn with_config_override(mut self, config_override: E::NodeConfig) -> Self {
        self.config_override = Some(config_override);
        self
    }

    #[must_use]
    pub fn create_patch(
        mut self,
        config_patch: impl Fn(E::NodeConfig) -> Result<E::NodeConfig, DynError> + Send + Sync + 'static,
    ) -> Self {
        self.config_patch = Some(Arc::new(config_patch));
        self
    }

    #[must_use]
    pub fn with_persist_dir(mut self, persist_dir: PathBuf) -> Self {
        self.persist_dir = Some(persist_dir);
        self
    }
}

/// Indicates whether a capability requires node control.
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
pub struct StartedNode<E: Application> {
    pub name: String,
    pub client: E::NodeClient,
}

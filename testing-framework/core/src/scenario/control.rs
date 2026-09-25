use std::{fmt, marker::PhantomData, path::PathBuf, sync::Arc, time::Duration};

use async_trait::async_trait;

use crate::scenario::{Application, DynError, NodeAccess};

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

/// Implementation-independent options for dynamically starting a node.
#[derive(Clone, Debug, Default)]
pub struct NodeLaunchOptions {
    /// How to select initial peers on startup.
    pub peers: Option<PeerSelection>,
    /// Optional persistent working directory for this node process.
    pub persist_dir: Option<PathBuf>,
    /// Optional directory whose contents should seed the node working dir.
    pub snapshot_dir: Option<PathBuf>,
    /// Extra process arguments appended on launch.
    pub args: Vec<String>,
    /// Runtime policy for this node launch.
    pub runtime: NodeRuntimeOptions,
}

/// Options for dynamically starting a node with typed configuration changes.
#[derive(Clone)]
pub struct StartNodeOptions<E: Application> {
    pub common: NodeLaunchOptions,
    /// Optional backend-specific initial config override.
    pub config_override: Option<E::NodeConfig>,
    /// Optional patch callback applied to generated node config before spawn.
    pub config_patch:
        Option<Arc<dyn Fn(E::NodeConfig) -> Result<E::NodeConfig, DynError> + Send + Sync>>,
    _phantom: PhantomData<E>,
}

/// Runtime supervision options for a node process.
#[derive(Clone, Copy, Debug, Default)]
pub struct NodeRuntimeOptions {
    /// Optional readiness/start timeout override for this node.
    pub start_timeout: Option<Duration>,
}

impl NodeLaunchOptions {
    #[must_use]
    pub fn with_peers(mut self, peers: PeerSelection) -> Self {
        self.peers = Some(peers);
        self
    }

    #[must_use]
    pub fn with_persist_dir(mut self, persist_dir: PathBuf) -> Self {
        self.persist_dir = Some(persist_dir);
        self
    }

    #[must_use]
    pub fn with_snapshot_dir(mut self, snapshot_dir: PathBuf) -> Self {
        self.snapshot_dir = Some(snapshot_dir);
        self
    }

    #[must_use]
    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn with_runtime(mut self, runtime: NodeRuntimeOptions) -> Self {
        self.runtime = runtime;
        self
    }

    #[must_use]
    pub fn with_start_timeout(mut self, start_timeout: Duration) -> Self {
        self.runtime.start_timeout = Some(start_timeout);
        self
    }
}

impl<E: Application> fmt::Debug for StartNodeOptions<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StartNodeOptions")
            .field("common", &self.common)
            .field("config_override", &self.config_override.is_some())
            .field("config_patch", &self.config_patch.is_some())
            .finish()
    }
}

impl<E: Application> Default for StartNodeOptions<E> {
    fn default() -> Self {
        NodeLaunchOptions::default().into()
    }
}

impl<E: Application> From<NodeLaunchOptions> for StartNodeOptions<E> {
    fn from(common: NodeLaunchOptions) -> Self {
        Self {
            common,
            config_override: None,
            config_patch: None,
            _phantom: PhantomData,
        }
    }
}

impl<E: Application> StartNodeOptions<E> {
    #[must_use]
    pub fn with_peers(mut self, peers: PeerSelection) -> Self {
        self.common = self.common.with_peers(peers);
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
        self.common = self.common.with_persist_dir(persist_dir);
        self
    }

    #[must_use]
    pub fn with_snapshot_dir(mut self, snapshot_dir: PathBuf) -> Self {
        self.common = self.common.with_snapshot_dir(snapshot_dir);
        self
    }

    #[must_use]
    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.common = self.common.with_args(args);
        self
    }

    #[must_use]
    pub fn with_runtime(mut self, runtime: NodeRuntimeOptions) -> Self {
        self.common = self.common.with_runtime(runtime);
        self
    }

    #[must_use]
    pub fn with_start_timeout(mut self, start_timeout: Duration) -> Self {
        self.common = self.common.with_start_timeout(start_timeout);
        self
    }
}

/// Node started dynamically through a cluster's node control surface.
#[derive(Clone)]
pub struct StartedNode<E: Application> {
    pub name: String,
    pub client: E::NodeClient,
}

/// Runtime identity and endpoints of a node started through common control.
#[derive(Clone, Debug)]
pub struct StartedNodeAccess {
    pub name: String,
    pub access: NodeAccess,
}

/// Deployer-agnostic control surface for runtime node operations.
#[async_trait]
pub trait NodeControl: Send + Sync {
    async fn restart_node(&self, name: &str) -> Result<(), DynError> {
        self.restart_node_with(name, NodeLaunchOptions::default())
            .await
    }

    async fn restart_node_with(
        &self,
        _name: &str,
        _options: NodeLaunchOptions,
    ) -> Result<(), DynError> {
        Err("restart_node_with not supported by this deployer".into())
    }

    async fn start_node(&self, name: &str) -> Result<StartedNodeAccess, DynError> {
        self.start_node_with(name, NodeLaunchOptions::default())
            .await
    }

    async fn start_node_with(
        &self,
        _name: &str,
        _options: NodeLaunchOptions,
    ) -> Result<StartedNodeAccess, DynError> {
        Err("start_node_with not supported by this deployer".into())
    }

    async fn stop_node(&self, _name: &str) -> Result<(), DynError> {
        Err("stop_node not supported by this deployer".into())
    }

    async fn wait_node_ready(&self, _name: &str) -> Result<(), DynError> {
        Err("wait_node_ready not supported by this deployer".into())
    }

    /// Returns runner-reachable endpoints; this does not check node readiness.
    async fn node_access(&self, _name: &str) -> Result<NodeAccess, DynError> {
        Err("node access not available for this node".into())
    }

    /// Returns the real names of the nodes this handle controls.
    ///
    /// Backends that know their node inventory override this so consumers can
    /// target actual service names. The default empty result makes callers
    /// fall back to convention-based names.
    fn node_names(&self) -> Vec<String> {
        Vec::new()
    }

    fn node_pid(&self, _name: &str) -> Option<u32> {
        None
    }
}

/// Typed configuration and client access for a node control handle.
#[async_trait]
pub trait NodeControlHandle<E: Application>: NodeControl {
    async fn start_node_with_config(
        &self,
        _name: &str,
        _options: StartNodeOptions<E>,
    ) -> Result<StartedNode<E>, DynError> {
        Err("start_node_with not supported by this deployer".into())
    }

    async fn restart_node_with_config(
        &self,
        _name: &str,
        _options: StartNodeOptions<E>,
    ) -> Result<(), DynError> {
        Err("restart_node_with not supported by this deployer".into())
    }

    fn node_client(&self, _name: &str) -> Option<E::NodeClient> {
        None
    }
}

/// Deployer-agnostic wait surface for cluster readiness checks.
#[async_trait]
pub trait ClusterWaitHandle: Send + Sync {
    async fn wait_network_ready(&self) -> Result<(), DynError> {
        Err("wait_network_ready not supported by this deployer".into())
    }
}

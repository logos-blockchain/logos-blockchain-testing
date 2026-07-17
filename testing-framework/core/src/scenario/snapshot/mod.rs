use std::{
    collections::{BTreeMap, BTreeSet},
    marker::PhantomData,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::scenario::{
    Application, DynError, NodeClients, PreparedRuntimeExtension, RuntimeExtensionFactory,
};

mod store;

pub use store::SnapshotStore;

/// Logical name of a snapshot inside the configured snapshot store.
pub type SnapshotName = String;
/// Stable identifier for an application-specific snapshot artifact provider.
pub type SnapshotArtifactProviderId = String;
/// Stable node identifier used as the key for per-node state in a snapshot.
pub type SnapshotNodeName = String;

/// Selects which snapshot parts should participate in a save or load operation.
///
/// Node selection is intentionally not part of this type. A snapshot can
/// contain state for many nodes, keyed by node name in
/// [`SnapshotManifest::node_state`]. When a caller needs one node's local state
/// as a startup directory, it uses [`NodeStateSource`] instead.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SnapshotSpec {
    include_node_state: bool,
    providers: BTreeSet<SnapshotArtifactProviderId>,
}

impl SnapshotSpec {
    /// Include only deployer-managed node state.
    #[must_use]
    pub fn node_state() -> Self {
        Self {
            include_node_state: true,
            providers: BTreeSet::new(),
        }
    }

    /// Include only artifacts from one provider.
    #[must_use]
    pub fn provider(provider: impl Into<SnapshotArtifactProviderId>) -> Self {
        Self::providers([provider])
    }

    /// Include only artifacts from the given providers.
    #[must_use]
    pub fn providers(
        providers: impl IntoIterator<Item = impl Into<SnapshotArtifactProviderId>>,
    ) -> Self {
        Self {
            include_node_state: false,
            providers: providers.into_iter().map(Into::into).collect(),
        }
    }

    /// Return a copy of this spec without deployer-managed node state.
    #[must_use]
    pub fn without_node_state(mut self) -> Self {
        self.include_node_state = false;
        self
    }

    /// Return a copy of this spec with deployer-managed node state included.
    #[must_use]
    pub fn with_node_state(mut self) -> Self {
        self.include_node_state = true;
        self
    }

    /// Return a copy of this spec with one additional artifact provider.
    #[must_use]
    pub fn with_provider(mut self, provider: impl Into<SnapshotArtifactProviderId>) -> Self {
        self.providers.insert(provider.into());
        self
    }

    /// Return a copy of this spec with the given artifact providers.
    #[must_use]
    pub fn with_providers(
        mut self,
        providers: impl IntoIterator<Item = impl Into<SnapshotArtifactProviderId>>,
    ) -> Self {
        for provider in providers {
            self = self.with_provider(provider);
        }
        self
    }

    /// Whether this spec includes deployer-managed node state.
    #[must_use]
    pub fn includes_node_state(&self) -> bool {
        self.include_node_state
    }

    /// Whether this spec includes artifacts from the given provider id.
    #[must_use]
    pub fn includes_provider(&self, id: &str) -> bool {
        self.providers.contains(id)
    }
}

impl Default for SnapshotSpec {
    fn default() -> Self {
        Self::node_state()
    }
}

/// Source for one node's local state when preparing or restoring a runtime dir.
///
/// This type is deliberately narrower than a whole snapshot reference.
/// Provider-owned artifacts and full snapshot loads are addressed by snapshot
/// name through [`SnapshotHandle::load`]. Node startup/restore needs a concrete
/// directory, so a snapshot source must name both the snapshot and the source
/// node.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NodeStateSource {
    /// Read state for one node from the configured snapshot store.
    Snapshot {
        /// Snapshot name under the store root.
        snapshot: SnapshotName,
        /// Node key inside that snapshot.
        node: SnapshotNodeName,
    },
    /// Read state directly from an existing node data directory.
    ExternalDirectory(PathBuf),
}

impl NodeStateSource {
    /// Use a node's saved state inside a named snapshot.
    #[must_use]
    pub fn snapshot_node(
        snapshot: impl Into<SnapshotName>,
        node: impl Into<SnapshotNodeName>,
    ) -> Self {
        Self::Snapshot {
            snapshot: snapshot.into(),
            node: node.into(),
        }
    }

    /// Use an existing node data directory outside the snapshot store.
    #[must_use]
    pub fn external_directory(path: impl Into<PathBuf>) -> Self {
        Self::ExternalDirectory(path.into())
    }
}

/// Versioned JSON artifact stored in a snapshot manifest.
///
/// Core stores artifacts opaquely. The producer owns the schema encoded in
/// `payload`, and `version` should be bumped when that schema changes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SnapshotArtifact {
    /// Producer-defined payload schema version.
    pub version: u32,
    /// Small diagnostic/indexing data that can be inspected without decoding
    /// the full payload.
    pub metadata: serde_json::Value,
    /// Producer-defined artifact payload.
    pub payload: serde_json::Value,
}

impl SnapshotArtifact {
    /// Create a snapshot artifact from explicit version, metadata, and payload.
    #[must_use]
    pub fn new(version: u32, metadata: serde_json::Value, payload: serde_json::Value) -> Self {
        Self {
            version,
            metadata,
            payload,
        }
    }
}

/// Manifest file describing the contents of one snapshot.
///
/// Node state entries are keyed by node name. Provider-owned artifacts are
/// keyed by provider id. The actual node files live beside the manifest in
/// deployer/application-specific directories.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SnapshotManifest {
    /// Snapshot manifest format version.
    pub format_version: u32,
    /// Snapshot name under the configured snapshot store.
    pub name: SnapshotName,
    /// Creation timestamp in Unix milliseconds.
    pub created_unix_millis: u128,
    /// Per-node state artifacts saved by the deployer/node-state adapter.
    pub node_state: BTreeMap<SnapshotNodeName, SnapshotArtifact>,
    /// Application-specific artifacts keyed by provider id.
    pub providers: BTreeMap<SnapshotArtifactProviderId, SnapshotArtifact>,
}

impl SnapshotManifest {
    const FORMAT_VERSION: u32 = 1;

    /// Create an empty manifest for a named snapshot.
    #[must_use]
    pub fn new(name: impl Into<SnapshotName>) -> Self {
        Self {
            format_version: Self::FORMAT_VERSION,
            name: name.into(),
            created_unix_millis: unix_millis_now(),
            node_state: BTreeMap::new(),
            providers: BTreeMap::new(),
        }
    }
}

/// Runtime context passed to snapshot artifact providers.
pub struct SnapshotContext<E: Application> {
    deployment: E::Deployment,
    node_clients: NodeClients<E>,
}

impl<E: Application> Clone for SnapshotContext<E> {
    fn clone(&self) -> Self {
        Self {
            deployment: self.deployment.clone(),
            node_clients: self.node_clients.clone(),
        }
    }
}

impl<E: Application> SnapshotContext<E> {
    /// Create snapshot artifact provider context from the active deployment and
    /// clients.
    #[must_use]
    pub fn new(deployment: E::Deployment, node_clients: NodeClients<E>) -> Self {
        Self {
            deployment,
            node_clients,
        }
    }

    /// Active deployment for the scenario.
    #[must_use]
    pub const fn deployment(&self) -> &E::Deployment {
        &self.deployment
    }

    /// Node clients available in the active scenario.
    #[must_use]
    pub const fn node_clients(&self) -> &NodeClients<E> {
        &self.node_clients
    }
}

/// Application-specific snapshot artifact provider.
///
/// Providers save and load artifacts that are not owned by the deployer/node
/// runtime itself, such as test harness wallet state. Implementations should be
/// deterministic and treat `spec` as a filter for optional sub-parts they own.
#[async_trait]
pub trait SnapshotArtifactProvider<E: Application>: Send + Sync {
    /// Stable id used as the key in [`SnapshotManifest::providers`].
    fn id(&self) -> &'static str;

    /// Save this provider's artifact for the active snapshot operation.
    ///
    /// Returning `Ok(None)` means this provider has no state to write.
    async fn save(
        &self,
        context: &SnapshotContext<E>,
        spec: &SnapshotSpec,
    ) -> Result<Option<SnapshotArtifact>, DynError>;

    /// Load this provider's artifact from the active snapshot operation.
    async fn load(
        &self,
        context: &SnapshotContext<E>,
        artifact: &SnapshotArtifact,
        spec: &SnapshotSpec,
    ) -> Result<(), DynError>;
}

/// Deployer-owned adapter for saving and loading node runtime state.
///
/// Core does not know which files make up a node's durable state. The adapter
/// bridges that deployer/application detail to the generic snapshot manifest.
#[async_trait]
pub trait SnapshotNodeStateAdapter<E: Application>: Send + Sync {
    /// Save node state for the active deployment into the named snapshot.
    async fn save_node_state(
        &self,
        snapshot: &str,
        spec: &SnapshotSpec,
    ) -> Result<BTreeMap<SnapshotNodeName, SnapshotArtifact>, DynError>;

    /// Load node state from a named snapshot manifest into the active
    /// deployment.
    async fn load_node_state(
        &self,
        snapshot: &str,
        manifest: &SnapshotManifest,
        spec: &SnapshotSpec,
    ) -> Result<(), DynError>;

    /// Resolve one node's state to a local startup directory.
    ///
    /// Deployers that cannot expose a local startup directory may keep the
    /// default unsupported implementation.
    async fn prepare_node_state_source_dir(
        &self,
        source: &NodeStateSource,
    ) -> Result<PathBuf, DynError> {
        Err(format!(
            "preparing node state source '{source:?}' as a startup directory is not supported by this deployer"
        )
        .into())
    }

    /// Persist a snapshot manifest.
    async fn write_manifest(&self, manifest: &SnapshotManifest) -> Result<(), DynError>;

    /// Read a snapshot manifest by name.
    async fn read_manifest(&self, snapshot: &str) -> Result<SnapshotManifest, DynError>;
}

/// Prepared snapshot handle for an active scenario.
pub struct SnapshotHandle<E: Application> {
    node_state: Arc<dyn SnapshotNodeStateAdapter<E>>,
    context: SnapshotContext<E>,
    providers: Vec<Arc<dyn SnapshotArtifactProvider<E>>>,
    _phantom: PhantomData<E>,
}

/// Factory that installs snapshot support into a scenario runtime.
pub struct SnapshotFactory<E: Application> {
    node_state: Arc<dyn SnapshotNodeStateAdapter<E>>,
    providers: Vec<Arc<dyn SnapshotArtifactProvider<E>>>,
}

impl<E: Application> SnapshotFactory<E> {
    /// Create a snapshot factory with a node-state adapter and no artifact
    /// providers.
    #[must_use]
    pub fn new(node_state: Arc<dyn SnapshotNodeStateAdapter<E>>) -> Self {
        Self {
            node_state,
            providers: Vec::new(),
        }
    }

    /// Register one application-specific snapshot artifact provider.
    #[must_use]
    pub fn with_provider(mut self, provider: Arc<dyn SnapshotArtifactProvider<E>>) -> Self {
        self.providers.push(provider);
        self
    }

    /// Register multiple application-specific snapshot artifact providers.
    #[must_use]
    pub fn with_providers(
        mut self,
        providers: impl IntoIterator<Item = Arc<dyn SnapshotArtifactProvider<E>>>,
    ) -> Self {
        self.providers.extend(providers);
        self
    }
}

#[async_trait]
impl<E: Application> RuntimeExtensionFactory<E> for SnapshotFactory<E> {
    async fn prepare(
        &self,
        deployment: &E::Deployment,
        node_clients: NodeClients<E>,
    ) -> Result<PreparedRuntimeExtension, DynError> {
        Ok(PreparedRuntimeExtension::new(SnapshotHandle::new(
            Arc::clone(&self.node_state),
            SnapshotContext::new(deployment.clone(), node_clients),
            self.providers.clone(),
        )))
    }
}

impl<E: Application> Clone for SnapshotHandle<E> {
    fn clone(&self) -> Self {
        Self {
            node_state: Arc::clone(&self.node_state),
            context: self.context.clone(),
            providers: self.providers.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<E: Application> SnapshotHandle<E> {
    /// Create a prepared snapshot handle.
    #[must_use]
    pub fn new(
        node_state: Arc<dyn SnapshotNodeStateAdapter<E>>,
        context: SnapshotContext<E>,
        providers: Vec<Arc<dyn SnapshotArtifactProvider<E>>>,
    ) -> Self {
        Self {
            node_state,
            context,
            providers,
            _phantom: PhantomData,
        }
    }

    /// Save a named snapshot according to `spec`.
    ///
    /// This writes a new manifest containing the requested node-state and
    /// provider-owned artifacts. Lifecycle concerns such as stopping nodes
    /// before saving are intentionally owned by the caller/deployer, not
    /// this method.
    pub async fn save(
        &self,
        name: impl Into<SnapshotName>,
        spec: SnapshotSpec,
    ) -> Result<SnapshotName, DynError> {
        let snapshot = name.into();
        let mut manifest = SnapshotManifest::new(snapshot.clone());

        if spec.includes_node_state() {
            manifest.node_state = self.node_state.save_node_state(&snapshot, &spec).await?;
        }

        for provider in &self.providers {
            if !spec.includes_provider(provider.id()) {
                continue;
            }

            if let Some(artifact) = provider.save(&self.context, &spec).await? {
                manifest
                    .providers
                    .insert(provider.id().to_owned(), artifact);
            }
        }

        self.node_state.write_manifest(&manifest).await?;

        Ok(snapshot)
    }

    /// Load requested snapshot parts into the active scenario runtime.
    ///
    /// Loading node state here delegates to the deployer's node-state adapter.
    /// Preparing a local node startup directory is a separate operation handled
    /// by [`Self::prepare_node_state_source_dir`].
    pub async fn load(
        &self,
        snapshot: impl AsRef<str>,
        spec: SnapshotSpec,
    ) -> Result<(), DynError> {
        let snapshot = snapshot.as_ref();
        let manifest = self.node_state.read_manifest(snapshot).await?;

        if spec.includes_node_state() {
            self.node_state
                .load_node_state(snapshot, &manifest, &spec)
                .await?;
        }

        for provider in &self.providers {
            if !spec.includes_provider(provider.id()) {
                continue;
            }

            let Some(artifact) = manifest.providers.get(provider.id()) else {
                return Err(format!(
                    "snapshot '{snapshot}' does not contain provider '{}'",
                    provider.id()
                )
                .into());
            };

            provider.load(&self.context, artifact, &spec).await?;
        }

        Ok(())
    }

    /// Resolve one node's saved state to a local startup directory.
    ///
    /// This is used when a node process should be started from existing local
    /// state. It does not load provider-owned artifacts.
    pub async fn prepare_node_state_source_dir(
        &self,
        source: NodeStateSource,
    ) -> Result<PathBuf, DynError> {
        self.node_state.prepare_node_state_source_dir(&source).await
    }
}

fn unix_millis_now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

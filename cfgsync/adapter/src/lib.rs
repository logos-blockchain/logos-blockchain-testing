use std::{collections::HashMap, error::Error, sync::Mutex};

use cfgsync_artifacts::ArtifactFile;
use cfgsync_core::{
    CfgSyncErrorResponse, CfgSyncPayload, ConfigProvider, NodeRegistration, RegistrationResponse,
    RepoResponse,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Type-erased cfgsync adapter error used to preserve source context.
pub type DynCfgsyncError = Box<dyn Error + Send + Sync + 'static>;

/// Per-node artifact payload served by cfgsync for one registered node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeArtifacts {
    /// Stable node identifier resolved by the adapter.
    pub identifier: String,
    /// Files served to the node after cfgsync registration.
    pub files: Vec<ArtifactFile>,
}

/// Materialized artifact files for a single registered node.
#[derive(Debug, Clone, Default)]
pub struct ArtifactSet {
    files: Vec<ArtifactFile>,
}

/// Immutable view of registrations currently known to cfgsync.
#[derive(Debug, Clone, Default)]
pub struct RegistrationSet {
    registrations: Vec<NodeRegistration>,
}

impl RegistrationSet {
    #[must_use]
    pub fn new(registrations: Vec<NodeRegistration>) -> Self {
        Self { registrations }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.registrations.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.registrations.is_empty()
    }

    #[must_use]
    pub fn iter(&self) -> impl Iterator<Item = &NodeRegistration> {
        self.registrations.iter()
    }

    #[must_use]
    pub fn get(&self, identifier: &str) -> Option<&NodeRegistration> {
        self.registrations
            .iter()
            .find(|registration| registration.identifier == identifier)
    }
}

impl ArtifactSet {
    #[must_use]
    pub fn new(files: Vec<ArtifactFile>) -> Self {
        Self { files }
    }

    #[must_use]
    pub fn files(&self) -> &[ArtifactFile] {
        &self.files
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// Artifact payloads indexed by stable node identifier.
#[derive(Debug, Clone, Default)]
pub struct NodeArtifactsCatalog {
    nodes: HashMap<String, NodeArtifacts>,
}

impl NodeArtifactsCatalog {
    #[must_use]
    pub fn new(nodes: Vec<NodeArtifacts>) -> Self {
        let nodes = nodes
            .into_iter()
            .map(|node| (node.identifier.clone(), node))
            .collect();

        Self { nodes }
    }

    #[must_use]
    pub fn resolve(&self, identifier: &str) -> Option<&NodeArtifacts> {
        self.nodes.get(identifier)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    #[must_use]
    pub fn into_nodes(self) -> Vec<NodeArtifacts> {
        self.nodes.into_values().collect()
    }

    #[doc(hidden)]
    #[must_use]
    pub fn into_configs(self) -> Vec<NodeArtifacts> {
        self.into_nodes()
    }
}

/// Adapter-side materialization contract for a single registered node.
pub trait NodeArtifactsMaterializer: Send + Sync {
    fn materialize(
        &self,
        registration: &NodeRegistration,
        registrations: &RegistrationSet,
    ) -> Result<Option<ArtifactSet>, DynCfgsyncError>;
}

/// Backward-compatible alias for the previous materializer trait name.
pub trait CfgsyncMaterializer: NodeArtifactsMaterializer {}

impl<T> CfgsyncMaterializer for T where T: NodeArtifactsMaterializer + ?Sized {}

/// Adapter contract for materializing a whole registration set into
/// per-node cfgsync artifacts.
pub trait RegistrationSetMaterializer: Send + Sync {
    fn materialize_snapshot(
        &self,
        registrations: &RegistrationSet,
    ) -> Result<Option<NodeArtifactsCatalog>, DynCfgsyncError>;
}

/// Backward-compatible alias for the previous snapshot materializer trait name.
pub trait CfgsyncSnapshotMaterializer: RegistrationSetMaterializer {}

impl<T> CfgsyncSnapshotMaterializer for T where T: RegistrationSetMaterializer + ?Sized {}

impl NodeArtifactsMaterializer for NodeArtifactsCatalog {
    fn materialize(
        &self,
        registration: &NodeRegistration,
        _registrations: &RegistrationSet,
    ) -> Result<Option<ArtifactSet>, DynCfgsyncError> {
        let artifacts = self
            .resolve(&registration.identifier)
            .map(build_node_artifacts_from_config);

        Ok(artifacts)
    }
}

impl RegistrationSetMaterializer for NodeArtifactsCatalog {
    fn materialize_snapshot(
        &self,
        _registrations: &RegistrationSet,
    ) -> Result<Option<NodeArtifactsCatalog>, DynCfgsyncError> {
        Ok(Some(self.clone()))
    }
}

/// Registration-aware provider backed by an adapter materializer.
pub struct RegistrationConfigProvider<M> {
    materializer: M,
    registrations: Mutex<HashMap<String, NodeRegistration>>,
}

impl<M> RegistrationConfigProvider<M> {
    #[must_use]
    pub fn new(materializer: M) -> Self {
        Self {
            materializer,
            registrations: Mutex::new(HashMap::new()),
        }
    }

    fn registration_for(&self, identifier: &str) -> Option<NodeRegistration> {
        let registrations = self
            .registrations
            .lock()
            .expect("cfgsync registration store should not be poisoned");

        registrations.get(identifier).cloned()
    }

    fn registration_set(&self) -> RegistrationSet {
        let registrations = self
            .registrations
            .lock()
            .expect("cfgsync registration store should not be poisoned");

        RegistrationSet::new(registrations.values().cloned().collect())
    }
}

/// Registration-aware provider backed by a snapshot materializer.
pub struct SnapshotConfigProvider<M> {
    materializer: M,
    registrations: Mutex<HashMap<String, NodeRegistration>>,
}

impl<M> SnapshotConfigProvider<M> {
    #[must_use]
    pub fn new(materializer: M) -> Self {
        Self {
            materializer,
            registrations: Mutex::new(HashMap::new()),
        }
    }

    fn registration_for(&self, identifier: &str) -> Option<NodeRegistration> {
        let registrations = self
            .registrations
            .lock()
            .expect("cfgsync registration store should not be poisoned");

        registrations.get(identifier).cloned()
    }

    fn registration_set(&self) -> RegistrationSet {
        let registrations = self
            .registrations
            .lock()
            .expect("cfgsync registration store should not be poisoned");

        RegistrationSet::new(registrations.values().cloned().collect())
    }
}

impl<M> ConfigProvider for SnapshotConfigProvider<M>
where
    M: RegistrationSetMaterializer,
{
    fn register(&self, registration: NodeRegistration) -> RegistrationResponse {
        let mut registrations = self
            .registrations
            .lock()
            .expect("cfgsync registration store should not be poisoned");
        registrations.insert(registration.identifier.clone(), registration);

        RegistrationResponse::Registered
    }

    fn resolve(&self, registration: &NodeRegistration) -> RepoResponse {
        let registration = match self.registration_for(&registration.identifier) {
            Some(registration) => registration,
            None => {
                return RepoResponse::Error(CfgSyncErrorResponse::not_ready(
                    &registration.identifier,
                ));
            }
        };

        let registrations = self.registration_set();
        let catalog = match self.materializer.materialize_snapshot(&registrations) {
            Ok(Some(catalog)) => catalog,
            Ok(None) => {
                return RepoResponse::Error(CfgSyncErrorResponse::not_ready(
                    &registration.identifier,
                ));
            }
            Err(error) => {
                return RepoResponse::Error(CfgSyncErrorResponse::internal(format!(
                    "failed to materialize config snapshot: {error}"
                )));
            }
        };

        match catalog.resolve(&registration.identifier) {
            Some(config) => RepoResponse::Config(CfgSyncPayload::from_files(config.files.clone())),
            None => RepoResponse::Error(CfgSyncErrorResponse::missing_config(
                &registration.identifier,
            )),
        }
    }
}

impl<M> ConfigProvider for RegistrationConfigProvider<M>
where
    M: NodeArtifactsMaterializer,
{
    fn register(&self, registration: NodeRegistration) -> RegistrationResponse {
        let mut registrations = self
            .registrations
            .lock()
            .expect("cfgsync registration store should not be poisoned");
        registrations.insert(registration.identifier.clone(), registration);

        RegistrationResponse::Registered
    }

    fn resolve(&self, registration: &NodeRegistration) -> RepoResponse {
        let registration = match self.registration_for(&registration.identifier) {
            Some(registration) => registration,
            None => {
                return RepoResponse::Error(CfgSyncErrorResponse::not_ready(
                    &registration.identifier,
                ));
            }
        };
        let registrations = self.registration_set();

        match self.materializer.materialize(&registration, &registrations) {
            Ok(Some(artifacts)) => {
                RepoResponse::Config(CfgSyncPayload::from_files(artifacts.files().to_vec()))
            }
            Ok(None) => {
                RepoResponse::Error(CfgSyncErrorResponse::not_ready(&registration.identifier))
            }
            Err(error) => RepoResponse::Error(CfgSyncErrorResponse::internal(format!(
                "failed to materialize config for host {}: {error}",
                registration.identifier
            ))),
        }
    }
}

/// Adapter contract for converting an application deployment model into
/// node-specific serialized config payloads.
pub trait CfgsyncEnv {
    type Deployment;
    type Node;
    type NodeConfig;
    type Error: Error + Send + Sync + 'static;

    fn nodes(deployment: &Self::Deployment) -> &[Self::Node];

    fn node_identifier(index: usize, node: &Self::Node) -> String;

    fn build_node_config(
        deployment: &Self::Deployment,
        node: &Self::Node,
    ) -> Result<Self::NodeConfig, Self::Error>;

    fn rewrite_for_hostnames(
        deployment: &Self::Deployment,
        node_index: usize,
        hostnames: &[String],
        config: &mut Self::NodeConfig,
    ) -> Result<(), Self::Error>;

    fn serialize_node_config(config: &Self::NodeConfig) -> Result<String, Self::Error>;
}

/// Preferred public name for application-side cfgsync integration.
pub trait DeploymentAdapter: CfgsyncEnv {}

impl<T> DeploymentAdapter for T where T: CfgsyncEnv + ?Sized {}

/// High-level failures while building adapter output for cfgsync.
#[derive(Debug, Error)]
pub enum BuildCfgsyncNodesError {
    #[error("cfgsync hostnames mismatch (nodes={nodes}, hostnames={hostnames})")]
    HostnameCountMismatch { nodes: usize, hostnames: usize },
    #[error("cfgsync adapter failed: {source}")]
    Adapter {
        #[source]
        source: DynCfgsyncError,
    },
}

fn adapter_error<E>(source: E) -> BuildCfgsyncNodesError
where
    E: Error + Send + Sync + 'static,
{
    BuildCfgsyncNodesError::Adapter {
        source: Box::new(source),
    }
}

/// Builds cfgsync node configs for a deployment by:
/// 1) validating hostname count,
/// 2) building each node config,
/// 3) rewriting host references,
/// 4) serializing each node payload.
pub fn build_cfgsync_node_configs<E: CfgsyncEnv>(
    deployment: &E::Deployment,
    hostnames: &[String],
) -> Result<Vec<CfgsyncNodeConfig>, BuildCfgsyncNodesError> {
    Ok(build_node_artifact_catalog::<E>(deployment, hostnames)?.into_nodes())
}

/// Builds cfgsync node configs and indexes them by stable identifier.
pub fn build_node_artifact_catalog<E: DeploymentAdapter>(
    deployment: &E::Deployment,
    hostnames: &[String],
) -> Result<NodeArtifactsCatalog, BuildCfgsyncNodesError> {
    let nodes = E::nodes(deployment);
    ensure_hostname_count(nodes.len(), hostnames.len())?;

    let mut output = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        output.push(build_node_entry::<E>(deployment, node, index, hostnames)?);
    }

    Ok(NodeArtifactsCatalog::new(output))
}

#[doc(hidden)]
pub fn build_cfgsync_node_catalog<E: CfgsyncEnv>(
    deployment: &E::Deployment,
    hostnames: &[String],
) -> Result<NodeArtifactsCatalog, BuildCfgsyncNodesError> {
    build_node_artifact_catalog::<E>(deployment, hostnames)
}

fn ensure_hostname_count(nodes: usize, hostnames: usize) -> Result<(), BuildCfgsyncNodesError> {
    if nodes != hostnames {
        return Err(BuildCfgsyncNodesError::HostnameCountMismatch { nodes, hostnames });
    }

    Ok(())
}

fn build_node_entry<E: DeploymentAdapter>(
    deployment: &E::Deployment,
    node: &E::Node,
    index: usize,
    hostnames: &[String],
) -> Result<NodeArtifacts, BuildCfgsyncNodesError> {
    let node_config = build_rewritten_node_config::<E>(deployment, node, index, hostnames)?;
    let config_yaml = E::serialize_node_config(&node_config).map_err(adapter_error)?;

    Ok(NodeArtifacts {
        identifier: E::node_identifier(index, node),
        files: vec![ArtifactFile::new("/config.yaml", &config_yaml)],
    })
}

fn build_rewritten_node_config<E: DeploymentAdapter>(
    deployment: &E::Deployment,
    node: &E::Node,
    index: usize,
    hostnames: &[String],
) -> Result<E::NodeConfig, BuildCfgsyncNodesError> {
    let mut node_config = E::build_node_config(deployment, node).map_err(adapter_error)?;
    E::rewrite_for_hostnames(deployment, index, hostnames, &mut node_config)
        .map_err(adapter_error)?;

    Ok(node_config)
}

fn build_node_artifacts_from_config(config: &NodeArtifacts) -> ArtifactSet {
    ArtifactSet::new(config.files.clone())
}

#[doc(hidden)]
pub type CfgsyncNodeConfig = NodeArtifacts;

#[doc(hidden)]
pub type CfgsyncNodeArtifacts = ArtifactSet;

#[doc(hidden)]
pub type RegistrationSnapshot = RegistrationSet;

#[doc(hidden)]
pub type CfgsyncNodeCatalog = NodeArtifactsCatalog;

#[doc(hidden)]
pub type MaterializingConfigProvider<M> = RegistrationConfigProvider<M>;

#[doc(hidden)]
pub type SnapshotMaterializingConfigProvider<M> = SnapshotConfigProvider<M>;

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use cfgsync_artifacts::ArtifactFile;
    use cfgsync_core::{CfgSyncErrorCode, ConfigProvider, NodeRegistration, RepoResponse};

    use super::{
        ArtifactSet, DynCfgsyncError, NodeArtifacts, NodeArtifactsCatalog,
        NodeArtifactsMaterializer, RegistrationConfigProvider, RegistrationSet,
    };

    #[test]
    fn catalog_resolves_identifier() {
        let catalog = NodeArtifactsCatalog::new(vec![NodeArtifacts {
            identifier: "node-1".to_owned(),
            files: vec![ArtifactFile::new("/config.yaml", "key: value")],
        }]);

        let node = catalog.resolve("node-1").expect("resolve node config");

        assert_eq!(node.files[0].content, "key: value");
    }

    #[test]
    fn materializing_provider_resolves_registered_node() {
        let catalog = NodeArtifactsCatalog::new(vec![NodeArtifacts {
            identifier: "node-1".to_owned(),
            files: vec![ArtifactFile::new("/config.yaml", "key: value")],
        }]);
        let provider = RegistrationConfigProvider::new(catalog);
        let registration = NodeRegistration::new("node-1", "127.0.0.1".parse().expect("parse ip"));

        let _ = provider.register(registration.clone());

        match provider.resolve(&registration) {
            RepoResponse::Config(payload) => assert_eq!(payload.files()[0].path, "/config.yaml"),
            RepoResponse::Error(error) => panic!("expected config, got {error}"),
        }
    }

    #[test]
    fn materializing_provider_reports_not_ready_before_registration() {
        let catalog = NodeArtifactsCatalog::new(vec![NodeArtifacts {
            identifier: "node-1".to_owned(),
            files: vec![ArtifactFile::new("/config.yaml", "key: value")],
        }]);
        let provider = RegistrationConfigProvider::new(catalog);
        let registration = NodeRegistration::new("node-1", "127.0.0.1".parse().expect("parse ip"));

        match provider.resolve(&registration) {
            RepoResponse::Config(_) => panic!("expected not-ready error"),
            RepoResponse::Error(error) => assert!(matches!(error.code, CfgSyncErrorCode::NotReady)),
        }
    }

    struct ThresholdMaterializer {
        calls: AtomicUsize,
    }

    impl NodeArtifactsMaterializer for ThresholdMaterializer {
        fn materialize(
            &self,
            registration: &NodeRegistration,
            registrations: &RegistrationSet,
        ) -> Result<Option<ArtifactSet>, DynCfgsyncError> {
            self.calls.fetch_add(1, Ordering::SeqCst);

            if registrations.len() < 2 {
                return Ok(None);
            }

            let peer_count = registrations.iter().count();
            let files = vec![
                ArtifactFile::new("/config.yaml", format!("id: {}", registration.identifier)),
                ArtifactFile::new("/shared.yaml", format!("peers: {peer_count}")),
            ];

            Ok(Some(ArtifactSet::new(files)))
        }
    }

    #[test]
    fn materializing_provider_uses_registration_snapshot_for_readiness() {
        let provider = RegistrationConfigProvider::new(ThresholdMaterializer {
            calls: AtomicUsize::new(0),
        });
        let node_a = NodeRegistration::new("node-a", "127.0.0.1".parse().expect("parse ip"));
        let node_b = NodeRegistration::new("node-b", "127.0.0.2".parse().expect("parse ip"));

        let _ = provider.register(node_a.clone());

        match provider.resolve(&node_a) {
            RepoResponse::Config(_) => panic!("expected not-ready error"),
            RepoResponse::Error(error) => assert!(matches!(error.code, CfgSyncErrorCode::NotReady)),
        }

        let _ = provider.register(node_b);

        match provider.resolve(&node_a) {
            RepoResponse::Config(payload) => {
                assert_eq!(payload.files()[0].content, "id: node-a");
                assert_eq!(payload.files()[1].content, "peers: 2");
            }
            RepoResponse::Error(error) => panic!("expected config, got {error}"),
        }
    }
}

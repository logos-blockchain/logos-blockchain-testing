use std::{collections::HashMap, error::Error, sync::Mutex};

use cfgsync_artifacts::ArtifactFile;
use cfgsync_core::{
    CfgSyncErrorResponse, CfgSyncPayload, ConfigProvider, NodeRegistration, RegistrationResponse,
    RepoResponse,
};
use thiserror::Error;

/// Type-erased cfgsync adapter error used to preserve source context.
pub type DynCfgsyncError = Box<dyn Error + Send + Sync + 'static>;

/// Per-node rendered config output used to build cfgsync bundles.
#[derive(Debug, Clone)]
pub struct CfgsyncNodeConfig {
    /// Stable node identifier resolved by the adapter.
    pub identifier: String,
    /// Serialized config payload for the node.
    pub config_yaml: String,
}

/// Node artifacts produced by a cfgsync materializer.
#[derive(Debug, Clone, Default)]
pub struct CfgsyncNodeArtifacts {
    files: Vec<ArtifactFile>,
}

impl CfgsyncNodeArtifacts {
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

/// Precomputed node configs indexed by stable identifier.
#[derive(Debug, Clone, Default)]
pub struct CfgsyncNodeCatalog {
    nodes: HashMap<String, CfgsyncNodeConfig>,
}

impl CfgsyncNodeCatalog {
    #[must_use]
    pub fn new(nodes: Vec<CfgsyncNodeConfig>) -> Self {
        let nodes = nodes
            .into_iter()
            .map(|node| (node.identifier.clone(), node))
            .collect();

        Self { nodes }
    }

    #[must_use]
    pub fn resolve(&self, identifier: &str) -> Option<&CfgsyncNodeConfig> {
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
    pub fn into_configs(self) -> Vec<CfgsyncNodeConfig> {
        self.nodes.into_values().collect()
    }
}

/// Adapter-side node config materialization contract used by cfgsync server.
pub trait CfgsyncMaterializer: Send + Sync {
    fn materialize(
        &self,
        registration: &NodeRegistration,
    ) -> Result<Option<CfgsyncNodeArtifacts>, DynCfgsyncError>;
}

impl CfgsyncMaterializer for CfgsyncNodeCatalog {
    fn materialize(
        &self,
        registration: &NodeRegistration,
    ) -> Result<Option<CfgsyncNodeArtifacts>, DynCfgsyncError> {
        let artifacts = self
            .resolve(&registration.identifier)
            .map(build_node_artifacts_from_config);

        Ok(artifacts)
    }
}

/// Registration-aware provider backed by an adapter materializer.
pub struct MaterializingConfigProvider<M> {
    materializer: M,
    registrations: Mutex<HashMap<String, NodeRegistration>>,
}

impl<M> MaterializingConfigProvider<M> {
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
}

impl<M> ConfigProvider for MaterializingConfigProvider<M>
where
    M: CfgsyncMaterializer,
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

        match self.materializer.materialize(&registration) {
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
    Ok(build_cfgsync_node_catalog::<E>(deployment, hostnames)?.into_configs())
}

/// Builds cfgsync node configs and indexes them by stable identifier.
pub fn build_cfgsync_node_catalog<E: CfgsyncEnv>(
    deployment: &E::Deployment,
    hostnames: &[String],
) -> Result<CfgsyncNodeCatalog, BuildCfgsyncNodesError> {
    let nodes = E::nodes(deployment);
    ensure_hostname_count(nodes.len(), hostnames.len())?;

    let mut output = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        output.push(build_node_entry::<E>(deployment, node, index, hostnames)?);
    }

    Ok(CfgsyncNodeCatalog::new(output))
}

fn ensure_hostname_count(nodes: usize, hostnames: usize) -> Result<(), BuildCfgsyncNodesError> {
    if nodes != hostnames {
        return Err(BuildCfgsyncNodesError::HostnameCountMismatch { nodes, hostnames });
    }

    Ok(())
}

fn build_node_entry<E: CfgsyncEnv>(
    deployment: &E::Deployment,
    node: &E::Node,
    index: usize,
    hostnames: &[String],
) -> Result<CfgsyncNodeConfig, BuildCfgsyncNodesError> {
    let node_config = build_rewritten_node_config::<E>(deployment, node, index, hostnames)?;
    let config_yaml = E::serialize_node_config(&node_config).map_err(adapter_error)?;

    Ok(CfgsyncNodeConfig {
        identifier: E::node_identifier(index, node),
        config_yaml,
    })
}

fn build_rewritten_node_config<E: CfgsyncEnv>(
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

fn build_node_artifacts_from_config(config: &CfgsyncNodeConfig) -> CfgsyncNodeArtifacts {
    CfgsyncNodeArtifacts::new(vec![ArtifactFile::new("/config.yaml", &config.config_yaml)])
}

#[cfg(test)]
mod tests {
    use cfgsync_core::{CfgSyncErrorCode, ConfigProvider, NodeRegistration, RepoResponse};

    use super::{CfgsyncNodeCatalog, CfgsyncNodeConfig, MaterializingConfigProvider};

    #[test]
    fn catalog_resolves_identifier() {
        let catalog = CfgsyncNodeCatalog::new(vec![CfgsyncNodeConfig {
            identifier: "node-1".to_owned(),
            config_yaml: "key: value".to_owned(),
        }]);

        let node = catalog.resolve("node-1").expect("resolve node config");

        assert_eq!(node.config_yaml, "key: value");
    }

    #[test]
    fn materializing_provider_resolves_registered_node() {
        let catalog = CfgsyncNodeCatalog::new(vec![CfgsyncNodeConfig {
            identifier: "node-1".to_owned(),
            config_yaml: "key: value".to_owned(),
        }]);
        let provider = MaterializingConfigProvider::new(catalog);
        let registration = NodeRegistration {
            identifier: "node-1".to_owned(),
            ip: "127.0.0.1".parse().expect("parse ip"),
        };

        let _ = provider.register(registration.clone());

        match provider.resolve(&registration) {
            RepoResponse::Config(payload) => assert_eq!(payload.files()[0].path, "/config.yaml"),
            RepoResponse::Error(error) => panic!("expected config, got {error}"),
        }
    }

    #[test]
    fn materializing_provider_reports_not_ready_before_registration() {
        let catalog = CfgsyncNodeCatalog::new(vec![CfgsyncNodeConfig {
            identifier: "node-1".to_owned(),
            config_yaml: "key: value".to_owned(),
        }]);
        let provider = MaterializingConfigProvider::new(catalog);
        let registration = NodeRegistration {
            identifier: "node-1".to_owned(),
            ip: "127.0.0.1".parse().expect("parse ip"),
        };

        match provider.resolve(&registration) {
            RepoResponse::Config(_) => panic!("expected not-ready error"),
            RepoResponse::Error(error) => assert!(matches!(error.code, CfgSyncErrorCode::NotReady)),
        }
    }
}

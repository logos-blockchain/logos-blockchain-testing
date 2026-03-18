use std::{collections::HashMap, fs, path::Path, sync::Arc};

use thiserror::Error;

use crate::{
    NodeArtifactsBundle, NodeArtifactsBundleEntry, NodeArtifactsPayload, NodeRegistration,
    RegisterNodeResponse, protocol::ConfigResolveResponse,
};

/// Source of cfgsync node payloads.
pub trait NodeConfigSource: Send + Sync {
    /// Records a node registration before config resolution.
    fn register(&self, registration: NodeRegistration) -> RegisterNodeResponse;

    /// Resolves the current artifact payload for a previously registered node.
    fn resolve(&self, registration: &NodeRegistration) -> ConfigResolveResponse;
}

/// In-memory map-backed source used by cfgsync server state.
pub struct StaticConfigSource {
    configs: HashMap<String, NodeArtifactsPayload>,
}

impl StaticConfigSource {
    /// Builds an in-memory source from fully formed payloads.
    #[must_use]
    pub fn from_payloads(configs: HashMap<String, NodeArtifactsPayload>) -> Arc<Self> {
        Arc::new(Self { configs })
    }

    /// Builds an in-memory source from a static bundle document.
    #[must_use]
    pub fn from_bundle(bundle: NodeArtifactsBundle) -> Arc<Self> {
        Self::from_payloads(bundle_to_payload_map(bundle))
    }
}

impl NodeConfigSource for StaticConfigSource {
    fn register(&self, registration: NodeRegistration) -> RegisterNodeResponse {
        if self.configs.contains_key(&registration.identifier) {
            RegisterNodeResponse::Registered
        } else {
            RegisterNodeResponse::Error(crate::CfgsyncErrorResponse::missing_config(
                &registration.identifier,
            ))
        }
    }

    fn resolve(&self, registration: &NodeRegistration) -> ConfigResolveResponse {
        self.configs
            .get(&registration.identifier)
            .cloned()
            .map_or_else(
                || {
                    ConfigResolveResponse::Error(crate::CfgsyncErrorResponse::missing_config(
                        &registration.identifier,
                    ))
                },
                ConfigResolveResponse::Config,
            )
    }
}

#[derive(Debug, Error)]
pub enum BundleLoadError {
    #[error("reading cfgsync bundle {path}: {source}")]
    ReadBundle {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing cfgsync bundle {path}: {source}")]
    ParseBundle {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
}

/// Converts a static bundle into the node payload map used by static sources.
#[must_use]
pub fn bundle_to_payload_map(bundle: NodeArtifactsBundle) -> HashMap<String, NodeArtifactsPayload> {
    let shared_files = bundle.shared_files;

    bundle
        .nodes
        .into_iter()
        .map(|node| {
            let NodeArtifactsBundleEntry { identifier, files } = node;

            let mut payload_files = files;
            payload_files.extend(shared_files.clone());

            (identifier, NodeArtifactsPayload::from_files(payload_files))
        })
        .collect()
}

/// Loads a cfgsync bundle YAML file from disk.
pub fn load_bundle(path: &Path) -> Result<NodeArtifactsBundle, BundleLoadError> {
    let path_string = path.display().to_string();
    let raw = fs::read_to_string(path).map_err(|source| BundleLoadError::ReadBundle {
        path: path_string.clone(),
        source,
    })?;
    serde_yaml::from_str(&raw).map_err(|source| BundleLoadError::ParseBundle {
        path: path_string,
        source,
    })
}

/// Failures when loading a bundle-backed cfgsync source.
#[derive(Debug, Error)]
pub enum BundleConfigSourceError {
    #[error("failed to read cfgsync bundle at {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse cfgsync bundle at {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
}

/// YAML bundle-backed source implementation.
pub struct BundleConfigSource {
    inner: StaticConfigSource,
}

impl BundleConfigSource {
    /// Loads source state from a cfgsync bundle YAML file.
    pub fn from_yaml_file(path: &Path) -> Result<Self, BundleConfigSourceError> {
        let raw = fs::read_to_string(path).map_err(|source| BundleConfigSourceError::Read {
            path: path.display().to_string(),
            source,
        })?;

        let bundle: NodeArtifactsBundle =
            serde_yaml::from_str(&raw).map_err(|source| BundleConfigSourceError::Parse {
                path: path.display().to_string(),
                source,
            })?;

        let configs = bundle
            .nodes
            .into_iter()
            .map(payload_from_bundle_node)
            .collect();

        Ok(Self {
            inner: StaticConfigSource { configs },
        })
    }
}

impl NodeConfigSource for BundleConfigSource {
    fn register(&self, registration: NodeRegistration) -> RegisterNodeResponse {
        self.inner.register(registration)
    }

    fn resolve(&self, registration: &NodeRegistration) -> ConfigResolveResponse {
        self.inner.resolve(registration)
    }
}

fn payload_from_bundle_node(node: NodeArtifactsBundleEntry) -> (String, NodeArtifactsPayload) {
    (
        node.identifier,
        NodeArtifactsPayload::from_files(node.files),
    )
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, io::Write as _};

    use tempfile::NamedTempFile;

    use super::{BundleConfigSource, StaticConfigSource};
    use crate::{
        CFGSYNC_SCHEMA_VERSION, CfgsyncErrorCode, ConfigResolveResponse, NodeArtifactFile,
        NodeArtifactsPayload, NodeConfigSource, NodeRegistration,
    };

    fn sample_payload() -> NodeArtifactsPayload {
        NodeArtifactsPayload::from_files(vec![NodeArtifactFile::new(
            "/config.yaml".to_string(),
            "key: value".to_string(),
        )])
    }

    #[test]
    fn resolves_existing_identifier() {
        let mut configs = HashMap::new();
        configs.insert("node-1".to_owned(), sample_payload());
        let repo = StaticConfigSource { configs };

        match repo.resolve(&NodeRegistration::new(
            "node-1".to_string(),
            "127.0.0.1".parse().expect("parse ip"),
        )) {
            ConfigResolveResponse::Config(payload) => {
                assert_eq!(payload.schema_version, CFGSYNC_SCHEMA_VERSION);
                assert_eq!(payload.files.len(), 1);
                assert_eq!(payload.files[0].path, "/config.yaml");
            }
            ConfigResolveResponse::Error(error) => panic!("expected config response, got {error}"),
        }
    }

    #[test]
    fn reports_missing_identifier() {
        let repo = StaticConfigSource {
            configs: HashMap::new(),
        };

        match repo.resolve(&NodeRegistration::new(
            "unknown-node".to_string(),
            "127.0.0.1".parse().expect("parse ip"),
        )) {
            ConfigResolveResponse::Config(_) => panic!("expected missing-config error"),
            ConfigResolveResponse::Error(error) => {
                assert!(matches!(error.code, CfgsyncErrorCode::MissingConfig));
                assert!(error.message.contains("unknown-node"));
            }
        }
    }

    #[test]
    fn loads_file_provider_bundle() {
        let mut bundle_file = NamedTempFile::new().expect("create temp bundle");
        let yaml = r#"
nodes:
  - identifier: node-1
    files:
      - path: /config.yaml
        content: "a: 1"
"#;
        bundle_file
            .write_all(yaml.as_bytes())
            .expect("write bundle yaml");

        let provider =
            BundleConfigSource::from_yaml_file(bundle_file.path()).expect("load file provider");

        let _ = provider.register(NodeRegistration::new(
            "node-1".to_string(),
            "127.0.0.1".parse().expect("parse ip"),
        ));

        match provider.resolve(&NodeRegistration::new(
            "node-1".to_string(),
            "127.0.0.1".parse().expect("parse ip"),
        )) {
            ConfigResolveResponse::Config(payload) => assert_eq!(payload.files.len(), 1),
            ConfigResolveResponse::Error(error) => panic!("expected config, got {error}"),
        }
    }

    #[test]
    fn resolve_accepts_known_registration_without_gating() {
        let mut configs = HashMap::new();
        configs.insert("node-1".to_owned(), sample_payload());
        let repo = StaticConfigSource { configs };

        match repo.resolve(&NodeRegistration::new(
            "node-1".to_string(),
            "127.0.0.1".parse().expect("parse ip"),
        )) {
            ConfigResolveResponse::Config(_) => {}
            ConfigResolveResponse::Error(error) => panic!("expected config, got {error}"),
        }
    }
}

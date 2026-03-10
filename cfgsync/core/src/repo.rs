use std::{collections::HashMap, fs, net::Ipv4Addr, path::Path, sync::Arc};

use cfgsync_artifacts::ArtifactFile;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{CfgSyncBundle, CfgSyncBundleNode};

/// Schema version served by cfgsync payload responses.
pub const CFGSYNC_SCHEMA_VERSION: u16 = 1;

/// Canonical cfgsync file type used in payloads and bundles.
pub type CfgSyncFile = ArtifactFile;

/// Payload returned by cfgsync server for one node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgSyncPayload {
    /// Payload schema version for compatibility checks.
    pub schema_version: u16,
    /// Files that must be written on the target node.
    #[serde(default)]
    pub files: Vec<CfgSyncFile>,
}

/// Node metadata recorded before config materialization.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeRegistration {
    pub identifier: String,
    pub ip: Ipv4Addr,
}

impl CfgSyncPayload {
    #[must_use]
    pub fn from_files(files: Vec<CfgSyncFile>) -> Self {
        Self {
            schema_version: CFGSYNC_SCHEMA_VERSION,
            files,
        }
    }

    #[must_use]
    pub fn files(&self) -> &[CfgSyncFile] {
        &self.files
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CfgSyncErrorCode {
    MissingConfig,
    NotReady,
    Internal,
}

/// Structured error body returned by cfgsync server.
#[derive(Debug, Clone, Serialize, Deserialize, Error)]
#[error("{code:?}: {message}")]
pub struct CfgSyncErrorResponse {
    pub code: CfgSyncErrorCode,
    pub message: String,
}

impl CfgSyncErrorResponse {
    #[must_use]
    pub fn missing_config(identifier: &str) -> Self {
        Self {
            code: CfgSyncErrorCode::MissingConfig,
            message: format!("missing config for host {identifier}"),
        }
    }

    #[must_use]
    pub fn not_ready(identifier: &str) -> Self {
        Self {
            code: CfgSyncErrorCode::NotReady,
            message: format!("config for host {identifier} is not ready"),
        }
    }

    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: CfgSyncErrorCode::Internal,
            message: message.into(),
        }
    }
}

/// Repository resolution outcome for a requested node identifier.
pub enum RepoResponse {
    Config(CfgSyncPayload),
    Error(CfgSyncErrorResponse),
}

/// Repository outcome for a node registration request.
pub enum RegistrationResponse {
    Registered,
    Error(CfgSyncErrorResponse),
}

/// Read-only source for cfgsync node payloads.
pub trait ConfigProvider: Send + Sync {
    fn register(&self, registration: NodeRegistration) -> RegistrationResponse;

    fn resolve(&self, registration: &NodeRegistration) -> RepoResponse;
}

/// In-memory map-backed provider used by cfgsync server state.
pub struct ConfigRepo {
    configs: HashMap<String, CfgSyncPayload>,
}

impl ConfigRepo {
    #[must_use]
    pub fn from_bundle(configs: HashMap<String, CfgSyncPayload>) -> Arc<Self> {
        Arc::new(Self { configs })
    }
}

impl ConfigProvider for ConfigRepo {
    fn register(&self, registration: NodeRegistration) -> RegistrationResponse {
        if self.configs.contains_key(&registration.identifier) {
            RegistrationResponse::Registered
        } else {
            RegistrationResponse::Error(CfgSyncErrorResponse::missing_config(
                &registration.identifier,
            ))
        }
    }

    fn resolve(&self, registration: &NodeRegistration) -> RepoResponse {
        self.configs
            .get(&registration.identifier)
            .cloned()
            .map_or_else(
                || {
                    RepoResponse::Error(CfgSyncErrorResponse::missing_config(
                        &registration.identifier,
                    ))
                },
                RepoResponse::Config,
            )
    }
}

/// Failures when loading a file-backed cfgsync provider.
#[derive(Debug, Error)]
pub enum FileConfigProviderError {
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

/// YAML bundle-backed provider implementation.
pub struct FileConfigProvider {
    inner: ConfigRepo,
}

impl FileConfigProvider {
    /// Loads provider state from a cfgsync bundle YAML file.
    pub fn from_yaml_file(path: &Path) -> Result<Self, FileConfigProviderError> {
        let raw = fs::read_to_string(path).map_err(|source| FileConfigProviderError::Read {
            path: path.display().to_string(),
            source,
        })?;

        let bundle: CfgSyncBundle =
            serde_yaml::from_str(&raw).map_err(|source| FileConfigProviderError::Parse {
                path: path.display().to_string(),
                source,
            })?;

        let configs = bundle
            .nodes
            .into_iter()
            .map(payload_from_bundle_node)
            .collect();

        Ok(Self {
            inner: ConfigRepo { configs },
        })
    }
}

impl ConfigProvider for FileConfigProvider {
    fn register(&self, registration: NodeRegistration) -> RegistrationResponse {
        self.inner.register(registration)
    }

    fn resolve(&self, registration: &NodeRegistration) -> RepoResponse {
        self.inner.resolve(registration)
    }
}

fn payload_from_bundle_node(node: CfgSyncBundleNode) -> (String, CfgSyncPayload) {
    (node.identifier, CfgSyncPayload::from_files(node.files))
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use tempfile::NamedTempFile;

    use super::*;

    fn sample_payload() -> CfgSyncPayload {
        CfgSyncPayload::from_files(vec![CfgSyncFile::new("/config.yaml", "key: value")])
    }

    #[test]
    fn resolves_existing_identifier() {
        let mut configs = HashMap::new();
        configs.insert("node-1".to_owned(), sample_payload());
        let repo = ConfigRepo { configs };

        match repo.resolve(&NodeRegistration {
            identifier: "node-1".to_owned(),
            ip: "127.0.0.1".parse().expect("parse ip"),
        }) {
            RepoResponse::Config(payload) => {
                assert_eq!(payload.schema_version, CFGSYNC_SCHEMA_VERSION);
                assert_eq!(payload.files.len(), 1);
                assert_eq!(payload.files[0].path, "/config.yaml");
            }
            RepoResponse::Error(error) => panic!("expected config response, got {error}"),
        }
    }

    #[test]
    fn reports_missing_identifier() {
        let repo = ConfigRepo {
            configs: HashMap::new(),
        };

        match repo.resolve(&NodeRegistration {
            identifier: "unknown-node".to_owned(),
            ip: "127.0.0.1".parse().expect("parse ip"),
        }) {
            RepoResponse::Config(_) => panic!("expected missing-config error"),
            RepoResponse::Error(error) => {
                assert!(matches!(error.code, CfgSyncErrorCode::MissingConfig));
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
            FileConfigProvider::from_yaml_file(bundle_file.path()).expect("load file provider");

        let _ = provider.register(NodeRegistration {
            identifier: "node-1".to_owned(),
            ip: "127.0.0.1".parse().expect("parse ip"),
        });

        match provider.resolve(&NodeRegistration {
            identifier: "node-1".to_owned(),
            ip: "127.0.0.1".parse().expect("parse ip"),
        }) {
            RepoResponse::Config(payload) => assert_eq!(payload.files.len(), 1),
            RepoResponse::Error(error) => panic!("expected config, got {error}"),
        }
    }

    #[test]
    fn resolve_accepts_known_registration_without_gating() {
        let mut configs = HashMap::new();
        configs.insert("node-1".to_owned(), sample_payload());
        let repo = ConfigRepo { configs };

        match repo.resolve(&NodeRegistration {
            identifier: "node-1".to_owned(),
            ip: "127.0.0.1".parse().expect("parse ip"),
        }) {
            RepoResponse::Config(_) => {}
            RepoResponse::Error(error) => panic!("expected config, got {error}"),
        }
    }
}

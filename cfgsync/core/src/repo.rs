use std::{collections::HashMap, fs, net::Ipv4Addr, path::Path, sync::Arc};

use cfgsync_artifacts::ArtifactFile;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use serde_json::Value;
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

/// Adapter-owned registration payload stored alongside a generic node identity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegistrationPayload {
    raw_json: Option<String>,
}

impl RegistrationPayload {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.raw_json.is_none()
    }

    pub fn from_serializable<T>(value: &T) -> Result<Self, serde_json::Error>
    where
        T: Serialize,
    {
        Ok(Self {
            raw_json: Some(serde_json::to_string(value)?),
        })
    }

    pub fn from_json_str(raw_json: &str) -> Result<Self, serde_json::Error> {
        let value: Value = serde_json::from_str(raw_json)?;

        Ok(Self {
            raw_json: Some(serde_json::to_string(&value)?),
        })
    }

    pub fn deserialize<T>(&self) -> Result<Option<T>, serde_json::Error>
    where
        T: DeserializeOwned,
    {
        self.raw_json
            .as_ref()
            .map(|raw_json| serde_json::from_str(raw_json))
            .transpose()
    }

    #[must_use]
    pub fn raw_json(&self) -> Option<&str> {
        self.raw_json.as_deref()
    }
}

impl Serialize for RegistrationPayload {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.raw_json.as_deref() {
            Some(raw_json) => {
                let value: Value =
                    serde_json::from_str(raw_json).map_err(serde::ser::Error::custom)?;
                value.serialize(serializer)
            }
            None => serializer.serialize_none(),
        }
    }
}

impl<'de> Deserialize<'de> for RegistrationPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Option::<Value>::deserialize(deserializer)?;
        let raw_json = value
            .map(|value| serde_json::to_string(&value).map_err(serde::de::Error::custom))
            .transpose()?;

        Ok(Self { raw_json })
    }
}

/// Node metadata recorded before config materialization.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeRegistration {
    pub identifier: String,
    pub ip: Ipv4Addr,
    #[serde(default, skip_serializing_if = "RegistrationPayload::is_empty")]
    pub metadata: RegistrationPayload,
}

impl NodeRegistration {
    #[must_use]
    pub fn new(identifier: impl Into<String>, ip: Ipv4Addr) -> Self {
        Self {
            identifier: identifier.into(),
            ip,
            metadata: RegistrationPayload::default(),
        }
    }

    pub fn with_metadata<T>(mut self, metadata: &T) -> Result<Self, serde_json::Error>
    where
        T: Serialize,
    {
        self.metadata = RegistrationPayload::from_serializable(metadata)?;
        Ok(self)
    }

    #[must_use]
    pub fn with_payload(mut self, payload: RegistrationPayload) -> Self {
        self.metadata = payload;
        self
    }
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

/// Resolution outcome for a requested node identifier.
pub enum ConfigResolveResponse {
    Config(CfgSyncPayload),
    Error(CfgSyncErrorResponse),
}

/// Outcome for a node registration request.
pub enum RegisterNodeResponse {
    Registered,
    Error(CfgSyncErrorResponse),
}

/// Source of cfgsync node payloads.
pub trait NodeConfigSource: Send + Sync {
    fn register(&self, registration: NodeRegistration) -> RegisterNodeResponse;

    fn resolve(&self, registration: &NodeRegistration) -> ConfigResolveResponse;
}

/// In-memory map-backed source used by cfgsync server state.
pub struct StaticConfigSource {
    configs: HashMap<String, CfgSyncPayload>,
}

impl StaticConfigSource {
    #[must_use]
    pub fn from_bundle(configs: HashMap<String, CfgSyncPayload>) -> Arc<Self> {
        Arc::new(Self { configs })
    }
}

impl NodeConfigSource for StaticConfigSource {
    fn register(&self, registration: NodeRegistration) -> RegisterNodeResponse {
        if self.configs.contains_key(&registration.identifier) {
            RegisterNodeResponse::Registered
        } else {
            RegisterNodeResponse::Error(CfgSyncErrorResponse::missing_config(
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
                    ConfigResolveResponse::Error(CfgSyncErrorResponse::missing_config(
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

#[must_use]
pub fn bundle_to_payload_map(bundle: CfgSyncBundle) -> HashMap<String, CfgSyncPayload> {
    bundle
        .nodes
        .into_iter()
        .map(|node| {
            let CfgSyncBundleNode { identifier, files } = node;

            (identifier, CfgSyncPayload::from_files(files))
        })
        .collect()
}

pub fn load_bundle(path: &Path) -> Result<CfgSyncBundle, BundleLoadError> {
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

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use tempfile::NamedTempFile;

    use super::*;

    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
    struct ExampleRegistration {
        network_port: u16,
        service: String,
    }

    #[test]
    fn registration_payload_round_trips_typed_value() {
        let registration = NodeRegistration::new("node-1", "127.0.0.1".parse().expect("parse ip"))
            .with_metadata(&ExampleRegistration {
                network_port: 3000,
                service: "blend".to_owned(),
            })
            .expect("serialize registration metadata");

        let encoded = serde_json::to_value(&registration).expect("serialize registration");
        let metadata = encoded.get("metadata").expect("registration metadata");
        assert_eq!(metadata.get("network_port"), Some(&Value::from(3000u16)));
        assert_eq!(metadata.get("service"), Some(&Value::from("blend")));

        let decoded: NodeRegistration =
            serde_json::from_value(encoded).expect("deserialize registration");
        let typed: ExampleRegistration = decoded
            .metadata
            .deserialize()
            .expect("deserialize metadata")
            .expect("registration metadata value");

        assert_eq!(typed.network_port, 3000);
        assert_eq!(typed.service, "blend");
    }

    fn sample_payload() -> CfgSyncPayload {
        CfgSyncPayload::from_files(vec![CfgSyncFile::new("/config.yaml", "key: value")])
    }

    #[test]
    fn resolves_existing_identifier() {
        let mut configs = HashMap::new();
        configs.insert("node-1".to_owned(), sample_payload());
        let repo = StaticConfigSource { configs };

        match repo.resolve(&NodeRegistration::new(
            "node-1",
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
            "unknown-node",
            "127.0.0.1".parse().expect("parse ip"),
        )) {
            ConfigResolveResponse::Config(_) => panic!("expected missing-config error"),
            ConfigResolveResponse::Error(error) => {
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
            BundleConfigSource::from_yaml_file(bundle_file.path()).expect("load file provider");

        let _ = provider.register(NodeRegistration::new(
            "node-1",
            "127.0.0.1".parse().expect("parse ip"),
        ));

        match provider.resolve(&NodeRegistration::new(
            "node-1",
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
            "node-1",
            "127.0.0.1".parse().expect("parse ip"),
        )) {
            ConfigResolveResponse::Config(_) => {}
            ConfigResolveResponse::Error(error) => panic!("expected config, got {error}"),
        }
    }
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
    /// Loads provider state from a cfgsync bundle YAML file.
    pub fn from_yaml_file(path: &Path) -> Result<Self, BundleConfigSourceError> {
        let raw = fs::read_to_string(path).map_err(|source| BundleConfigSourceError::Read {
            path: path.display().to_string(),
            source,
        })?;

        let bundle: CfgSyncBundle =
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

fn payload_from_bundle_node(node: CfgSyncBundleNode) -> (String, CfgSyncPayload) {
    (node.identifier, CfgSyncPayload::from_files(node.files))
}

#[doc(hidden)]
pub type RepoResponse = ConfigResolveResponse;

#[doc(hidden)]
pub type RegistrationResponse = RegisterNodeResponse;

#[doc(hidden)]
pub trait ConfigProvider: NodeConfigSource {}

impl<T: NodeConfigSource + ?Sized> ConfigProvider for T {}

#[doc(hidden)]
pub type ConfigRepo = StaticConfigSource;

#[doc(hidden)]
pub type FileConfigProvider = BundleConfigSource;

#[doc(hidden)]
pub type FileConfigProviderError = BundleConfigSourceError;

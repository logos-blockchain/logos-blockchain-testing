use std::net::Ipv4Addr;

use cfgsync_artifacts::ArtifactFile;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use serde_json::Value;
use thiserror::Error;

/// Schema version served by cfgsync payload responses.
pub const CFGSYNC_SCHEMA_VERSION: u16 = 1;

/// Canonical cfgsync file type used in payloads and bundles.
pub type NodeArtifactFile = ArtifactFile;

/// Payload returned by cfgsync server for one node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeArtifactsPayload {
    /// Payload schema version for compatibility checks.
    pub schema_version: u16,
    /// Files that must be written on the target node.
    #[serde(default)]
    pub files: Vec<NodeArtifactFile>,
}

/// Adapter-owned registration payload stored alongside a generic node identity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegistrationPayload {
    raw_json: Option<String>,
}

impl RegistrationPayload {
    /// Creates an empty adapter-owned payload.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when no adapter-owned payload was attached.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.raw_json.is_none()
    }

    /// Stores one typed adapter payload as opaque JSON.
    pub fn from_serializable<T>(value: &T) -> Result<Self, serde_json::Error>
    where
        T: Serialize,
    {
        Ok(Self {
            raw_json: Some(serde_json::to_string(value)?),
        })
    }

    /// Stores a raw JSON payload after validating that it parses.
    pub fn from_json_str(raw_json: &str) -> Result<Self, serde_json::Error> {
        let value: Value = serde_json::from_str(raw_json)?;

        Ok(Self {
            raw_json: Some(serde_json::to_string(&value)?),
        })
    }

    /// Deserializes the adapter-owned payload into the requested type.
    pub fn deserialize<T>(&self) -> Result<Option<T>, serde_json::Error>
    where
        T: DeserializeOwned,
    {
        self.raw_json
            .as_ref()
            .map(|raw_json| serde_json::from_str(raw_json))
            .transpose()
    }

    /// Returns the validated JSON representation stored in this payload.
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
    /// Stable node identifier used for registration and artifact lookup.
    pub identifier: String,
    /// IPv4 address advertised as part of registration.
    pub ip: Ipv4Addr,
    /// Adapter-owned payload interpreted only by the app materializer.
    #[serde(default, skip_serializing_if = "RegistrationPayload::is_empty")]
    pub metadata: RegistrationPayload,
}

impl NodeRegistration {
    /// Creates a registration with the generic node identity fields only.
    #[must_use]
    pub fn new(identifier: impl Into<String>, ip: Ipv4Addr) -> Self {
        Self {
            identifier: identifier.into(),
            ip,
            metadata: RegistrationPayload::default(),
        }
    }

    /// Attaches one typed adapter-owned payload to this registration.
    pub fn with_metadata<T>(mut self, metadata: &T) -> Result<Self, serde_json::Error>
    where
        T: Serialize,
    {
        self.metadata = RegistrationPayload::from_serializable(metadata)?;
        Ok(self)
    }

    /// Attaches a prebuilt registration payload to this registration.
    #[must_use]
    pub fn with_payload(mut self, payload: RegistrationPayload) -> Self {
        self.metadata = payload;
        self
    }
}

impl NodeArtifactsPayload {
    /// Creates a payload from the files that should be written for one node.
    #[must_use]
    pub fn from_files(files: Vec<NodeArtifactFile>) -> Self {
        Self {
            schema_version: CFGSYNC_SCHEMA_VERSION,
            files,
        }
    }

    /// Returns the files carried by this payload.
    #[must_use]
    pub fn files(&self) -> &[NodeArtifactFile] {
        &self.files
    }

    /// Returns `true` when the payload carries no files.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CfgsyncErrorCode {
    /// No artifact payload is available for the requested node.
    MissingConfig,
    /// The node is registered but artifacts are not ready yet.
    NotReady,
    /// An unexpected server-side failure occurred.
    Internal,
}

/// Structured error body returned by cfgsync server.
#[derive(Debug, Clone, Serialize, Deserialize, Error)]
#[error("{code:?}: {message}")]
pub struct CfgsyncErrorResponse {
    /// Machine-readable failure category.
    pub code: CfgsyncErrorCode,
    /// Human-readable error details.
    pub message: String,
}

impl CfgsyncErrorResponse {
    /// Builds a missing-config error for one identifier.
    #[must_use]
    pub fn missing_config(identifier: &str) -> Self {
        Self {
            code: CfgsyncErrorCode::MissingConfig,
            message: format!("missing config for host {identifier}"),
        }
    }

    /// Builds a not-ready error for one identifier.
    #[must_use]
    pub fn not_ready(identifier: &str) -> Self {
        Self {
            code: CfgsyncErrorCode::NotReady,
            message: format!("config for host {identifier} is not ready"),
        }
    }

    /// Builds an internal cfgsync error.
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: CfgsyncErrorCode::Internal,
            message: message.into(),
        }
    }
}

/// Resolution outcome for a requested node identifier.
pub enum ConfigResolveResponse {
    /// Artifacts are ready for the requested node.
    Config(NodeArtifactsPayload),
    /// Artifacts could not be resolved for the requested node.
    Error(CfgsyncErrorResponse),
}

/// Outcome for a node registration request.
pub enum RegisterNodeResponse {
    /// Registration was accepted.
    Registered,
    /// Registration failed.
    Error(CfgsyncErrorResponse),
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    use super::{NodeRegistration, RegistrationPayload};

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

    #[test]
    fn registration_payload_accepts_raw_json() {
        let payload =
            RegistrationPayload::from_json_str(r#"{"network_port":3000}"#).expect("parse raw json");

        assert_eq!(payload.raw_json(), Some(r#"{"network_port":3000}"#));
    }
}

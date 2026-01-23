use serde::Serialize;
use testing_framework_core::topology::generation::GeneratedNodeConfig;

use super::{ComposeNodeKind, base_environment, base_volumes, default_extra_hosts};

/// Describes a validator container in the compose stack.
#[derive(Clone, Debug, Serialize)]
pub struct NodeDescriptor {
    name: String,
    image: String,
    entrypoint: String,
    volumes: Vec<String>,
    extra_hosts: Vec<String>,
    ports: Vec<String>,
    environment: Vec<EnvEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<String>,
}

/// Environment variable entry for docker-compose templating.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct EnvEntry {
    key: String,
    value: String,
}

impl EnvEntry {
    pub(crate) fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }

    #[cfg(test)]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[cfg(test)]
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl NodeDescriptor {
    pub(crate) fn from_node(
        kind: ComposeNodeKind,
        index: usize,
        node: &GeneratedNodeConfig,
        image: &str,
        platform: Option<&str>,
        use_kzg_mount: bool,
        cfgsync_port: u16,
    ) -> Self {
        let mut environment = base_environment(cfgsync_port, use_kzg_mount);
        let identifier = kind.instance_name(index);
        let api_port = node.general.api_config.address.port();
        let testing_port = node.general.api_config.testing_http_address.port();
        environment.extend([
            EnvEntry::new(
                "CFG_NETWORK_PORT",
                node.general.network_config.backend.swarm.port.to_string(),
            ),
            EnvEntry::new("CFG_BLEND_PORT", node.blend_port.to_string()),
            EnvEntry::new("CFG_API_PORT", api_port.to_string()),
            EnvEntry::new("CFG_TESTING_HTTP_PORT", testing_port.to_string()),
            EnvEntry::new("CFG_HOST_IDENTIFIER", identifier),
        ]);

        // Publish container ports on random host ports to avoid collisions with
        // local services and allow multiple compose stacks to run concurrently.
        // The runner discovers the chosen host ports via `docker compose port`.
        let ports = vec![
            format!("127.0.0.1::{api_port}"),
            format!("127.0.0.1::{testing_port}"),
        ];

        Self {
            name: kind.instance_name(index),
            image: image.to_owned(),
            entrypoint: kind.entrypoint().to_owned(),
            volumes: base_volumes(use_kzg_mount),
            extra_hosts: default_extra_hosts(),
            ports,
            environment,
            platform: platform.map(ToOwned::to_owned),
        }
    }

    #[cfg(test)]
    pub fn ports(&self) -> &[String] {
        &self.ports
    }

    #[cfg(test)]
    pub fn environment(&self) -> &[EnvEntry] {
        &self.environment
    }
}

use std::env;

use serde::Serialize;

use crate::infrastructure::ports::node_identifier;

/// Describes a node container in the compose stack.
#[derive(Clone, Debug, Serialize)]
pub struct NodeDescriptor {
    name: String,
    image: String,
    entrypoint: Vec<String>,
    volumes: Vec<String>,
    extra_hosts: Vec<String>,
    ports: Vec<String>,
    #[serde(skip)]
    container_ports: Vec<u16>,
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
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
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
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: impl Into<String>,
        image: impl Into<String>,
        entrypoint: Vec<String>,
        volumes: Vec<String>,
        extra_hosts: Vec<String>,
        ports: Vec<String>,
        container_ports: Vec<u16>,
        environment: Vec<EnvEntry>,
        platform: Option<String>,
    ) -> Self {
        Self {
            name: name.into(),
            image: image.into(),
            entrypoint,
            volumes,
            extra_hosts,
            ports,
            container_ports,
            environment,
            platform,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_loopback_ports(
        name: impl Into<String>,
        image: impl Into<String>,
        entrypoint: Vec<String>,
        volumes: Vec<String>,
        extra_hosts: Vec<String>,
        container_ports: Vec<u16>,
        environment: Vec<EnvEntry>,
        platform: Option<String>,
    ) -> Self {
        Self::new(
            name,
            image,
            entrypoint,
            volumes,
            extra_hosts,
            container_ports
                .iter()
                .copied()
                .map(Self::loopback_port_binding)
                .collect(),
            container_ports,
            environment,
            platform,
        )
    }

    #[must_use]
    pub fn loopback_port_binding(port: u16) -> String {
        format!("127.0.0.1::{port}")
    }

    pub fn ports(&self) -> &[String] {
        &self.ports
    }

    pub fn container_ports(&self) -> &[u16] {
        &self.container_ports
    }

    pub fn image(&self) -> &str {
        &self.image
    }

    pub fn platform(&self) -> Option<&str> {
        self.platform.as_deref()
    }

    #[cfg(test)]
    pub fn test_ports(&self) -> &[String] {
        self.ports()
    }

    #[cfg(test)]
    pub fn environment(&self) -> &[EnvEntry] {
        &self.environment
    }
}

#[derive(Clone, Debug)]
pub struct LoopbackNodeRuntimeSpec {
    pub image: String,
    pub entrypoint: Vec<String>,
    pub volumes: Vec<String>,
    pub extra_hosts: Vec<String>,
    pub container_ports: Vec<u16>,
    pub environment: Vec<EnvEntry>,
    pub platform: Option<String>,
}

#[derive(Clone, Debug)]
pub struct BinaryConfigNodeSpec {
    pub image_env_var: String,
    pub default_image: String,
    pub platform_env_var: String,
    pub binary_path: String,
    pub config_container_path: String,
    pub config_file_extension: String,
    pub container_ports: Vec<u16>,
    pub rust_log: String,
}

impl BinaryConfigNodeSpec {
    #[must_use]
    pub fn conventional(
        binary_path: &str,
        config_container_path: &str,
        container_ports: Vec<u16>,
    ) -> Self {
        let binary_name = binary_path
            .rsplit('/')
            .next()
            .unwrap_or(binary_path)
            .to_owned();
        let app_name = binary_name
            .strip_suffix("-node")
            .unwrap_or(&binary_name)
            .to_owned();
        let env_prefix = app_name.replace('-', "_").to_ascii_uppercase();
        let config_file_extension = config_container_path
            .rsplit('.')
            .next()
            .filter(|ext| !ext.contains('/'))
            .unwrap_or("yaml")
            .to_owned();
        let rust_target = binary_name.replace('-', "_");

        Self {
            image_env_var: format!("{env_prefix}_IMAGE"),
            default_image: format!("{binary_name}:local"),
            platform_env_var: format!("{env_prefix}_PLATFORM"),
            binary_path: binary_path.to_owned(),
            config_container_path: config_container_path.to_owned(),
            config_file_extension,
            container_ports,
            rust_log: format!("{rust_target}=info"),
        }
    }
}

pub fn build_loopback_node_descriptors(
    node_count: usize,
    mut spec_for_index: impl FnMut(usize) -> LoopbackNodeRuntimeSpec,
) -> Vec<NodeDescriptor> {
    (0..node_count)
        .map(|index| {
            let spec = spec_for_index(index);
            NodeDescriptor::with_loopback_ports(
                node_identifier(index),
                spec.image,
                spec.entrypoint,
                spec.volumes,
                spec.extra_hosts,
                spec.container_ports,
                spec.environment,
                spec.platform,
            )
        })
        .collect()
}

pub fn build_binary_config_node_descriptors(
    node_count: usize,
    spec: &BinaryConfigNodeSpec,
) -> Vec<NodeDescriptor> {
    build_loopback_node_descriptors(node_count, |index| {
        binary_config_node_runtime_spec(index, spec)
    })
}

pub fn binary_config_node_runtime_spec(
    index: usize,
    spec: &BinaryConfigNodeSpec,
) -> LoopbackNodeRuntimeSpec {
    let image = env::var(&spec.image_env_var).unwrap_or_else(|_| spec.default_image.clone());
    let platform = env::var(&spec.platform_env_var).ok();
    let entrypoint = vec![
        spec.binary_path.clone(),
        "--config".to_owned(),
        spec.config_container_path.clone(),
    ];

    LoopbackNodeRuntimeSpec {
        image,
        entrypoint,
        volumes: vec![format!(
            "./stack/configs/node-{index}.{}:{}:ro",
            spec.config_file_extension, spec.config_container_path
        )],
        extra_hosts: vec![],
        container_ports: spec.container_ports.clone(),
        environment: vec![EnvEntry::new("RUST_LOG", &spec.rust_log)],
        platform,
    }
}

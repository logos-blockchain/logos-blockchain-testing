use serde::Serialize;

/// Describes a node container in the compose stack.
#[derive(Clone, Debug, Serialize)]
pub struct NodeDescriptor {
    name: String,
    image: String,
    entrypoint: String,
    volumes: Vec<String>,
    extra_hosts: Vec<String>,
    ports: Vec<String>,
    api_container_port: u16,
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
        entrypoint: impl Into<String>,
        volumes: Vec<String>,
        extra_hosts: Vec<String>,
        ports: Vec<String>,
        api_container_port: u16,
        environment: Vec<EnvEntry>,
        platform: Option<String>,
    ) -> Self {
        Self {
            name: name.into(),
            image: image.into(),
            entrypoint: entrypoint.into(),
            volumes,
            extra_hosts,
            ports,
            api_container_port,
            environment,
            platform,
        }
    }

    pub fn ports(&self) -> &[String] {
        &self.ports
    }

    #[cfg(test)]
    pub fn test_ports(&self) -> &[String] {
        self.ports()
    }

    #[cfg(test)]
    pub fn environment(&self) -> &[EnvEntry] {
        &self.environment
    }

    #[cfg(test)]
    pub fn api_container_port(&self) -> u16 {
        self.api_container_port
    }
}

use std::collections::HashMap;

use reqwest::Url;

use super::DynError;

/// Deployer-neutral node access facts discovered at runtime.
#[derive(Clone, Debug, Default)]
pub struct NodeAccess {
    host: String,
    api_port: u16,
    testing_port: Option<u16>,
    named_ports: HashMap<String, u16>,
}

impl NodeAccess {
    #[must_use]
    pub fn new(host: impl Into<String>, api_port: u16) -> Self {
        Self {
            host: host.into(),
            api_port,
            testing_port: None,
            named_ports: HashMap::new(),
        }
    }

    #[must_use]
    pub fn with_testing_port(mut self, port: u16) -> Self {
        self.testing_port = Some(port);
        self
    }

    #[must_use]
    pub fn with_named_port(mut self, name: impl Into<String>, port: u16) -> Self {
        self.named_ports.insert(name.into(), port);
        self
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub fn api_port(&self) -> u16 {
        self.api_port
    }

    #[must_use]
    pub fn testing_port(&self) -> Option<u16> {
        self.testing_port
    }

    #[must_use]
    pub fn named_port(&self, name: &str) -> Option<u16> {
        self.named_ports.get(name).copied()
    }

    pub fn api_base_url(&self) -> Result<Url, DynError> {
        Ok(Url::parse(&format!(
            "http://{}:{}",
            self.host, self.api_port
        ))?)
    }
}

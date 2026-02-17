use std::{env, path::Path};

use async_trait::async_trait;
use reqwest::Url;
use testing_framework_core::scenario::{
    Application, DynError, HttpReadinessRequirement, NodeClients,
    wait_for_http_ports_with_host_and_requirement, wait_http_readiness,
};

use crate::{
    descriptor::{ComposeDescriptor, NodeDescriptor},
    infrastructure::ports::{HostPortMapping, NodeContainerPorts, NodeHostPorts},
};

/// Handle returned by a compose config server (cfgsync or equivalent).
pub trait ConfigServerHandle: Send + Sync {
    fn shutdown(&mut self);
    fn mark_preserved(&mut self);
    fn container_name(&self) -> Option<&str> {
        None
    }
}

/// Compose-specific topology surface needed by the runner.
#[async_trait]
pub trait ComposeDeployEnv: Application {
    type ConfigHandle: ConfigServerHandle;

    /// Produce the compose descriptor for the given topology.
    fn compose_descriptor(topology: &Self::Deployment, cfgsync_port: u16) -> ComposeDescriptor;

    /// Container ports (API/testing) per node, used for docker-compose port
    /// discovery.
    fn node_container_ports(topology: &Self::Deployment) -> Vec<NodeContainerPorts> {
        let descriptor = Self::compose_descriptor(topology, 0);
        descriptor
            .nodes()
            .iter()
            .enumerate()
            .filter_map(|(index, node)| parse_node_container_ports(index, node))
            .collect()
    }

    /// Update the config server template based on topology.
    fn update_cfgsync_config(
        path: &Path,
        topology: &Self::Deployment,
        port: u16,
        metrics_otlp_ingest_url: Option<&Url>,
    ) -> Result<(), DynError>;

    /// Start the config server and return its handle.
    async fn start_cfgsync(
        cfgsync_path: &Path,
        port: u16,
        network: &str,
    ) -> Result<Self::ConfigHandle, DynError>;

    /// Build node clients from discovered host ports.
    fn node_client_from_ports(
        ports: &NodeHostPorts,
        host: &str,
    ) -> Result<Self::NodeClient, DynError>;

    /// Build node clients from discovered host ports.
    fn build_node_clients(
        _topology: &Self::Deployment,
        host_ports: &HostPortMapping,
        host: &str,
    ) -> Result<NodeClients<Self>, DynError>
    where
        Self: Sized,
    {
        let clients = host_ports
            .nodes
            .iter()
            .map(|ports| Self::node_client_from_ports(ports, host))
            .collect::<Result<_, _>>()?;
        Ok(NodeClients::new(clients))
    }

    /// Return the compose image name and optional platform override.
    ///
    /// Defaults:
    /// - image: `COMPOSE_RUNNER_IMAGE` or `logos-blockchain-testing:local`
    /// - platform: `COMPOSE_RUNNER_PLATFORM` when set
    fn compose_image() -> (String, Option<String>) {
        let image = compose_image_from_env();
        let platform = env::var("COMPOSE_RUNNER_PLATFORM").ok();
        (image, platform)
    }

    /// Path used by default readiness checks.
    fn readiness_path() -> &'static str {
        "/"
    }

    /// Host used by default remote readiness checks.
    fn compose_runner_host() -> String {
        "127.0.0.1".to_string()
    }

    /// Remote readiness probe for node APIs.
    async fn wait_remote_readiness(
        _topology: &Self::Deployment,
        mapping: &HostPortMapping,
        requirement: HttpReadinessRequirement,
    ) -> Result<(), DynError> {
        let host = Self::compose_runner_host();
        let urls = readiness_urls(&host, mapping, Self::readiness_path())?;
        wait_http_readiness(&urls, requirement).await?;
        Ok(())
    }

    /// Wait for HTTP readiness on node ports.
    async fn wait_for_nodes(
        ports: &[u16],
        host: &str,
        requirement: HttpReadinessRequirement,
    ) -> Result<(), DynError> {
        wait_for_http_ports_with_host_and_requirement(
            ports,
            host,
            Self::readiness_path(),
            requirement,
        )
        .await?;
        Ok(())
    }
}

fn parse_container_port(entry: &str) -> Option<u16> {
    entry.rsplit(':').next()?.parse().ok()
}

fn compose_image_from_env() -> String {
    env::var("COMPOSE_RUNNER_IMAGE")
        .unwrap_or_else(|_| String::from("logos-blockchain-testing:local"))
}

fn parse_node_container_ports(index: usize, node: &NodeDescriptor) -> Option<NodeContainerPorts> {
    let mut ports = node
        .ports()
        .iter()
        .filter_map(|entry| parse_container_port(entry));
    let api = ports.next()?;
    let testing = ports.next()?;

    Some(NodeContainerPorts {
        index,
        api,
        testing,
    })
}

fn readiness_urls(
    host: &str,
    mapping: &HostPortMapping,
    endpoint_path: &str,
) -> Result<Vec<Url>, DynError> {
    let endpoint_path = normalize_endpoint_path(endpoint_path);

    mapping
        .nodes
        .iter()
        .map(|ports| readiness_url(host, ports.api, &endpoint_path))
        .collect::<Result<_, _>>()
}

fn normalize_endpoint_path(endpoint_path: &str) -> String {
    if endpoint_path.starts_with('/') {
        endpoint_path.to_string()
    } else {
        format!("/{endpoint_path}")
    }
}

fn readiness_url(host: &str, api_port: u16, endpoint_path: &str) -> Result<Url, DynError> {
    let url = Url::parse(&format!("http://{host}:{api_port}{endpoint_path}"))?;
    Ok(url)
}

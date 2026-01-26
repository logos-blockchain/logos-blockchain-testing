use std::time::Duration;

use anyhow::{Context as _, anyhow};
use reqwest::Url;
use testing_framework_core::{
    adjust_timeout, scenario::http_probe::NODE_ROLE, topology::generation::GeneratedTopology,
};
use tokio::{process::Command, time::timeout};
use tracing::{debug, info};
use url::ParseError;

use crate::{
    errors::{ComposeRunnerError, StackReadinessError},
    infrastructure::environment::StackEnvironment,
};

const COMPOSE_PORT_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);

/// Host ports mapped for a single node.
#[derive(Clone, Debug)]
pub struct NodeHostPorts {
    pub api: u16,
    pub testing: u16,
}

/// All host port mappings for nodes.
#[derive(Clone, Debug)]
pub struct HostPortMapping {
    pub nodes: Vec<NodeHostPorts>,
}

impl HostPortMapping {
    /// Returns API ports for all nodes.
    pub fn node_api_ports(&self) -> Vec<u16> {
        self.nodes.iter().map(|ports| ports.api).collect()
    }
}

/// Resolve host ports for all nodes from docker compose.
pub async fn discover_host_ports(
    environment: &StackEnvironment,
    descriptors: &GeneratedTopology,
) -> Result<HostPortMapping, ComposeRunnerError> {
    debug!(
        compose_file = %environment.compose_path().display(),
        project = environment.project_name(),
        nodes = descriptors.nodes().len(),
        "resolving compose host ports"
    );
    let mut nodes = Vec::new();
    for node in descriptors.nodes() {
        let service = node_identifier(node.index());
        let api = resolve_service_port(environment, &service, node.api_port()).await?;
        let testing = resolve_service_port(environment, &service, node.testing_http_port()).await?;
        nodes.push(NodeHostPorts { api, testing });
    }

    let mapping = HostPortMapping { nodes };

    info!(
        node_ports = ?mapping.nodes,
        "compose host ports resolved"
    );

    Ok(mapping)
}

async fn resolve_service_port(
    environment: &StackEnvironment,
    service: &str,
    container_port: u16,
) -> Result<u16, ComposeRunnerError> {
    let mut cmd = Command::new("docker");
    cmd.arg("compose")
        .arg("-f")
        .arg(environment.compose_path())
        .arg("-p")
        .arg(environment.project_name())
        .arg("port")
        .arg(service)
        .arg(container_port.to_string())
        .current_dir(environment.root());

    let output = timeout(adjust_timeout(COMPOSE_PORT_DISCOVERY_TIMEOUT), cmd.output())
        .await
        .map_err(|_| ComposeRunnerError::PortDiscovery {
            service: service.to_owned(),
            container_port,
            source: anyhow!("docker compose port timed out"),
        })?
        .with_context(|| format!("running docker compose port {service} {container_port}"))
        .map_err(|source| ComposeRunnerError::PortDiscovery {
            service: service.to_owned(),
            container_port,
            source,
        })?;

    if !output.status.success() {
        return Err(ComposeRunnerError::PortDiscovery {
            service: service.to_owned(),
            container_port,
            source: anyhow!("docker compose port exited with {}", output.status),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(port_str) = line.rsplit(':').next()
            && let Ok(port) = port_str.trim().parse::<u16>()
        {
            return Ok(port);
        }
    }

    Err(ComposeRunnerError::PortDiscovery {
        service: service.to_owned(),
        container_port,
        source: anyhow!("unable to parse docker compose port output: {stdout}"),
    })
}

/// Wait for remote readiness using mapped host ports.
pub async fn ensure_remote_readiness_with_ports(
    descriptors: &GeneratedTopology,
    mapping: &HostPortMapping,
) -> Result<(), StackReadinessError> {
    let node_urls = mapping
        .nodes
        .iter()
        .map(|ports| readiness_url(NODE_ROLE, ports.api))
        .collect::<Result<Vec<_>, _>>()?;

    descriptors
        .wait_remote_readiness(&node_urls)
        .await
        .map_err(|source| StackReadinessError::Remote { source })
}

fn readiness_url(role: &'static str, port: u16) -> Result<Url, StackReadinessError> {
    localhost_url(port).map_err(|source| StackReadinessError::Endpoint { role, port, source })
}

fn localhost_url(port: u16) -> Result<Url, ParseError> {
    Url::parse(&format!("http://{}:{port}/", compose_runner_host()))
}

fn node_identifier(index: usize) -> String {
    format!("node-{index}")
}

pub(crate) fn compose_runner_host() -> String {
    let host = std::env::var("COMPOSE_RUNNER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    debug!(host, "compose runner host resolved for readiness URLs");
    host
}

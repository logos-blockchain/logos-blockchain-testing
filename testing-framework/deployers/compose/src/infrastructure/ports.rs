use std::time::Duration;

use anyhow::{Context as _, anyhow};
use reqwest::Url;
use testing_framework_core::{
    adjust_timeout,
    scenario::http_probe::NodeRole as HttpNodeRole,
    topology::generation::{GeneratedTopology, NodeRole as TopologyNodeRole},
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

/// All host port mappings for validators and executors.
#[derive(Clone, Debug)]
pub struct HostPortMapping {
    pub validators: Vec<NodeHostPorts>,
    pub executors: Vec<NodeHostPorts>,
}

impl HostPortMapping {
    /// Returns API ports for all validators.
    pub fn validator_api_ports(&self) -> Vec<u16> {
        self.validators.iter().map(|ports| ports.api).collect()
    }

    /// Returns API ports for all executors.
    pub fn executor_api_ports(&self) -> Vec<u16> {
        self.executors.iter().map(|ports| ports.api).collect()
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
        validators = descriptors.validators().len(),
        executors = descriptors.executors().len(),
        "resolving compose host ports"
    );
    let mut validators = Vec::new();
    for node in descriptors.validators() {
        let service = node_identifier(TopologyNodeRole::Validator, node.index());
        let api = resolve_service_port(environment, &service, node.api_port()).await?;
        let testing = resolve_service_port(environment, &service, node.testing_http_port()).await?;
        validators.push(NodeHostPorts { api, testing });
    }

    let mut executors = Vec::new();
    for node in descriptors.executors() {
        let service = node_identifier(TopologyNodeRole::Executor, node.index());
        let api = resolve_service_port(environment, &service, node.api_port()).await?;
        let testing = resolve_service_port(environment, &service, node.testing_http_port()).await?;
        executors.push(NodeHostPorts { api, testing });
    }

    let mapping = HostPortMapping {
        validators,
        executors,
    };

    info!(
        validator_ports = ?mapping.validators,
        executor_ports = ?mapping.executors,
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
    let validator_urls = mapping
        .validators
        .iter()
        .map(|ports| readiness_url(HttpNodeRole::Validator, ports.api))
        .collect::<Result<Vec<_>, _>>()?;
    let executor_urls = mapping
        .executors
        .iter()
        .map(|ports| readiness_url(HttpNodeRole::Executor, ports.api))
        .collect::<Result<Vec<_>, _>>()?;

    descriptors
        .wait_remote_readiness(&validator_urls, &executor_urls, None, None)
        .await
        .map_err(|source| StackReadinessError::Remote { source })
}

fn readiness_url(role: HttpNodeRole, port: u16) -> Result<Url, StackReadinessError> {
    localhost_url(port).map_err(|source| StackReadinessError::Endpoint { role, port, source })
}

fn localhost_url(port: u16) -> Result<Url, ParseError> {
    Url::parse(&format!("http://{}:{port}/", compose_runner_host()))
}

fn node_identifier(role: TopologyNodeRole, index: usize) -> String {
    match role {
        TopologyNodeRole::Validator => format!("validator-{index}"),
        TopologyNodeRole::Executor => format!("executor-{index}"),
    }
}

pub(crate) fn compose_runner_host() -> String {
    let host = std::env::var("COMPOSE_RUNNER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    debug!(host, "compose runner host resolved for readiness URLs");
    host
}

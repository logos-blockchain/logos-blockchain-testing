use std::{env, path::Path, process::Output, time::Duration};

use anyhow::{Context as _, anyhow};
use testing_framework_core::adjust_timeout;
use tokio::{process::Command, time::timeout};
use tracing::{debug, info};

use crate::{errors::ComposeRunnerError, infrastructure::environment::StackEnvironment};

const COMPOSE_PORT_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);

/// Host ports mapped for a single node.
#[derive(Clone, Debug)]
pub struct NodeHostPorts {
    pub api: u16,
    pub testing: u16,
}

/// Container ports for a single node.
#[derive(Clone, Debug)]
pub struct NodeContainerPorts {
    pub index: usize,
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
    nodes: &[NodeContainerPorts],
) -> Result<HostPortMapping, ComposeRunnerError> {
    debug!(
        compose_file = %environment.compose_path().display(),
        project = environment.project_name(),
        nodes = nodes.len(),
        "resolving compose host ports"
    );
    let mut host_nodes = Vec::with_capacity(nodes.len());
    for node in nodes {
        host_nodes.push(resolve_node_ports(environment, node).await?);
    }

    let mapping = HostPortMapping { nodes: host_nodes };

    info!(
        node_ports = ?mapping.nodes,
        "compose host ports resolved"
    );

    Ok(mapping)
}

async fn resolve_node_ports(
    environment: &StackEnvironment,
    node: &NodeContainerPorts,
) -> Result<NodeHostPorts, ComposeRunnerError> {
    let service = node_identifier(node.index);
    let api = resolve_service_port(environment, &service, node.api).await?;
    let testing = resolve_service_port(environment, &service, node.testing).await?;

    Ok(NodeHostPorts { api, testing })
}

async fn resolve_service_port(
    environment: &StackEnvironment,
    service: &str,
    container_port: u16,
) -> Result<u16, ComposeRunnerError> {
    environment
        .project()
        .resolve_service_port(service, container_port)
        .await
}

pub(crate) async fn resolve_service_port_with(
    compose_path: &Path,
    project_name: &str,
    root: &Path,
    service: &str,
    container_port: u16,
) -> Result<u16, ComposeRunnerError> {
    let mut cmd =
        docker_compose_port_command(compose_path, project_name, root, service, container_port);
    let output = run_port_discovery_command(&mut cmd, service, container_port).await?;
    parse_port_from_output(service, container_port, &output)
}

pub fn node_identifier(index: usize) -> String {
    format!("node-{index}")
}

pub fn compose_runner_host() -> String {
    let host = env::var("COMPOSE_RUNNER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    debug!(host, "compose runner host resolved for readiness URLs");
    host
}

fn docker_compose_port_command(
    compose_path: &Path,
    project_name: &str,
    root: &Path,
    service: &str,
    container_port: u16,
) -> Command {
    let mut cmd = Command::new("docker");
    cmd.arg("compose")
        .arg("-f")
        .arg(compose_path)
        .arg("-p")
        .arg(project_name)
        .arg("port")
        .arg(service)
        .arg(container_port.to_string())
        .current_dir(root);
    cmd
}

async fn run_port_discovery_command(
    cmd: &mut Command,
    service: &str,
    container_port: u16,
) -> Result<Output, ComposeRunnerError> {
    timeout(adjust_timeout(COMPOSE_PORT_DISCOVERY_TIMEOUT), cmd.output())
        .await
        .map_err(|_| {
            port_discovery_error(
                service,
                container_port,
                anyhow!("docker compose port timed out"),
            )
        })?
        .with_context(|| format!("running docker compose port {service} {container_port}"))
        .map_err(|source| port_discovery_error(service, container_port, source))
}

fn parse_port_from_output(
    service: &str,
    container_port: u16,
    output: &Output,
) -> Result<u16, ComposeRunnerError> {
    if !output.status.success() {
        return Err(port_discovery_error(
            service,
            container_port,
            anyhow!("docker compose port exited with {}", output.status),
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_mapped_port(&stdout).ok_or_else(|| {
        port_discovery_error(
            service,
            container_port,
            anyhow!("unable to parse docker compose port output: {stdout}"),
        )
    })
}

fn port_discovery_error(
    service: &str,
    container_port: u16,
    source: anyhow::Error,
) -> ComposeRunnerError {
    ComposeRunnerError::PortDiscovery {
        service: service.to_owned(),
        container_port,
        source,
    }
}

fn parse_mapped_port(stdout: &str) -> Option<u16> {
    stdout.lines().map(str::trim).find_map(parse_port_line)
}

fn parse_port_line(line: &str) -> Option<u16> {
    if line.is_empty() {
        return None;
    }

    line.rsplit(':').next()?.trim().parse::<u16>().ok()
}

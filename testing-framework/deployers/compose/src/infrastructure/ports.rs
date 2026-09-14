use std::{env, path::Path, process::Output, time::Duration};

use anyhow::{Context as _, anyhow};
use testing_framework_core::adjust_timeout;
use tokio::{process::Command, time::timeout};
use tracing::{debug, info};

use crate::{errors::ComposeRunnerError, infrastructure::project::ComposeProject};

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
pub(crate) async fn discover_host_ports(
    project: &ComposeProject,
    nodes: &[NodeContainerPorts],
) -> Result<HostPortMapping, ComposeRunnerError> {
    debug!(
        compose_file = %project.compose_file().display(),
        project = project.name(),
        nodes = nodes.len(),
        "resolving compose host ports"
    );
    let mut host_nodes = Vec::with_capacity(nodes.len());
    for node in nodes {
        host_nodes.push(resolve_node_ports(project, node).await?);
    }

    let mapping = HostPortMapping { nodes: host_nodes };

    info!(
        node_ports = ?mapping.nodes,
        "compose host ports resolved"
    );

    Ok(mapping)
}

async fn resolve_node_ports(
    project: &ComposeProject,
    node: &NodeContainerPorts,
) -> Result<NodeHostPorts, ComposeRunnerError> {
    let service = node_identifier(node.index);
    let api = project.resolve_service_port(&service, node.api).await?;
    let testing = project.resolve_service_port(&service, node.testing).await?;

    Ok(NodeHostPorts { api, testing })
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

tokio::task_local! {
    /// Cluster namespace applied to node identifiers for the duration of one
    /// managed provisioning attempt.
    static SERVICE_NAMESPACE: Option<String>;
}

/// Runs `future` with every [`node_identifier`] call namespaced by the given
/// cluster name; `None` keeps the plain `node-{index}` convention.
pub(crate) async fn with_service_namespace<F: Future>(
    namespace: Option<String>,
    future: F,
) -> F::Output {
    SERVICE_NAMESPACE.scope(namespace, future).await
}

fn service_namespace() -> Option<String> {
    SERVICE_NAMESPACE.try_with(Clone::clone).ok().flatten()
}

/// Returns the compose service name for one managed node.
///
/// Within a named-cluster provisioning attempt the identifier carries the
/// cluster namespace (for example `alpha-node-0`), so descriptors, config
/// hostnames, and port discovery agree on the same service names.
pub fn node_identifier(index: usize) -> String {
    service_namespace().map_or_else(
        || format!("node-{index}"),
        |namespace| format!("{namespace}-node-{index}"),
    )
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

#[cfg(test)]
mod tests {
    use super::{node_identifier, with_service_namespace};

    #[test]
    fn node_identifier_defaults_to_plain_names() {
        assert_eq!(node_identifier(2), "node-2");
    }

    #[tokio::test]
    async fn node_identifier_carries_the_cluster_namespace() {
        let namespaced =
            with_service_namespace(Some("alpha".to_owned()), async { node_identifier(0) }).await;
        let plain = with_service_namespace(None, async { node_identifier(0) }).await;

        assert_eq!(namespaced, "alpha-node-0");
        assert_eq!(plain, "node-0");
        assert_eq!(node_identifier(0), "node-0");
    }
}

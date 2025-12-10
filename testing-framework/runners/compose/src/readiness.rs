use std::time::Duration;

use reqwest::Url;
use testing_framework_core::{
    nodes::ApiClient,
    scenario::{Metrics, MetricsError, NodeClients, http_probe::NodeRole as HttpNodeRole},
    topology::generation::{GeneratedTopology, NodeRole as TopologyNodeRole},
};
use tokio::time::sleep;

use crate::{
    compose::{HostPortMapping, NodeHostPorts},
    errors::{NodeClientError, StackReadinessError},
    wait::{wait_for_executors, wait_for_validators},
};

const DISABLED_READINESS_SLEEP: Duration = Duration::from_secs(5);

/// Build a metrics client from host/port, validating the URL.
pub fn metrics_handle_from_port(port: u16, host: &str) -> Result<Metrics, MetricsError> {
    let url = Url::parse(&format!("http://{host}:{port}/"))
        .map_err(|err| MetricsError::new(format!("invalid prometheus url: {err}")))?;
    Metrics::from_prometheus(url)
}

/// Wait until all validators respond on their API ports.
pub async fn ensure_validators_ready_with_ports(ports: &[u16]) -> Result<(), StackReadinessError> {
    if ports.is_empty() {
        return Ok(());
    }

    wait_for_validators(ports).await.map_err(Into::into)
}

/// Wait until all executors respond on their API ports.
pub async fn ensure_executors_ready_with_ports(ports: &[u16]) -> Result<(), StackReadinessError> {
    if ports.is_empty() {
        return Ok(());
    }

    wait_for_executors(ports).await.map_err(Into::into)
}

/// Allow a brief pause when readiness probes are disabled.
pub async fn maybe_sleep_for_disabled_readiness(readiness_enabled: bool) {
    if !readiness_enabled {
        sleep(DISABLED_READINESS_SLEEP).await;
    }
}

/// Construct API clients using the mapped host ports.
pub fn build_node_clients_with_ports(
    descriptors: &GeneratedTopology,
    mapping: &HostPortMapping,
    host: &str,
) -> Result<NodeClients, NodeClientError> {
    let validators = descriptors
        .validators()
        .iter()
        .zip(mapping.validators.iter())
        .map(|(node, ports)| api_client_from_host_ports(to_http_role(node.role()), ports, host))
        .collect::<Result<Vec<_>, _>>()?;
    let executors = descriptors
        .executors()
        .iter()
        .zip(mapping.executors.iter())
        .map(|(node, ports)| api_client_from_host_ports(to_http_role(node.role()), ports, host))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(NodeClients::new(validators, executors))
}

fn api_client_from_host_ports(
    role: HttpNodeRole,
    ports: &NodeHostPorts,
    host: &str,
) -> Result<ApiClient, NodeClientError> {
    let base_url = localhost_url(ports.api, host).map_err(|source| NodeClientError::Endpoint {
        role,
        endpoint: "api",
        port: ports.api,
        source,
    })?;

    let testing_url =
        Some(
            localhost_url(ports.testing, host).map_err(|source| NodeClientError::Endpoint {
                role,
                endpoint: "testing",
                port: ports.testing,
                source,
            })?,
        );

    Ok(ApiClient::from_urls(base_url, testing_url))
}

fn to_http_role(role: TopologyNodeRole) -> testing_framework_core::scenario::http_probe::NodeRole {
    match role {
        TopologyNodeRole::Validator => HttpNodeRole::Validator,
        TopologyNodeRole::Executor => HttpNodeRole::Executor,
    }
}

fn localhost_url(port: u16, host: &str) -> Result<Url, url::ParseError> {
    Url::parse(&format!("http://{host}:{port}/"))
}

use std::time::Duration;

use reqwest::Url;
use testing_framework_core::{
    nodes::ApiClient,
    scenario::{NodeClients, http_probe::NodeKind as HttpNodeKind},
    topology::generation::GeneratedTopology,
};
use tokio::time::sleep;

use crate::{
    errors::{NodeClientError, StackReadinessError},
    infrastructure::ports::{HostPortMapping, NodeHostPorts},
    lifecycle::wait::wait_for_nodes,
};

const DISABLED_READINESS_SLEEP: Duration = Duration::from_secs(5);

/// Wait until all nodes respond on their API ports.
pub async fn ensure_nodes_ready_with_ports(ports: &[u16]) -> Result<(), StackReadinessError> {
    if ports.is_empty() {
        return Ok(());
    }

    wait_for_nodes(ports).await.map_err(Into::into)
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
    let nodes = descriptors
        .nodes()
        .iter()
        .zip(mapping.nodes.iter())
        .map(|(_node, ports)| api_client_from_host_ports(HttpNodeKind::Node, ports, host))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(NodeClients::new(nodes))
}

fn api_client_from_host_ports(
    role: HttpNodeKind,
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

fn localhost_url(port: u16, host: &str) -> Result<Url, url::ParseError> {
    Url::parse(&format!("http://{host}:{port}/"))
}

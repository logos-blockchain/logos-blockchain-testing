use testing_framework_core::scenario::http_probe::{self, HttpReadinessError, NodeRole};

use super::{ClusterWaitError, HTTP_POLL_INTERVAL, NODE_HTTP_PROBE_TIMEOUT, NODE_HTTP_TIMEOUT};
use crate::host::node_host;

pub async fn wait_for_node_http_nodeport(
    ports: &[u16],
    role: NodeRole,
) -> Result<(), ClusterWaitError> {
    let host = node_host();
    wait_for_node_http_on_host(ports, role, &host, NODE_HTTP_PROBE_TIMEOUT).await
}

pub async fn wait_for_node_http_port_forward(
    ports: &[u16],
    role: NodeRole,
) -> Result<(), ClusterWaitError> {
    wait_for_node_http_on_host(ports, role, "127.0.0.1", NODE_HTTP_TIMEOUT).await
}

async fn wait_for_node_http_on_host(
    ports: &[u16],
    role: NodeRole,
    host: &str,
    timeout: std::time::Duration,
) -> Result<(), ClusterWaitError> {
    http_probe::wait_for_http_ports_with_host(ports, role, host, timeout, HTTP_POLL_INTERVAL)
        .await
        .map_err(map_http_error)
}

const fn map_http_error(error: HttpReadinessError) -> ClusterWaitError {
    ClusterWaitError::NodeHttpTimeout {
        role: error.role(),
        port: error.port(),
        timeout: error.timeout(),
    }
}

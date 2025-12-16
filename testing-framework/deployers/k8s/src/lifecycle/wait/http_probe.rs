use testing_framework_core::scenario::http_probe::{self, HttpReadinessError, NodeRole};

use super::{ClusterWaitError, http_poll_interval, node_http_probe_timeout, node_http_timeout};
use crate::host::node_host;

pub async fn wait_for_node_http_nodeport(
    ports: &[u16],
    role: NodeRole,
) -> Result<(), ClusterWaitError> {
    let host = node_host();
    wait_for_node_http_on_host(ports, role, &host, node_http_probe_timeout()).await
}

const LOCALHOST: &str = "127.0.0.1";

pub async fn wait_for_node_http_port_forward(
    ports: &[u16],
    role: NodeRole,
) -> Result<(), ClusterWaitError> {
    wait_for_node_http_on_host(ports, role, LOCALHOST, node_http_timeout()).await
}

async fn wait_for_node_http_on_host(
    ports: &[u16],
    role: NodeRole,
    host: &str,
    timeout: std::time::Duration,
) -> Result<(), ClusterWaitError> {
    http_probe::wait_for_http_ports_with_host(ports, role, host, timeout, http_poll_interval())
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

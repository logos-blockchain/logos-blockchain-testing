use tokio::time::sleep;

use super::{ClusterWaitError, node_http_probe_timeout, node_http_timeout};
use crate::host::node_host;

const GRAFANA_HTTP_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

pub async fn wait_for_grafana_http_nodeport(port: u16) -> Result<(), ClusterWaitError> {
    let host = node_host();
    wait_for_grafana_http(&host, port, node_http_probe_timeout()).await
}

pub async fn wait_for_grafana_http_port_forward(port: u16) -> Result<(), ClusterWaitError> {
    wait_for_grafana_http("127.0.0.1", port, node_http_timeout()).await
}

async fn wait_for_grafana_http(
    host: &str,
    port: u16,
    timeout: std::time::Duration,
) -> Result<(), ClusterWaitError> {
    let client = reqwest::Client::new();
    let url = format!("http://{host}:{port}/api/health");

    let attempts = timeout.as_secs();
    for _ in 0..attempts {
        if let Ok(resp) = client.get(&url).send().await
            && resp.status().is_success()
        {
            return Ok(());
        }
        sleep(GRAFANA_HTTP_POLL_INTERVAL).await;
    }

    Err(ClusterWaitError::GrafanaTimeout { port })
}

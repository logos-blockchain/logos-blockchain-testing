use tokio::time::sleep;

use super::{ClusterWaitError, prometheus_http_timeout};
use crate::host::node_host;

const PROMETHEUS_HTTP_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

pub async fn wait_for_prometheus_http_nodeport(
    port: u16,
    timeout: std::time::Duration,
) -> Result<(), ClusterWaitError> {
    let host = node_host();
    wait_for_prometheus_http(&host, port, timeout).await
}

pub async fn wait_for_prometheus_http_port_forward(port: u16) -> Result<(), ClusterWaitError> {
    wait_for_prometheus_http("127.0.0.1", port, prometheus_http_timeout()).await
}

async fn wait_for_prometheus_http(
    host: &str,
    port: u16,
    timeout: std::time::Duration,
) -> Result<(), ClusterWaitError> {
    let client = reqwest::Client::new();
    let url = format!("http://{host}:{port}/-/ready");

    let attempts = timeout.as_secs();
    for _ in 0..attempts {
        if let Ok(resp) = client.get(&url).send().await
            && resp.status().is_success()
        {
            return Ok(());
        }
        sleep(PROMETHEUS_HTTP_POLL_INTERVAL).await;
    }

    Err(ClusterWaitError::PrometheusTimeout { port })
}

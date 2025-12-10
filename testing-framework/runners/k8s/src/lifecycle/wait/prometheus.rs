use tokio::time::sleep;

use super::{ClusterWaitError, PROMETHEUS_HTTP_TIMEOUT};
use crate::host::node_host;

pub async fn wait_for_prometheus_http_nodeport(
    port: u16,
    timeout: std::time::Duration,
) -> Result<(), ClusterWaitError> {
    let host = node_host();
    wait_for_prometheus_http(&host, port, timeout).await
}

pub async fn wait_for_prometheus_http_port_forward(port: u16) -> Result<(), ClusterWaitError> {
    wait_for_prometheus_http("127.0.0.1", port, PROMETHEUS_HTTP_TIMEOUT).await
}

async fn wait_for_prometheus_http(
    host: &str,
    port: u16,
    timeout: std::time::Duration,
) -> Result<(), ClusterWaitError> {
    let client = reqwest::Client::new();
    let url = format!("http://{host}:{port}/-/ready");

    for _ in 0..timeout.as_secs() {
        if let Ok(resp) = client.get(&url).send().await
            && resp.status().is_success()
        {
            return Ok(());
        }
        sleep(std::time::Duration::from_secs(1)).await;
    }

    Err(ClusterWaitError::PrometheusTimeout { port })
}

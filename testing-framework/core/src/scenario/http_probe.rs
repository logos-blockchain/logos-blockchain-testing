use std::time::Duration;

use futures::future::try_join_all;
use nomos_http_api_common::paths;
use reqwest::Client as ReqwestClient;
use thiserror::Error;
use tokio::time::{Instant, sleep};
use tracing::{debug, info};

/// Error raised when HTTP readiness checks time out.
#[derive(Clone, Copy, Debug, Error)]
#[error("timeout waiting for {role} HTTP endpoint on port {port} after {timeout:?}")]
pub struct HttpReadinessError {
    role: &'static str,
    port: u16,
    timeout: Duration,
}

impl HttpReadinessError {
    #[must_use]
    pub const fn new(role: &'static str, port: u16, timeout: Duration) -> Self {
        Self {
            role,
            port,
            timeout,
        }
    }

    #[must_use]
    pub const fn role(&self) -> &'static str {
        self.role
    }

    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    #[must_use]
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }
}

/// Wait for HTTP readiness on the provided ports against localhost.
pub async fn wait_for_http_ports(
    ports: &[u16],
    role: &'static str,
    timeout_duration: Duration,
    poll_interval: Duration,
) -> Result<(), HttpReadinessError> {
    wait_for_http_ports_with_host(ports, role, "127.0.0.1", timeout_duration, poll_interval).await
}

/// Wait for HTTP readiness on the provided ports against a specific host.
pub async fn wait_for_http_ports_with_host(
    ports: &[u16],
    role: &'static str,
    host: &str,
    timeout_duration: Duration,
    poll_interval: Duration,
) -> Result<(), HttpReadinessError> {
    if ports.is_empty() {
        return Ok(());
    }

    info!(
        role,
        ?ports,
        host,
        timeout_secs = timeout_duration.as_secs_f32(),
        poll_ms = poll_interval.as_millis(),
        "waiting for HTTP readiness"
    );

    let client = ReqwestClient::new();
    let probes = ports.iter().copied().map(|port| {
        wait_for_single_port(
            client.clone(),
            port,
            role,
            host,
            timeout_duration,
            poll_interval,
        )
    });

    try_join_all(probes).await.map(|_| ())
}

async fn wait_for_single_port(
    client: ReqwestClient,
    port: u16,
    role: &'static str,
    host: &str,
    timeout_duration: Duration,
    poll_interval: Duration,
) -> Result<(), HttpReadinessError> {
    let url = format!("http://{host}:{port}{}", paths::CRYPTARCHIA_INFO);
    debug!(role, %url, "probing HTTP endpoint");
    let start = Instant::now();
    let deadline = start + timeout_duration;
    let mut attempts: u64 = 0;

    loop {
        attempts += 1;

        let last_failure: Option<String> = match client.get(&url).send().await {
            Ok(response) if response.status().is_success() => {
                info!(
                    role,
                    port,
                    host,
                    %url,
                    attempts,
                    elapsed_ms = start.elapsed().as_millis(),
                    "HTTP readiness confirmed"
                );
                return Ok(());
            }
            Ok(response) => {
                let status = response.status();
                Some(format!("HTTP {status}"))
            }
            Err(error) => Some(format!("request error: {error}")),
        };

        if attempts == 1 || attempts % 10 == 0 {
            debug!(
                role,
                port,
                host,
                %url,
                attempts,
                elapsed_ms = start.elapsed().as_millis(),
                last_failure = last_failure.as_deref().unwrap_or("<none>"),
                "HTTP readiness not yet available"
            );
        }

        if Instant::now() >= deadline {
            info!(
                role,
                port,
                host,
                %url,
                attempts,
                elapsed_ms = start.elapsed().as_millis(),
                timeout_secs = timeout_duration.as_secs_f32(),
                last_failure = last_failure.as_deref().unwrap_or("<none>"),
                "HTTP readiness timed out"
            );
            return Err(HttpReadinessError::new(role, port, timeout_duration));
        }

        sleep(poll_interval).await;
    }
}

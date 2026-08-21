use std::{
    future::Future,
    time::{Duration, Instant},
};

use reqwest::{Client, Url};
use thiserror::Error;

use crate::{adjust_timeout, scenario::DynError};

const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(200);
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const LOCALHOST: &str = "127.0.0.1";
const NO_STABILIZATION_DETAILS: &str = "no probe details reported";
const NO_FAILING_ENDPOINTS: &str = "<none>";

#[derive(Debug, Error)]
pub enum ReadinessError {
    #[error("readiness probe timed out: {message}")]
    ProbeTimeout { message: String },
    #[error("invalid readiness endpoint '{endpoint}': {reason}")]
    InvalidEndpoint { endpoint: String, reason: String },
    #[error("cluster stabilization failed: {source}")]
    ClusterStable {
        #[source]
        source: DynError,
    },
    #[error("cluster stabilization timed out after {timeout:?}: {details}")]
    StabilizationTimeout { timeout: Duration, details: String },
    #[error("cluster stabilization probe failed: {source}")]
    StabilizationProbe {
        #[source]
        source: DynError,
    },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum HttpReadinessRequirement {
    AllNodesReady,
    AnyNodeReady,
    AtLeast(usize),
}

#[derive(Debug)]
struct HttpProbeStatus {
    endpoint: Url,
    ok: bool,
    detail: String,
}

#[derive(Debug, Clone, Copy)]
pub struct StabilizationConfig {
    pub timeout: Duration,
    pub poll_interval: Duration,
}

impl StabilizationConfig {
    #[must_use]
    pub const fn new(timeout: Duration, poll_interval: Duration) -> Self {
        Self {
            timeout,
            poll_interval,
        }
    }
}

fn normalize_endpoint_path(endpoint_path: &str) -> String {
    if endpoint_path.starts_with('/') {
        endpoint_path.to_string()
    } else {
        format!("/{endpoint_path}")
    }
}

fn build_local_endpoints(ports: &[u16], endpoint_path: &str) -> Result<Vec<Url>, ReadinessError> {
    build_endpoints_with_host(ports, LOCALHOST, endpoint_path)
}

fn build_endpoints_with_host(
    ports: &[u16],
    host: &str,
    endpoint_path: &str,
) -> Result<Vec<Url>, ReadinessError> {
    let endpoint_path = normalize_endpoint_path(endpoint_path);

    ports
        .iter()
        .map(|port| format!("http://{host}:{port}{endpoint_path}"))
        .map(|endpoint| {
            Url::parse(&endpoint).map_err(|source| ReadinessError::InvalidEndpoint {
                endpoint,
                reason: source.to_string(),
            })
        })
        .collect()
}

fn requirement_satisfied(
    statuses: &[HttpProbeStatus],
    requirement: HttpReadinessRequirement,
) -> bool {
    let ready = ready_count(statuses);
    match requirement {
        HttpReadinessRequirement::AllNodesReady => ready == statuses.len(),
        HttpReadinessRequirement::AnyNodeReady => ready >= 1,
        HttpReadinessRequirement::AtLeast(min_ready) => ready >= min_ready,
    }
}

fn ready_count(statuses: &[HttpProbeStatus]) -> usize {
    statuses.iter().filter(|status| status.ok).count()
}

fn format_http_timeout_message(
    statuses: &[HttpProbeStatus],
    requirement: HttpReadinessRequirement,
) -> String {
    let summary = timeout_summary(statuses);
    let required = required_ready_nodes(requirement, summary.total);

    format!(
        "timed out waiting for readiness {:?}; ready={}, required={}, total={}, failing endpoints: {}",
        requirement, summary.ready, required, summary.total, summary.failed_list
    )
}

struct TimeoutSummary {
    ready: usize,
    total: usize,
    failed_list: String,
}

fn timeout_summary(statuses: &[HttpProbeStatus]) -> TimeoutSummary {
    let total = statuses.len();

    TimeoutSummary {
        ready: ready_count(statuses),
        total,
        failed_list: format_failed_endpoints(statuses),
    }
}

fn failed_endpoints(statuses: &[HttpProbeStatus]) -> Vec<String> {
    statuses
        .iter()
        .filter(|status| !status.ok)
        .map(|status| format!("{} ({})", status.endpoint, status.detail))
        .collect()
}

fn format_failed_endpoints(statuses: &[HttpProbeStatus]) -> String {
    let failed = failed_endpoints(statuses);
    if failed.is_empty() {
        return NO_FAILING_ENDPOINTS.to_string();
    }

    failed.join(", ")
}

fn required_ready_nodes(requirement: HttpReadinessRequirement, total: usize) -> usize {
    match requirement {
        HttpReadinessRequirement::AllNodesReady => total,
        HttpReadinessRequirement::AnyNodeReady => usize::from(total > 0),
        HttpReadinessRequirement::AtLeast(min_ready) => min_ready,
    }
}

fn stabilization_details(failures: &[String]) -> String {
    if failures.is_empty() {
        NO_STABILIZATION_DETAILS.to_string()
    } else {
        failures.join(", ")
    }
}

async fn collect_http_statuses(client: &Client, endpoints: &[Url]) -> Vec<HttpProbeStatus> {
    let futures = endpoints.iter().map(|endpoint| async move {
        match client.get(endpoint.clone()).send().await {
            Ok(response) => {
                let status = response.status();
                HttpProbeStatus {
                    endpoint: endpoint.clone(),
                    ok: status.is_success(),
                    detail: format!("status {}", status.as_u16()),
                }
            }

            Err(err) => HttpProbeStatus {
                endpoint: endpoint.clone(),
                ok: false,
                detail: err.to_string(),
            },
        }
    });
    futures::future::join_all(futures).await
}

pub async fn wait_until_stable<F, Fut>(
    config: StabilizationConfig,
    mut probe: F,
) -> Result<(), ReadinessError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Vec<String>, DynError>>,
{
    let timeout = adjust_timeout(config.timeout);
    let poll_interval = config.poll_interval;
    let deadline = Instant::now() + timeout;

    loop {
        let failures = probe()
            .await
            .map_err(|source| ReadinessError::StabilizationProbe { source })?;
        if failures.is_empty() {
            return Ok(());
        }

        if Instant::now() >= deadline {
            let details = stabilization_details(&failures);

            return Err(ReadinessError::StabilizationTimeout { timeout, details });
        }

        tokio::time::sleep(poll_interval).await;
    }
}

pub async fn wait_http_readiness(
    endpoints: &[Url],
    requirement: HttpReadinessRequirement,
) -> Result<(), ReadinessError> {
    wait_http_readiness_with_timeout(endpoints, requirement, None).await
}

pub async fn wait_http_readiness_with_timeout(
    endpoints: &[Url],
    requirement: HttpReadinessRequirement,
    timeout: Option<Duration>,
) -> Result<(), ReadinessError> {
    wait_http_readiness_with_config(
        endpoints,
        requirement,
        timeout.unwrap_or(DEFAULT_TIMEOUT),
        DEFAULT_POLL_INTERVAL,
    )
    .await
}

async fn wait_http_readiness_with_config(
    endpoints: &[Url],
    requirement: HttpReadinessRequirement,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<(), ReadinessError> {
    if endpoints.is_empty() {
        return Ok(());
    }

    let timeout = adjust_timeout(timeout);
    let poll_interval = poll_interval.max(Duration::from_millis(1));
    let deadline = Instant::now() + timeout;
    let client = Client::new();
    let mut last_statuses = None;

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(http_probe_timeout(
                last_statuses.as_deref(),
                requirement,
                timeout,
            ));
        }

        let statuses = tokio::time::timeout(remaining, collect_http_statuses(&client, endpoints))
            .await
            .map_err(|_| http_probe_timeout(last_statuses.as_deref(), requirement, timeout))?;
        if requirement_satisfied(&statuses, requirement) {
            return Ok(());
        }

        last_statuses = Some(statuses);
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(http_probe_timeout(
                last_statuses.as_deref(),
                requirement,
                timeout,
            ));
        }
        tokio::time::sleep(poll_interval.min(remaining)).await;
    }
}

fn http_probe_timeout(
    statuses: Option<&[HttpProbeStatus]>,
    requirement: HttpReadinessRequirement,
    timeout: Duration,
) -> ReadinessError {
    let message = statuses.map_or_else(
        || format!("timed out after {timeout:?} waiting for readiness {requirement:?}"),
        |statuses| format_http_timeout_message(statuses, requirement),
    );
    ReadinessError::ProbeTimeout { message }
}

pub async fn wait_for_http_ports(ports: &[u16], endpoint_path: &str) -> Result<(), ReadinessError> {
    wait_for_http_ports_with_timeout(ports, endpoint_path, None).await
}

pub async fn wait_for_http_ports_with_timeout(
    ports: &[u16],
    endpoint_path: &str,
    timeout: Option<Duration>,
) -> Result<(), ReadinessError> {
    wait_for_http_ports_with_requirement_and_timeout(
        ports,
        endpoint_path,
        default_readiness_requirement(),
        timeout,
    )
    .await
}

pub async fn wait_for_http_ports_with_requirement(
    ports: &[u16],
    endpoint_path: &str,
    requirement: HttpReadinessRequirement,
) -> Result<(), ReadinessError> {
    wait_for_http_ports_with_requirement_and_timeout(ports, endpoint_path, requirement, None).await
}

pub async fn wait_for_http_ports_with_requirement_and_timeout(
    ports: &[u16],
    endpoint_path: &str,
    requirement: HttpReadinessRequirement,
    timeout: Option<Duration>,
) -> Result<(), ReadinessError> {
    let endpoints = build_local_endpoints(ports, endpoint_path)?;
    wait_http_readiness_with_timeout(&endpoints, requirement, timeout).await
}

pub async fn wait_for_http_ports_with_host(
    ports: &[u16],
    host: &str,
    endpoint_path: &str,
) -> Result<(), ReadinessError> {
    wait_for_http_ports_with_host_and_requirement(
        ports,
        host,
        endpoint_path,
        default_readiness_requirement(),
    )
    .await
}

pub async fn wait_for_http_ports_with_host_and_requirement(
    ports: &[u16],
    host: &str,
    endpoint_path: &str,
    requirement: HttpReadinessRequirement,
) -> Result<(), ReadinessError> {
    let endpoints = build_endpoints_with_host(ports, host, endpoint_path)?;
    wait_http_readiness(&endpoints, requirement).await
}

pub async fn wait_for_http_ports_with_host_and_config(
    ports: &[u16],
    host: &str,
    endpoint_path: &str,
    requirement: HttpReadinessRequirement,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<(), ReadinessError> {
    let endpoints = build_endpoints_with_host(ports, host, endpoint_path)?;
    wait_http_readiness_with_config(&endpoints, requirement, timeout, poll_interval).await
}

const fn default_readiness_requirement() -> HttpReadinessRequirement {
    HttpReadinessRequirement::AllNodesReady
}

#[cfg(test)]
mod tests {
    use std::{
        io::Read,
        net::TcpListener,
        thread,
        time::{Duration, Instant},
    };

    use super::*;

    #[tokio::test]
    async fn readiness_timeout_is_a_hard_deadline_for_stalled_requests() {
        let listener = TcpListener::bind((LOCALHOST, 0)).expect("bind stalled HTTP server");
        let port = listener.local_addr().expect("server address").port();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 256];
            let _ = stream.read(&mut request);
            thread::sleep(Duration::from_secs(1));
        });

        let started = Instant::now();
        let result = wait_for_http_ports_with_host_and_config(
            &[port],
            LOCALHOST,
            "/ready",
            HttpReadinessRequirement::AllNodesReady,
            Duration::from_millis(50),
            Duration::from_millis(5),
        )
        .await;

        assert!(matches!(result, Err(ReadinessError::ProbeTimeout { .. })));
        assert!(started.elapsed() < Duration::from_millis(500));
    }
}

use std::{
    future::Future,
    time::{Duration, Instant},
};

use reqwest::{Client, Url};
use thiserror::Error;

use crate::{
    adjust_timeout,
    retry::{RetryConfig, retry_async},
    scenario::DynError,
};

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
    if endpoints.is_empty() {
        return Ok(());
    }

    let (poll_interval, max_attempts) = http_retry_plan();
    let client = Client::new();
    let retry = RetryConfig::bounded(max_attempts, poll_interval, poll_interval);
    retry_async(retry, |_| async {
        let statuses = collect_http_statuses(&client, endpoints).await;
        if requirement_satisfied(&statuses, requirement) {
            Ok(())
        } else {
            Err(statuses)
        }
    })
    .await
    .map_err(|statuses| ReadinessError::ProbeTimeout {
        message: format_http_timeout_message(&statuses, requirement),
    })
}

fn http_retry_plan() -> (Duration, usize) {
    let timeout_duration = adjust_timeout(DEFAULT_TIMEOUT);
    let poll_interval = DEFAULT_POLL_INTERVAL;
    let max_attempts = retry_attempts(timeout_duration, poll_interval);
    (poll_interval, max_attempts)
}

fn retry_attempts(timeout: Duration, interval: Duration) -> usize {
    let timeout_ms = timeout.as_millis();
    let interval_ms = interval.as_millis();

    timeout_ms.div_ceil(interval_ms).max(1).saturating_add(1) as usize
}

pub async fn wait_for_http_ports(ports: &[u16], endpoint_path: &str) -> Result<(), ReadinessError> {
    wait_for_http_ports_with_requirement(ports, endpoint_path, default_readiness_requirement())
        .await
}

pub async fn wait_for_http_ports_with_requirement(
    ports: &[u16],
    endpoint_path: &str,
    requirement: HttpReadinessRequirement,
) -> Result<(), ReadinessError> {
    let endpoints = build_local_endpoints(ports, endpoint_path)?;
    wait_http_readiness(&endpoints, requirement).await
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

const fn default_readiness_requirement() -> HttpReadinessRequirement {
    HttpReadinessRequirement::AllNodesReady
}

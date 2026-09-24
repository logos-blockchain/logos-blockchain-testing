use std::{
    future::Future,
    time::{Duration, Instant},
};

use reqwest::{Client, Url};
use thiserror::Error;
use tokio::{
    net::TcpStream,
    time::{sleep, timeout},
};

use crate::{adjust_timeout, scenario::DynError};

pub const DEFAULT_READINESS_POLL_INTERVAL: Duration = Duration::from_millis(200);
pub const DEFAULT_READINESS_TIMEOUT: Duration = Duration::from_secs(60);
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
pub enum ReadinessRequirement {
    AllNodesReady,
    AnyNodeReady,
    AtLeast(usize),
}

/// Checks the reachable node endpoint supplied by the backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadinessProbe {
    Http { path: &'static str },
    Tcp,
}

#[derive(Debug)]
struct ProbeStatus {
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

fn requirement_satisfied(statuses: &[ProbeStatus], requirement: ReadinessRequirement) -> bool {
    let ready = ready_count(statuses);
    match requirement {
        ReadinessRequirement::AllNodesReady => ready == statuses.len(),
        ReadinessRequirement::AnyNodeReady => ready >= 1,
        ReadinessRequirement::AtLeast(min_ready) => ready >= min_ready,
    }
}

fn ready_count(statuses: &[ProbeStatus]) -> usize {
    statuses.iter().filter(|status| status.ok).count()
}

fn format_timeout_message(statuses: &[ProbeStatus], requirement: ReadinessRequirement) -> String {
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

fn timeout_summary(statuses: &[ProbeStatus]) -> TimeoutSummary {
    let total = statuses.len();

    TimeoutSummary {
        ready: ready_count(statuses),
        total,
        failed_list: format_failed_endpoints(statuses),
    }
}

fn failed_endpoints(statuses: &[ProbeStatus]) -> Vec<String> {
    statuses
        .iter()
        .filter(|status| !status.ok)
        .map(|status| format!("{} ({})", status.endpoint, status.detail))
        .collect()
}

fn format_failed_endpoints(statuses: &[ProbeStatus]) -> String {
    let failed = failed_endpoints(statuses);
    if failed.is_empty() {
        return NO_FAILING_ENDPOINTS.to_string();
    }

    failed.join(", ")
}

fn required_ready_nodes(requirement: ReadinessRequirement, total: usize) -> usize {
    match requirement {
        ReadinessRequirement::AllNodesReady => total,
        ReadinessRequirement::AnyNodeReady => usize::from(total > 0),
        ReadinessRequirement::AtLeast(min_ready) => min_ready,
    }
}

fn stabilization_details(failures: &[String]) -> String {
    if failures.is_empty() {
        NO_STABILIZATION_DETAILS.to_string()
    } else {
        failures.join(", ")
    }
}

async fn collect_http_statuses(client: &Client, endpoints: &[Url]) -> Vec<ProbeStatus> {
    let futures = endpoints.iter().map(|endpoint| async move {
        match client.get(endpoint.clone()).send().await {
            Ok(response) => {
                let status = response.status();
                ProbeStatus {
                    endpoint: endpoint.clone(),
                    ok: status.is_success(),
                    detail: format!("status {}", status.as_u16()),
                }
            }

            Err(err) => ProbeStatus {
                endpoint: endpoint.clone(),
                ok: false,
                detail: err.to_string(),
            },
        }
    });
    futures::future::join_all(futures).await
}

async fn collect_tcp_statuses(endpoints: &[Url]) -> Vec<ProbeStatus> {
    let probes = endpoints.iter().map(|endpoint| async move {
        let result = async {
            let host = endpoint.host_str().ok_or("missing host")?;
            let port = endpoint.port_or_known_default().ok_or("missing port")?;
            TcpStream::connect((host, port))
                .await
                .map_err(|error| error.to_string())?;
            Ok::<_, String>(())
        };

        let (ok, detail) = match timeout(Duration::from_millis(100), result).await {
            Ok(Ok(())) => (true, "TCP connected".to_owned()),
            Ok(Err(error)) => (false, format!("TCP: {error}")),
            Err(_) => (false, "TCP connection timed out".to_owned()),
        };

        ProbeStatus {
            endpoint: endpoint.clone(),
            ok,
            detail,
        }
    });

    futures::future::join_all(probes).await
}

/// Waits for the selected probe using the backend's reachable node endpoints.
pub async fn wait_readiness(
    endpoints: &[Url],
    probe: ReadinessProbe,
    requirement: ReadinessRequirement,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<(), ReadinessError> {
    match probe {
        ReadinessProbe::Http { path } => {
            let path = normalize_endpoint_path(path);
            let endpoints = endpoints
                .iter()
                .map(|url| {
                    let mut url = url.clone();
                    url.set_path(&path);
                    url
                })
                .collect::<Vec<_>>();

            probe_http_endpoints(&endpoints, requirement, timeout, poll_interval).await
        }
        ReadinessProbe::Tcp => {
            poll_readiness(endpoints, requirement, timeout, poll_interval, || {
                collect_tcp_statuses(endpoints)
            })
            .await
        }
    }
}

pub async fn wait_for_readiness_ports(
    ports: &[u16],
    host: &str,
    probe: ReadinessProbe,
    requirement: ReadinessRequirement,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<(), ReadinessError> {
    let endpoints = build_endpoints_with_host(ports, host, "/")?;
    wait_readiness(&endpoints, probe, requirement, timeout, poll_interval).await
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

        sleep(poll_interval).await;
    }
}

pub async fn wait_http_readiness(
    endpoints: &[Url],
    requirement: ReadinessRequirement,
) -> Result<(), ReadinessError> {
    wait_http_readiness_with_timeout(endpoints, requirement, None).await
}

pub async fn wait_http_readiness_with_timeout(
    endpoints: &[Url],
    requirement: ReadinessRequirement,
    timeout: Option<Duration>,
) -> Result<(), ReadinessError> {
    probe_http_endpoints(
        endpoints,
        requirement,
        timeout.unwrap_or(DEFAULT_READINESS_TIMEOUT),
        DEFAULT_READINESS_POLL_INTERVAL,
    )
    .await
}

async fn probe_http_endpoints(
    endpoints: &[Url],
    requirement: ReadinessRequirement,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<(), ReadinessError> {
    let client = Client::new();
    poll_readiness(endpoints, requirement, timeout, poll_interval, || {
        collect_http_statuses(&client, endpoints)
    })
    .await
}

async fn poll_readiness<F, Fut>(
    endpoints: &[Url],
    requirement: ReadinessRequirement,
    timeout_duration: Duration,
    poll_interval: Duration,
    mut probe: F,
) -> Result<(), ReadinessError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Vec<ProbeStatus>>,
{
    if endpoints.is_empty() {
        return Ok(());
    }

    let timeout_duration = adjust_timeout(timeout_duration);
    let poll_interval = poll_interval.max(Duration::from_millis(1));
    let deadline = Instant::now() + timeout_duration;
    let mut last_statuses = None;

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(probe_timeout(
                last_statuses.as_deref(),
                requirement,
                timeout_duration,
            ));
        }

        let statuses = timeout(remaining, probe())
            .await
            .map_err(|_| probe_timeout(last_statuses.as_deref(), requirement, timeout_duration))?;
        if requirement_satisfied(&statuses, requirement) {
            return Ok(());
        }

        last_statuses = Some(statuses);
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(probe_timeout(
                last_statuses.as_deref(),
                requirement,
                timeout_duration,
            ));
        }
        sleep(poll_interval.min(remaining)).await;
    }
}

fn probe_timeout(
    statuses: Option<&[ProbeStatus]>,
    requirement: ReadinessRequirement,
    timeout: Duration,
) -> ReadinessError {
    let message = statuses.map_or_else(
        || format!("timed out after {timeout:?} waiting for readiness {requirement:?}"),
        |statuses| format_timeout_message(statuses, requirement),
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
    requirement: ReadinessRequirement,
) -> Result<(), ReadinessError> {
    wait_for_http_ports_with_requirement_and_timeout(ports, endpoint_path, requirement, None).await
}

pub async fn wait_for_http_ports_with_requirement_and_timeout(
    ports: &[u16],
    endpoint_path: &str,
    requirement: ReadinessRequirement,
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
    requirement: ReadinessRequirement,
) -> Result<(), ReadinessError> {
    let endpoints = build_endpoints_with_host(ports, host, endpoint_path)?;
    wait_http_readiness(&endpoints, requirement).await
}

pub async fn wait_for_http_ports_with_host_and_config(
    ports: &[u16],
    host: &str,
    endpoint_path: &str,
    requirement: ReadinessRequirement,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<(), ReadinessError> {
    let endpoints = build_endpoints_with_host(ports, host, endpoint_path)?;
    probe_http_endpoints(&endpoints, requirement, timeout, poll_interval).await
}

const fn default_readiness_requirement() -> ReadinessRequirement {
    ReadinessRequirement::AllNodesReady
}

#[cfg(test)]
mod tests {
    use std::{
        io::Read,
        net::TcpListener,
        thread,
        time::{Duration, Instant},
    };

    use tokio::net::TcpSocket;

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
            ReadinessRequirement::AllNodesReady,
            Duration::from_millis(50),
            Duration::from_millis(5),
        )
        .await;

        assert!(matches!(result, Err(ReadinessError::ProbeTimeout { .. })));
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[tokio::test]
    async fn tcp_readiness_does_not_require_an_http_response() {
        let listener = TcpListener::bind((LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();

        wait_for_readiness_ports(
            &[port],
            LOCALHOST,
            ReadinessProbe::Tcp,
            ReadinessRequirement::AllNodesReady,
            Duration::from_millis(200),
            Duration::from_millis(5),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tcp_readiness_respects_node_counts_and_reports_failures() {
        let listener = TcpListener::bind((LOCALHOST, 0)).unwrap();
        let ready_port = listener.local_addr().unwrap().port();
        // Reserve a port without listening so another test cannot take it.
        let closed = TcpSocket::new_v4().unwrap();
        closed
            .bind(format!("{LOCALHOST}:0").parse().unwrap())
            .unwrap();
        let closed_port = closed.local_addr().unwrap().port();
        let ports = [ready_port, closed_port];

        for requirement in [
            ReadinessRequirement::AnyNodeReady,
            ReadinessRequirement::AtLeast(1),
        ] {
            wait_for_readiness_ports(
                &ports,
                LOCALHOST,
                ReadinessProbe::Tcp,
                requirement,
                Duration::from_millis(200),
                Duration::from_millis(5),
            )
            .await
            .unwrap();
        }

        for requirement in [
            ReadinessRequirement::AllNodesReady,
            ReadinessRequirement::AtLeast(2),
        ] {
            let error = wait_for_readiness_ports(
                &ports,
                LOCALHOST,
                ReadinessProbe::Tcp,
                requirement,
                Duration::from_millis(300),
                Duration::from_millis(5),
            )
            .await
            .unwrap_err();

            let message = error.to_string();
            assert!(message.contains("ready=1"), "{message}");
            assert!(message.contains(&closed_port.to_string()), "{message}");
        }
    }

    #[tokio::test]
    async fn shared_http_probe_uses_the_selected_path() {
        use std::io::Write as _;

        let listener = TcpListener::bind((LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0_u8; 1024];
            let length = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..length]);
            assert!(request.starts_with("GET /custom-ready "), "{request}");
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        });

        wait_for_readiness_ports(
            &[port],
            LOCALHOST,
            ReadinessProbe::Http {
                path: "custom-ready",
            },
            ReadinessRequirement::AllNodesReady,
            Duration::from_secs(2),
            Duration::from_millis(5),
        )
        .await
        .unwrap();

        server.join().unwrap();
    }
}

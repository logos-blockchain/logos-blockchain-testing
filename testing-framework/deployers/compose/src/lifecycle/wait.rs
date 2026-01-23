use std::{env, time::Duration};

use testing_framework_core::{
    adjust_timeout,
    scenario::http_probe::{self, HttpReadinessError, NodeRole},
};
use tracing::{debug, info};

const DEFAULT_WAIT_TIMEOUT_SECS: u64 = 180;
const POLL_INTERVAL_MILLIS: u64 = 250;

const DEFAULT_WAIT: Duration = Duration::from_secs(DEFAULT_WAIT_TIMEOUT_SECS);
const POLL_INTERVAL: Duration = Duration::from_millis(POLL_INTERVAL_MILLIS);

pub async fn wait_for_validators(ports: &[u16]) -> Result<(), HttpReadinessError> {
    wait_for_ports(ports, NodeRole::Validator).await
}

async fn wait_for_ports(ports: &[u16], role: NodeRole) -> Result<(), HttpReadinessError> {
    let host = compose_runner_host();
    let timeout = compose_http_timeout();

    info!(role = ?role, ports = ?ports, host, "waiting for compose HTTP readiness");

    http_probe::wait_for_http_ports_with_host(
        ports,
        role,
        &host,
        adjust_timeout(timeout),
        POLL_INTERVAL,
    )
    .await
}

const DEFAULT_COMPOSE_HOST: &str = "127.0.0.1";

fn compose_runner_host() -> String {
    let host = env::var("COMPOSE_RUNNER_HOST").unwrap_or_else(|_| DEFAULT_COMPOSE_HOST.to_string());
    debug!(host, "compose runner host resolved");
    host
}

fn compose_http_timeout() -> Duration {
    env::var("COMPOSE_RUNNER_HTTP_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_WAIT)
}

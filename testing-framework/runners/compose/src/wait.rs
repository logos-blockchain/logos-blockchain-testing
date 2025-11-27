use std::{env, time::Duration};

use testing_framework_core::{
    adjust_timeout,
    scenario::http_probe::{self, HttpReadinessError, NodeRole},
};

const DEFAULT_WAIT: Duration = Duration::from_secs(180);
const POLL_INTERVAL: Duration = Duration::from_millis(250);

pub async fn wait_for_validators(ports: &[u16]) -> Result<(), HttpReadinessError> {
    wait_for_ports(ports, NodeRole::Validator).await
}

pub async fn wait_for_executors(ports: &[u16]) -> Result<(), HttpReadinessError> {
    wait_for_ports(ports, NodeRole::Executor).await
}

async fn wait_for_ports(ports: &[u16], role: NodeRole) -> Result<(), HttpReadinessError> {
    let host = compose_runner_host();
    http_probe::wait_for_http_ports_with_host(
        ports,
        role,
        &host,
        adjust_timeout(DEFAULT_WAIT),
        POLL_INTERVAL,
    )
    .await
}

fn compose_runner_host() -> String {
    env::var("COMPOSE_RUNNER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string())
}

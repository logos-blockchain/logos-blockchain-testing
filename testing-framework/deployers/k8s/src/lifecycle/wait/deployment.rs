use std::time::Duration;

use k8s_openapi::api::apps::v1::Deployment;
use kube::{Api, Client};
use tokio_retry::{RetryIf, strategy::FixedInterval};

use super::{ClusterWaitError, deployment_timeout};

const DEPLOYMENT_POLL_INTERVAL: Duration = Duration::from_secs(2);
const MILLIS_PER_SECOND: u64 = 1_000;

#[derive(Debug)]
enum DeploymentWaitError {
    NotReady,
    Fatal(ClusterWaitError),
}

pub async fn wait_for_deployment_ready(
    client: &Client,
    namespace: &str,
    name: &str,
) -> Result<(), ClusterWaitError> {
    let timeout = deployment_timeout();
    let strategy = deployment_retry_strategy(timeout);
    let deployments = Api::<Deployment>::namespaced(client.clone(), namespace);
    let result = RetryIf::spawn(
        strategy,
        || check_deployment_ready(&deployments, name),
        |error: &DeploymentWaitError| matches!(error, DeploymentWaitError::NotReady),
    )
    .await;

    map_deployment_wait_result(result, name, namespace, timeout)
}

fn deployment_retry_strategy(timeout: Duration) -> impl Iterator<Item = Duration> {
    let max_attempts = max_attempts_for_timeout(timeout);
    FixedInterval::from_millis(retry_interval_millis()).take(max_attempts)
}

const fn retry_interval_millis() -> u64 {
    DEPLOYMENT_POLL_INTERVAL.as_secs() * MILLIS_PER_SECOND
}

fn max_attempts_for_timeout(timeout: Duration) -> usize {
    let timeout_ms = timeout.as_millis();
    let interval_ms = DEPLOYMENT_POLL_INTERVAL.as_millis();

    timeout_ms.div_ceil(interval_ms).max(1) as usize
}

async fn check_deployment_ready(
    deployments: &Api<Deployment>,
    name: &str,
) -> Result<(), DeploymentWaitError> {
    match deployments.get(name).await {
        Ok(deployment) => ensure_ready_replicas(deployment),
        Err(source) => Err(DeploymentWaitError::Fatal(
            ClusterWaitError::DeploymentFetch {
                name: name.to_owned(),
                source,
            },
        )),
    }
}

fn ensure_ready_replicas(deployment: Deployment) -> Result<(), DeploymentWaitError> {
    let desired = deployment
        .spec
        .as_ref()
        .and_then(|spec| spec.replicas)
        .unwrap_or(1);
    let ready = deployment
        .status
        .as_ref()
        .and_then(|status| status.ready_replicas)
        .unwrap_or(0);

    if ready >= desired {
        return Ok(());
    }

    Err(DeploymentWaitError::NotReady)
}

fn map_deployment_wait_result(
    result: Result<(), DeploymentWaitError>,
    name: &str,
    namespace: &str,
    timeout: Duration,
) -> Result<(), ClusterWaitError> {
    match result {
        Ok(()) => Ok(()),
        Err(DeploymentWaitError::Fatal(error)) => Err(error),
        Err(DeploymentWaitError::NotReady) => Err(ClusterWaitError::DeploymentTimeout {
            name: name.to_owned(),
            namespace: namespace.to_owned(),
            timeout,
        }),
    }
}

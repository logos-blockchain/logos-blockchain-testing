use std::time::Duration;

use testing_framework_core::scenario::{HttpReadinessRequirement, NodeClients};
use tokio::time::sleep;

use crate::{
    env::ComposeDeployEnv,
    errors::{NodeClientError, StackReadinessError},
    infrastructure::ports::{HostPortMapping, compose_runner_host},
};

const DISABLED_READINESS_SLEEP: Duration = Duration::from_secs(5);

/// Wait until all nodes respond on their API ports.
pub async fn ensure_nodes_ready_with_ports<E: ComposeDeployEnv>(
    ports: &[u16],
    requirement: HttpReadinessRequirement,
) -> Result<(), StackReadinessError> {
    if ports.is_empty() {
        return Ok(());
    }

    let host = compose_runner_host();
    E::wait_for_nodes(ports, &host, requirement)
        .await
        .map_err(|source| StackReadinessError::Remote { source })
}

/// Allow a brief pause when readiness probes are disabled.
pub async fn maybe_sleep_for_disabled_readiness(readiness_enabled: bool) {
    if readiness_enabled {
        return;
    }

    sleep(DISABLED_READINESS_SLEEP).await;
}

/// Construct API clients using the mapped host ports.
pub fn build_node_clients_with_ports<E: ComposeDeployEnv>(
    descriptors: &E::Deployment,
    mapping: &HostPortMapping,
    host: &str,
) -> Result<NodeClients<E>, NodeClientError> {
    E::build_node_clients(descriptors, mapping, host)
        .map_err(|source| NodeClientError::Build { source })
}

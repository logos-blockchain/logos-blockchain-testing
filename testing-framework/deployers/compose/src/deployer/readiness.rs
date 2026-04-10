use std::marker::PhantomData;

use testing_framework_core::scenario::HttpReadinessRequirement;
use tracing::{info, warn};

use crate::{
    env::{ComposeDeployEnv, wait_remote_readiness as remote_readiness_future},
    errors::ComposeRunnerError,
    infrastructure::{environment::StackEnvironment, ports::HostPortMapping},
    lifecycle::readiness::ensure_nodes_ready_with_ports,
};

pub struct ReadinessChecker<E: ComposeDeployEnv> {
    _env: PhantomData<E>,
}

impl<E: ComposeDeployEnv> ReadinessChecker<E> {
    pub async fn wait_all(
        descriptors: &E::Deployment,
        host_ports: &HostPortMapping,
        requirement: HttpReadinessRequirement,
        environment: &mut StackEnvironment,
    ) -> Result<(), ComposeRunnerError> {
        let node_ports = host_ports.node_api_ports();
        info!(ports = ?node_ports, "waiting for node HTTP endpoints");

        wait_local_readiness::<E>(environment, &node_ports, requirement).await?;

        info!("waiting for remote service readiness");
        wait_remote_readiness::<E>(environment, descriptors, host_ports, requirement).await?;

        info!("compose readiness checks passed");
        Ok(())
    }
}

async fn wait_local_readiness<E: ComposeDeployEnv>(
    environment: &mut StackEnvironment,
    node_ports: &[u16],
    requirement: HttpReadinessRequirement,
) -> Result<(), ComposeRunnerError> {
    let result = ensure_nodes_ready_with_ports::<E>(node_ports, requirement)
        .await
        .map_err(ComposeRunnerError::from);

    run_readiness_check(environment, "node readiness failed", result).await
}

async fn wait_remote_readiness<E: ComposeDeployEnv>(
    environment: &mut StackEnvironment,
    descriptors: &E::Deployment,
    host_ports: &HostPortMapping,
    requirement: HttpReadinessRequirement,
) -> Result<(), ComposeRunnerError> {
    run_readiness_check(
        environment,
        "remote readiness probe failed",
        remote_readiness_future::<E>(descriptors, host_ports, requirement)
            .map_err(|source| {
                ComposeRunnerError::Readiness(crate::errors::StackReadinessError::Remote { source })
            })?
            .await
            .map_err(|source| {
                ComposeRunnerError::Readiness(crate::errors::StackReadinessError::Remote { source })
            }),
    )
    .await
}

async fn run_readiness_check(
    environment: &mut StackEnvironment,
    fail_reason: &str,
    result: Result<(), ComposeRunnerError>,
) -> Result<(), ComposeRunnerError> {
    if let Err(error) = result {
        environment.fail(fail_reason).await;
        warn!(error = ?error, "{fail_reason}");
        return Err(error);
    }

    Ok(())
}

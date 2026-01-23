use testing_framework_core::topology::generation::GeneratedTopology;
use tracing::info;

use crate::{
    errors::ComposeRunnerError,
    infrastructure::{
        environment::StackEnvironment,
        ports::{HostPortMapping, ensure_remote_readiness_with_ports},
    },
    lifecycle::readiness::ensure_validators_ready_with_ports,
};

pub struct ReadinessChecker;

impl ReadinessChecker {
    pub async fn wait_all(
        descriptors: &GeneratedTopology,
        host_ports: &HostPortMapping,
        environment: &mut StackEnvironment,
    ) -> Result<(), ComposeRunnerError> {
        let validator_ports = host_ports.validator_api_ports();
        info!(ports = ?validator_ports, "waiting for validator HTTP endpoints");
        if let Err(err) = ensure_validators_ready_with_ports(&validator_ports).await {
            return fail_readiness_step(
                environment,
                "validator readiness failed",
                "validator readiness failed",
                err,
            )
            .await;
        }

        info!("waiting for remote service readiness");
        if let Err(err) = ensure_remote_readiness_with_ports(descriptors, host_ports).await {
            return fail_readiness_step(
                environment,
                "remote readiness probe failed",
                "remote readiness probe failed",
                err,
            )
            .await;
        }

        info!("compose readiness checks passed");
        Ok(())
    }
}

async fn fail_readiness_step<E>(
    environment: &mut StackEnvironment,
    reason: &str,
    log_message: &str,
    error: E,
) -> Result<(), ComposeRunnerError>
where
    E: std::fmt::Debug + Into<ComposeRunnerError>,
{
    environment.fail(reason).await;
    tracing::warn!(error = ?error, "{log_message}");
    Err(error.into())
}

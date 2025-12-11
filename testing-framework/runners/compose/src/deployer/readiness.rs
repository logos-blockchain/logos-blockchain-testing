use testing_framework_core::topology::generation::GeneratedTopology;
use tracing::info;

use crate::{
    errors::ComposeRunnerError,
    infrastructure::{
        environment::StackEnvironment,
        ports::{HostPortMapping, ensure_remote_readiness_with_ports},
    },
    lifecycle::readiness::{ensure_executors_ready_with_ports, ensure_validators_ready_with_ports},
};

pub struct ReadinessChecker;

impl ReadinessChecker {
    pub async fn wait_all(
        descriptors: &GeneratedTopology,
        host_ports: &HostPortMapping,
        environment: &mut StackEnvironment,
    ) -> Result<(), ComposeRunnerError> {
        info!(
            ports = ?host_ports.validator_api_ports(),
            "waiting for validator HTTP endpoints"
        );
        if let Err(err) =
            ensure_validators_ready_with_ports(&host_ports.validator_api_ports()).await
        {
            environment.fail("validator readiness failed").await;
            return Err(err.into());
        }

        info!(
            ports = ?host_ports.executor_api_ports(),
            "waiting for executor HTTP endpoints"
        );
        if let Err(err) = ensure_executors_ready_with_ports(&host_ports.executor_api_ports()).await
        {
            environment.fail("executor readiness failed").await;
            return Err(err.into());
        }

        info!("waiting for remote service readiness");
        if let Err(err) = ensure_remote_readiness_with_ports(descriptors, host_ports).await {
            environment.fail("remote readiness probe failed").await;
            return Err(err.into());
        }

        info!("compose readiness checks passed");
        Ok(())
    }
}

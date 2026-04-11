use std::{fmt::Debug, marker::PhantomData};

use testing_framework_core::scenario::NodeClients;
use tracing::warn;

use crate::{
    env::ComposeDeployEnv,
    errors::ComposeRunnerError,
    infrastructure::{environment::StackEnvironment, ports::HostPortMapping},
    lifecycle::readiness::build_node_clients_with_ports,
};

pub struct ClientBuilder<E: ComposeDeployEnv> {
    _env: PhantomData<E>,
}

impl<E: ComposeDeployEnv> ClientBuilder<E> {
    #[must_use]
    pub const fn new() -> Self {
        Self { _env: PhantomData }
    }

    pub async fn build_node_clients(
        &self,
        descriptors: &E::Deployment,
        host_ports: &HostPortMapping,
        host: &str,
        environment: &mut StackEnvironment,
    ) -> Result<NodeClients<E>, ComposeRunnerError> {
        ensure_step(
            environment,
            build_node_clients_with_ports::<E>(descriptors, host_ports, host),
            "failed to construct node api clients",
            "failed to build node clients",
        )
        .await
    }
}

async fn ensure_step<T, E>(
    environment: &mut StackEnvironment,
    result: Result<T, E>,
    fail_reason: &str,
    log_message: &str,
) -> Result<T, ComposeRunnerError>
where
    E: Debug + Into<ComposeRunnerError>,
{
    let value = match result {
        Ok(value) => value,
        Err(error) => {
            environment.fail(fail_reason).await;
            warn!(error = ?error, "{log_message}");
            return Err(error.into());
        }
    };

    Ok(value)
}

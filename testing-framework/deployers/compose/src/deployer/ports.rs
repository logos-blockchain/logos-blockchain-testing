use std::marker::PhantomData;

use tracing::{debug, info, warn};

use crate::{
    env::{ComposeDeployEnv, node_container_ports},
    errors::ComposeRunnerError,
    infrastructure::{
        environment::StackEnvironment,
        ports::{HostPortMapping, discover_host_ports},
    },
};

pub struct PortManager<E: ComposeDeployEnv> {
    _env: PhantomData<E>,
}

impl<E: ComposeDeployEnv> PortManager<E> {
    pub async fn prepare(
        environment: &mut StackEnvironment,
        descriptors: &E::Deployment,
    ) -> Result<HostPortMapping, ComposeRunnerError> {
        let nodes = node_container_ports::<E>(descriptors).map_err(|source| {
            ComposeRunnerError::Config(crate::errors::ConfigError::Descriptor { source })
        })?;
        debug!(
            nodes = nodes.len(),
            "resolving host ports for compose services"
        );
        let mapping = match discover_host_ports(environment, &nodes).await {
            Ok(mapping) => mapping,
            Err(error) => return Err(fail_host_port_resolution(environment, error).await),
        };

        info!(
            node_ports = ?mapping.node_api_ports(),
            "resolved container host ports"
        );

        Ok(mapping)
    }
}

async fn fail_host_port_resolution(
    environment: &mut StackEnvironment,
    error: ComposeRunnerError,
) -> ComposeRunnerError {
    environment
        .fail("failed to determine container host ports")
        .await;
    warn!(%error, "failed to resolve host ports");
    error
}

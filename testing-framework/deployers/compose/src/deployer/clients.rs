use std::{fmt::Debug, marker::PhantomData};

use testing_framework_core::scenario::{
    Application, FeedRuntime, NodeClients, internal::FeedHandle,
};
use tracing::{info, warn};

use crate::{
    env::ComposeDeployEnv,
    errors::ComposeRunnerError,
    infrastructure::{environment::StackEnvironment, ports::HostPortMapping},
    lifecycle::{
        block_feed::spawn_block_feed_with_retry, readiness::build_node_clients_with_ports,
    },
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

    pub async fn start_block_feed(
        &self,
        node_clients: &NodeClients<E>,
        environment: &mut StackEnvironment,
    ) -> Result<
        (
            <<E as Application>::FeedRuntime as FeedRuntime>::Feed,
            FeedHandle,
        ),
        ComposeRunnerError,
    > {
        let pair = ensure_step(
            environment,
            spawn_block_feed_with_retry::<E>(node_clients).await,
            "failed to initialize block feed",
            "block feed initialization failed",
        )
        .await?;

        info!("block feed connected to node");
        Ok(pair)
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

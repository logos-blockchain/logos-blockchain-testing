use testing_framework_core::{
    scenario::{BlockFeed, BlockFeedTask, NodeClients},
    topology::generation::GeneratedTopology,
};
use tracing::info;

use crate::{
    errors::ComposeRunnerError,
    infrastructure::{environment::StackEnvironment, ports::HostPortMapping},
    lifecycle::{
        block_feed::spawn_block_feed_with_retry, readiness::build_node_clients_with_ports,
    },
};

pub struct ClientBuilder;

impl ClientBuilder {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    pub async fn build_node_clients(
        &self,
        descriptors: &GeneratedTopology,
        host_ports: &HostPortMapping,
        host: &str,
        environment: &mut StackEnvironment,
    ) -> Result<NodeClients, ComposeRunnerError> {
        let clients = match build_node_clients_with_ports(descriptors, host_ports, host) {
            Ok(clients) => clients,
            Err(err) => {
                return Err(fail_deploy_step(
                    environment,
                    "failed to construct node api clients",
                    "failed to build node clients",
                    err,
                )
                .await);
            }
        };
        Ok(clients)
    }

    pub async fn start_block_feed(
        &self,
        node_clients: &NodeClients,
        environment: &mut StackEnvironment,
    ) -> Result<(BlockFeed, BlockFeedTask), ComposeRunnerError> {
        let pair = match spawn_block_feed_with_retry(node_clients).await {
            Ok(pair) => pair,
            Err(err) => {
                return Err(fail_deploy_step(
                    environment,
                    "failed to initialize block feed",
                    "block feed initialization failed",
                    err,
                )
                .await);
            }
        };
        info!("block feed connected to validator");
        Ok(pair)
    }
}

async fn fail_deploy_step<E>(
    environment: &mut StackEnvironment,
    reason: &str,
    log_message: &str,
    error: E,
) -> ComposeRunnerError
where
    E: std::fmt::Debug + Into<ComposeRunnerError>,
{
    environment.fail(reason).await;
    tracing::warn!(error = ?error, "{log_message}");
    error.into()
}

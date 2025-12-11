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
        match build_node_clients_with_ports(descriptors, host_ports, host) {
            Ok(clients) => Ok(clients),
            Err(err) => {
                environment
                    .fail("failed to construct node api clients")
                    .await;
                tracing::warn!(error = ?err, host, "failed to build node clients");
                Err(err.into())
            }
        }
    }

    pub async fn start_block_feed(
        &self,
        node_clients: &NodeClients,
        environment: &mut StackEnvironment,
    ) -> Result<(BlockFeed, BlockFeedTask), ComposeRunnerError> {
        match spawn_block_feed_with_retry(node_clients).await {
            Ok(pair) => {
                info!("block feed connected to validator");
                Ok(pair)
            }
            Err(err) => {
                environment.fail("failed to initialize block feed").await;
                tracing::warn!(error = ?err, "block feed initialization failed");
                Err(err)
            }
        }
    }
}

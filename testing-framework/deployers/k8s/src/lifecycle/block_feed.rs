use testing_framework_core::scenario::{BlockFeed, BlockFeedTask, NodeClients, spawn_block_feed};
use tracing::{debug, info};

use crate::deployer::K8sRunnerError;

pub async fn spawn_block_feed_with(
    node_clients: &NodeClients,
) -> Result<(BlockFeed, BlockFeedTask), K8sRunnerError> {
    debug!(
        validators = node_clients.validator_clients().len(),
        executors = node_clients.executor_clients().len(),
        "selecting node client for block feed"
    );

    let block_source_client = node_clients
        .validator_clients()
        .into_iter()
        .next()
        .or_else(|| node_clients.any_client())
        .ok_or(K8sRunnerError::BlockFeedMissing)?;

    info!("starting block feed");
    spawn_block_feed(block_source_client)
        .await
        .map_err(|source| K8sRunnerError::BlockFeed { source })
}

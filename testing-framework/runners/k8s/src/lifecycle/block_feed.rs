use testing_framework_core::scenario::{BlockFeed, BlockFeedTask, NodeClients, spawn_block_feed};
use tracing::info;

use crate::deployer::K8sRunnerError;

pub async fn spawn_block_feed_with(
    node_clients: &NodeClients,
) -> Result<(BlockFeed, BlockFeedTask), K8sRunnerError> {
    let block_source_client = node_clients
        .any_client()
        .cloned()
        .ok_or(K8sRunnerError::BlockFeedMissing)?;

    info!("starting block feed");
    spawn_block_feed(block_source_client)
        .await
        .map_err(|source| K8sRunnerError::BlockFeed { source })
}

use testing_framework_core::scenario::{
    Application, FeedHandle, FeedRuntime, NodeClients, spawn_feed,
};
use tracing::{debug, info};

use crate::deployer::K8sRunnerError;

pub async fn spawn_block_feed_with<E: Application>(
    node_clients: &NodeClients<E>,
) -> Result<
    (
        <<E as Application>::FeedRuntime as FeedRuntime>::Feed,
        FeedHandle,
    ),
    K8sRunnerError,
> {
    debug!(
        nodes = node_clients.len(),
        "selecting node client for block feed"
    );

    let block_source_client = node_clients
        .random_client()
        .ok_or(K8sRunnerError::BlockFeedMissing)?;

    info!("starting block feed");
    spawn_feed::<E>(block_source_client)
        .await
        .map_err(|source| K8sRunnerError::BlockFeed { source })
}

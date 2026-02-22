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
    let node_count = node_clients.len();
    debug!(nodes = node_count, "starting k8s block feed");

    if node_count == 0 {
        return Err(K8sRunnerError::BlockFeedMissing);
    }

    info!("starting block feed");
    spawn_feed::<E>(node_clients.clone())
        .await
        .map_err(|source| K8sRunnerError::BlockFeed { source })
}

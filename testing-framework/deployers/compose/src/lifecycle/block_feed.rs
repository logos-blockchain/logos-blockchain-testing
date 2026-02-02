use std::time::Duration;

use testing_framework_core::scenario::{
    Application, FeedHandle, FeedRuntime, NodeClients, spawn_feed,
};
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::errors::ComposeRunnerError;

const BLOCK_FEED_MAX_ATTEMPTS: usize = 5;
const BLOCK_FEED_RETRY_DELAY: Duration = Duration::from_secs(1);

async fn spawn_block_feed_with<E: Application>(
    node_clients: &NodeClients<E>,
) -> Result<
    (
        <<E as Application>::FeedRuntime as FeedRuntime>::Feed,
        FeedHandle,
    ),
    ComposeRunnerError,
> {
    debug!(
        nodes = node_clients.len(),
        "selecting node client for block feed"
    );

    let block_source_client = node_clients
        .random_client()
        .ok_or(ComposeRunnerError::BlockFeedMissing)?;

    spawn_feed::<E>(block_source_client)
        .await
        .map_err(|source| ComposeRunnerError::BlockFeed { source })
}

pub async fn spawn_block_feed_with_retry<E: Application>(
    node_clients: &NodeClients<E>,
) -> Result<
    (
        <<E as Application>::FeedRuntime as FeedRuntime>::Feed,
        FeedHandle,
    ),
    ComposeRunnerError,
> {
    for attempt in 1..=BLOCK_FEED_MAX_ATTEMPTS {
        info!(attempt, "starting block feed");
        match spawn_block_feed_with(node_clients).await {
            Ok(result) => {
                info!(attempt, "block feed established");
                return Ok(result);
            }

            Err(error) => {
                if attempt == BLOCK_FEED_MAX_ATTEMPTS {
                    return Err(error);
                }

                warn!(attempt, "block feed initialization failed; retrying");
                sleep(BLOCK_FEED_RETRY_DELAY).await;
            }
        }
    }

    unreachable!("retry loop always returns on success or final failure")
}

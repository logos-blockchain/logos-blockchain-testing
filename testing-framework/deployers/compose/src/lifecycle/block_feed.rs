use std::time::Duration;

use testing_framework_core::scenario::{BlockFeed, BlockFeedTask, NodeClients, spawn_block_feed};
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::errors::ComposeRunnerError;

const BLOCK_FEED_MAX_ATTEMPTS: usize = 5;
const BLOCK_FEED_RETRY_DELAY: Duration = Duration::from_secs(1);

async fn spawn_block_feed_with(
    node_clients: &NodeClients,
) -> Result<(BlockFeed, BlockFeedTask), ComposeRunnerError> {
    debug!(
        validators = node_clients.validator_clients().len(),
        executors = node_clients.executor_clients().len(),
        "selecting validator client for block feed"
    );

    let block_source_client = node_clients
        .random_validator()
        .ok_or(ComposeRunnerError::BlockFeedMissing)?;

    spawn_block_feed(block_source_client)
        .await
        .map_err(|source| ComposeRunnerError::BlockFeed { source })
}

pub async fn spawn_block_feed_with_retry(
    node_clients: &NodeClients,
) -> Result<(BlockFeed, BlockFeedTask), ComposeRunnerError> {
    let mut last_err = None;
    for attempt in 1..=BLOCK_FEED_MAX_ATTEMPTS {
        info!(attempt, "starting block feed");
        match spawn_block_feed_with(node_clients).await {
            Ok(result) => {
                info!(attempt, "block feed established");
                return Ok(result);
            }
            Err(err) => {
                last_err = Some(err);
                if attempt < BLOCK_FEED_MAX_ATTEMPTS {
                    warn!(attempt, "block feed initialization failed; retrying");
                    sleep(BLOCK_FEED_RETRY_DELAY).await;
                }
            }
        }
    }

    Err(last_err.unwrap_or(ComposeRunnerError::InternalInvariant {
        message: "block feed retry exhausted without capturing an error",
    }))
}

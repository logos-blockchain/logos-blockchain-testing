use std::time::Duration;

use testing_framework_core::{
    nodes::ApiClient,
    scenario::{DynError, NodeControlHandle, StartNodeOptions, StartedNode},
};
use thiserror::Error;
use tokio::time::{Instant, sleep, timeout};

#[derive(Debug, Error)]
pub enum ManualTestError {
    #[error("timeout: {message}")]
    Timeout { message: String },
    #[error("start node failed: {message}")]
    StartNode { message: String },
    #[error("consensus_info failed: {source}")]
    ConsensusInfo {
        #[from]
        source: reqwest::Error,
    },
}

pub async fn start_node_with_timeout<H: NodeControlHandle + ?Sized>(
    handle: &H,
    name: &str,
    options: StartNodeOptions,
    timeout_duration: Duration,
) -> Result<StartedNode, ManualTestError> {
    timeout(timeout_duration, handle.start_node_with(name, options))
        .await
        .map_err(|_| ManualTestError::Timeout {
            message: format!("starting node '{name}' exceeded timeout"),
        })?
        .map_err(|err: DynError| ManualTestError::StartNode {
            message: err.to_string(),
        })
}

pub async fn wait_for_min_height(
    clients: &[ApiClient],
    min_height: u64,
    timeout_duration: Duration,
    poll_interval: Duration,
) -> Result<(), ManualTestError> {
    let start = Instant::now();

    loop {
        let mut heights = Vec::with_capacity(clients.len());
        for client in clients {
            match client.consensus_info().await {
                Ok(info) => heights.push(info.height),
                Err(err) => {
                    if start.elapsed() >= timeout_duration {
                        return Err(ManualTestError::ConsensusInfo { source: err });
                    }
                    sleep(poll_interval).await;
                    continue;
                }
            }
        }

        if heights.len() == clients.len() && heights.iter().all(|height| *height >= min_height) {
            return Ok(());
        }

        if start.elapsed() >= timeout_duration {
            return Err(ManualTestError::Timeout {
                message: format!(
                    "min height {min_height} not reached before timeout; heights={heights:?}"
                ),
            });
        }

        sleep(poll_interval).await;
    }
}

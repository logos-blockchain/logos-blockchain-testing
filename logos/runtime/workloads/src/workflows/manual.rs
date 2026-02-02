use std::time::Duration;

use lb_framework::NodeHttpClient;
use testing_framework_core::scenario::{
    Application, DynError, NodeControlHandle, StartNodeOptions, StartedNode,
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
    ConsensusInfo { source: DynError },
}

pub async fn start_node_with_timeout<App, H>(
    handle: &H,
    name: &str,
    options: StartNodeOptions<App>,
    timeout_duration: Duration,
) -> Result<StartedNode<App>, ManualTestError>
where
    App: Application,
    H: NodeControlHandle<App> + ?Sized,
{
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
    clients: &[NodeHttpClient],
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
                        return Err(ManualTestError::ConsensusInfo { source: err.into() });
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

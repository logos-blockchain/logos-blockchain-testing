use std::time::Duration;

use anyhow::{Result, anyhow};
use testing_framework_core::{
    scenario::StartNodeOptions,
    topology::{
        config::{TopologyBuilder, TopologyConfig},
        configs::network::Libp2pNetworkLayout,
    },
};
use testing_framework_runner_local::LocalDeployer;
use testing_framework_workflows::{start_node_with_timeout, wait_for_min_height};
use tokio::time::{sleep, timeout};
use tracing_subscriber::fmt::try_init;

const MIN_HEIGHT: u64 = 5;
const INITIAL_READY_TIMEOUT: Duration = Duration::from_secs(500);
const CATCH_UP_TIMEOUT: Duration = Duration::from_secs(300);
const START_NODE_TIMEOUT: Duration = Duration::from_secs(90);
const TEST_TIMEOUT: Duration = Duration::from_secs(600);
const POLL_INTERVAL: Duration = Duration::from_secs(1);

#[tokio::test]
#[ignore = "run manually with `cargo test -p runner-examples -- --ignored orphan_manual_cluster`"]
async fn orphan_manual_cluster() -> Result<()> {
    let _ = try_init();
    // Required env vars (set on the command line when running this test):
    // - `LOGOS_BLOCKCHAIN_NODE_BIN=...`
    // - `NOMOS_KZGRS_PARAMS_PATH=...` (path to KZG params directory/file)
    // - `RUST_LOG=info` (optional; better visibility)

    let config = TopologyConfig::with_node_numbers(3);
    timeout(TEST_TIMEOUT, async {
        let builder = TopologyBuilder::new(config).with_network_layout(Libp2pNetworkLayout::Full);

        let deployer = LocalDeployer::new();
        let cluster = deployer.manual_cluster_with_builder(builder)?;
        // Nodes are stopped automatically when the cluster is dropped.

        let node_a = start_node_with_timeout(
            &cluster,
            "a",
            StartNodeOptions::default(),
            START_NODE_TIMEOUT,
        )
        .await?
        .api;

        let node_b = start_node_with_timeout(
            &cluster,
            "b",
            StartNodeOptions::default(),
            START_NODE_TIMEOUT,
        )
        .await?
        .api;

        wait_for_min_height(
            &[node_a.clone(), node_b.clone()],
            MIN_HEIGHT,
            INITIAL_READY_TIMEOUT,
            POLL_INTERVAL,
        )
        .await?;

        let behind_node = start_node_with_timeout(
            &cluster,
            "c",
            StartNodeOptions::default(),
            START_NODE_TIMEOUT,
        )
        .await?
        .api;

        timeout(CATCH_UP_TIMEOUT, async {
            loop {
                let node_a_info = node_a
                    .consensus_info()
                    .await
                    .map_err(|err| anyhow!("node-a consensus_info failed: {err}"))?;

                let node_b_info = node_b
                    .consensus_info()
                    .await
                    .map_err(|err| anyhow!("node-b consensus_info failed: {err}"))?;

                let behind_info = behind_node
                    .consensus_info()
                    .await
                    .map_err(|err| anyhow!("node-c consensus_info failed: {err}"))?;

                let initial_min_height = node_a_info.height.min(node_b_info.height);

                if behind_info.height >= initial_min_height.saturating_sub(1) {
                    return Ok::<(), anyhow::Error>(());
                }

                sleep(POLL_INTERVAL).await;
            }
        })
        .await
        .map_err(|_| anyhow!("timeout waiting for behind node to catch up"))??;

        Ok::<(), anyhow::Error>(())
    })
    .await
    .map_err(|_| anyhow!("test timeout exceeded"))??;

    Ok(())
}

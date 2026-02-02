use std::time::Duration;

use anyhow::{Result, anyhow};
use lb_framework::{
    DeploymentBuilder, LbcEnv, LbcLocalDeployer, NodeHttpClient, TopologyConfig,
    configs::network::NetworkLayout,
};
use lb_workloads::{start_node_with_timeout, wait_for_min_height};
use testing_framework_core::scenario::StartNodeOptions;
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
    timeout(TEST_TIMEOUT, run_orphan_flow(config))
        .await
        .map_err(|_| anyhow!("test timeout exceeded"))??;

    Ok(())
}

async fn run_orphan_flow(config: TopologyConfig) -> Result<()> {
    let builder = DeploymentBuilder::new(config).with_network_layout(NetworkLayout::Full);
    let deployer = LbcLocalDeployer::new();
    let descriptors = builder.build()?;
    let cluster = deployer.manual_cluster_from_descriptors(descriptors);

    let node_a = start_node_with_timeout(
        &cluster,
        "a",
        StartNodeOptions::<LbcEnv>::default(),
        START_NODE_TIMEOUT,
    )
    .await?
    .client;

    let node_b = start_node_with_timeout(
        &cluster,
        "b",
        StartNodeOptions::<LbcEnv>::default(),
        START_NODE_TIMEOUT,
    )
    .await?
    .client;

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
        StartNodeOptions::<LbcEnv>::default(),
        START_NODE_TIMEOUT,
    )
    .await?
    .client;

    wait_for_catch_up(&node_a, &node_b, &behind_node).await
}

async fn wait_for_catch_up(
    node_a: &NodeHttpClient,
    node_b: &NodeHttpClient,
    behind_node: &NodeHttpClient,
) -> Result<()> {
    timeout(CATCH_UP_TIMEOUT, async {
        loop {
            let node_a_height = node_height(node_a, "node-a").await?;
            let node_b_height = node_height(node_b, "node-b").await?;
            let behind_height = node_height(behind_node, "node-c").await?;

            let initial_min_height = node_a_height.min(node_b_height);
            if behind_height >= initial_min_height.saturating_sub(1) {
                return Ok::<(), anyhow::Error>(());
            }

            sleep(POLL_INTERVAL).await;
        }
    })
    .await
    .map_err(|_| anyhow!("timeout waiting for behind node to catch up"))?
}

async fn node_height(node: &NodeHttpClient, name: &str) -> Result<u64> {
    let info = node
        .consensus_info()
        .await
        .map_err(|error| anyhow!("{name} consensus_info failed: {error}"))?;

    Ok(info.height)
}

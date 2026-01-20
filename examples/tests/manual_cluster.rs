use std::time::Duration;

use anyhow::Result;
use testing_framework_core::{
    scenario::{PeerSelection, StartNodeOptions},
    topology::config::TopologyConfig,
};
use testing_framework_runner_local::LocalDeployer;
use tokio::time::sleep;
use tracing_subscriber::fmt::try_init;

const MAX_HEIGHT_DIFF: u64 = 5;

#[tokio::test]
#[ignore = "run manually with `cargo test -p runner-examples -- --ignored manual_cluster_two_clusters_merge`"]
async fn manual_cluster_two_clusters_merge() -> Result<()> {
    let _ = try_init();
    // Required env vars (set on the command line when running this test):
    // - `POL_PROOF_DEV_MODE=true`
    // - `RUST_LOG=info` (optional)
    let config = TopologyConfig::with_node_count(2);
    let deployer = LocalDeployer::new();
    let cluster = deployer.manual_cluster(config)?;
    // Nodes are stopped automatically when the cluster is dropped.

    println!("starting node a");

    let node_a = cluster
        .start_node_with(
            "a",
            StartNodeOptions {
                peers: PeerSelection::None,
            },
        )
        .await?
        .api;

    println!("waiting briefly before starting c");
    sleep(Duration::from_secs(30)).await;

    println!("starting node c -> a");
    let node_c = cluster
        .start_node_with(
            "c",
            StartNodeOptions {
                peers: PeerSelection::Named(vec!["node-a".to_owned()]),
            },
        )
        .await?
        .api;

    println!("waiting for network readiness: cluster a,c");
    cluster.wait_network_ready().await?;

    sleep(Duration::from_secs(5)).await;

    let a_info = node_a.consensus_info().await?;
    let c_info = node_c.consensus_info().await?;
    let height_diff = a_info.height.abs_diff(c_info.height);

    println!(
        "final heights: node-a={}, node-c={}, diff={}",
        a_info.height, c_info.height, height_diff
    );

    if height_diff > MAX_HEIGHT_DIFF {
        return Err(anyhow::anyhow!(
            "height diff too large: {height_diff} > {MAX_HEIGHT_DIFF}"
        ));
    }
    Ok(())
}

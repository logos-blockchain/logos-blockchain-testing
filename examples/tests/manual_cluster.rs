use std::{env, time::Duration};

use anyhow::Result;
use testing_framework_core::{
    scenario::{PeerSelection, StartNodeOptions},
    topology::config::TopologyConfig,
};
use testing_framework_runner_local::ManualCluster;
use tokio::time::sleep;
use tracing_subscriber::fmt::try_init;

const MAX_HEIGHT_DIFF: u64 = 5;

#[tokio::test]
#[ignore = "run manually with `cargo test -p runner-examples -- --ignored manual_cluster_two_clusters_merge`"]
async fn manual_cluster_two_clusters_merge() -> Result<()> {
    let _ = try_init();
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("examples crate should live under the workspace root");
    let circuits_dir = workspace_root.join("testing-framework/assets/stack/circuits");
    unsafe {
        env::set_var("LOGOS_BLOCKCHAIN_CIRCUITS", circuits_dir);
    }
    // Required env vars (set on the command line when running this test):
    // - `POL_PROOF_DEV_MODE=true`
    // - `RUST_LOG=info` (optional)
    let config = TopologyConfig::with_node_numbers(2, 0);
    let cluster = ManualCluster::from_config(config)?;
    // Nodes are stopped automatically when the cluster is dropped.

    println!("starting validator a");

    let validator_a = cluster
        .start_validator_with(
            "a",
            StartNodeOptions {
                peers: PeerSelection::None,
            },
        )
        .await?
        .api;

    println!("waiting briefly before starting c");
    sleep(Duration::from_secs(30)).await;

    println!("starting validator c -> a");
    let validator_c = cluster
        .start_validator_with(
            "c",
            StartNodeOptions {
                peers: PeerSelection::Named(vec!["validator-a".to_owned()]),
            },
        )
        .await?
        .api;

    println!("waiting for network readiness: cluster a,c");
    cluster.wait_network_ready().await?;

    sleep(Duration::from_secs(5)).await;

    let a_info = validator_a.consensus_info().await?;
    let c_info = validator_c.consensus_info().await?;
    let height_diff = a_info.height.abs_diff(c_info.height);

    println!(
        "final heights: validator-a={}, validator-c={}, diff={}",
        a_info.height, c_info.height, height_diff
    );

    if height_diff > MAX_HEIGHT_DIFF {
        return Err(anyhow::anyhow!(
            "height diff too large: {height_diff} > {MAX_HEIGHT_DIFF}"
        ));
    }
    Ok(())
}

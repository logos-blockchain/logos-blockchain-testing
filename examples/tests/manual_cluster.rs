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
const CONVERGENCE_TIMEOUT: Duration = Duration::from_secs(60);
const CONVERGENCE_POLL: Duration = Duration::from_secs(2);

#[tokio::test]
#[ignore = "run manually with `cargo test -p runner-examples -- --ignored manual_cluster_two_clusters_merge`"]
async fn manual_cluster_two_clusters_merge() -> Result<()> {
    let _ = try_init();
    // Required env vars (set on the command line when running this test):
    // - `RUST_LOG=info` (optional)
    let config = TopologyConfig::with_node_numbers(2);
    let deployer = LocalDeployer::new();
    let cluster = deployer.manual_cluster(config)?;
    // Nodes are stopped automatically when the cluster is dropped.

    println!("starting node a");

    let node_a = cluster
        .start_node_with(
            "a",
            StartNodeOptions {
                peers: PeerSelection::None,
                config_patch: None,
                persist_dir: None,
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
                config_patch: None,
                persist_dir: None,
            },
        )
        .await?
        .api;

    println!("waiting for network readiness: cluster a,c");
    cluster.wait_network_ready().await?;

    let start = tokio::time::Instant::now();

    loop {
        let a_info = node_a.consensus_info().await?;
        let c_info = node_c.consensus_info().await?;
        let a_height = a_info.height;
        let c_height = c_info.height;
        let diff = a_height.abs_diff(c_height);

        if diff <= MAX_HEIGHT_DIFF {
            println!(
                "final heights: node-a={}, node-c={}, diff={}",
                a_height, c_height, diff
            );
            return Ok(());
        }

        if start.elapsed() >= CONVERGENCE_TIMEOUT {
            return Err(anyhow::anyhow!(
                "height diff too large after timeout: {diff} > {MAX_HEIGHT_DIFF} (node-a={a_height}, node-c={c_height})"
            ));
        }

        sleep(CONVERGENCE_POLL).await;
    }
}

#[tokio::test]
#[ignore = "run manually with `cargo test -p runner-examples -- --ignored manual_cluster_with_persist_dir`"]
async fn manual_cluster_with_persist_dir() -> Result<()> {
    use std::path::PathBuf;

    let _ = try_init();
    // Required env vars (set on the command line when running this test):
    // - `RUST_LOG=info` (optional)
    let config = TopologyConfig::with_node_numbers(1);
    let deployer = LocalDeployer::new();
    let cluster = deployer.manual_cluster(config)?;

    let persist_dir = PathBuf::from("/tmp/test-node-persist-dir");

    println!("starting validator with persist_dir: {:?}", persist_dir);

    let _node = cluster
        .start_node_with(
            "test",
            StartNodeOptions {
                peers: PeerSelection::None,
                config_patch: None,
                persist_dir: Some(persist_dir.clone()),
            },
        )
        .await?
        .api;

    println!("validator started, waiting briefly");
    sleep(Duration::from_secs(5)).await;

    // Drop the cluster to trigger the persist logic
    drop(cluster);

    println!("cluster dropped, checking if persist_dir exists");

    // Verify the persist_dir was created
    if !persist_dir.exists() {
        return Err(anyhow::anyhow!(
            "persist_dir was not created: {:?}",
            persist_dir
        ));
    }

    println!("persist_dir verified: {:?}", persist_dir);

    // Clean up
    if persist_dir.exists() {
        std::fs::remove_dir_all(&persist_dir)?;
    }

    Ok(())
}

use std::time::Duration;

use anyhow::{Result, anyhow};
use lb_framework::{DeploymentBuilder, LbcEnv, LbcLocalDeployer, NodeHttpClient, TopologyConfig};
use testing_framework_core::scenario::{PeerSelection, StartNodeOptions};
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
    // - `POL_PROOF_DEV_MODE=true`
    // - `RUST_LOG=info` (optional)
    let config = TopologyConfig::with_node_numbers(2);
    let deployer = LbcLocalDeployer::new();
    let descriptors = DeploymentBuilder::new(config).build()?;
    let cluster = deployer.manual_cluster_from_descriptors(descriptors);
    // Nodes are stopped automatically when the cluster is dropped.

    println!("starting node a");

    let node_a = cluster
        .start_node_with("a", node_start_options(PeerSelection::None))
        .await?
        .client;

    println!("waiting briefly before starting c");
    sleep(Duration::from_secs(30)).await;

    println!("starting node c -> a");
    let node_c = cluster
        .start_node_with(
            "c",
            node_start_options(PeerSelection::Named(vec!["node-a".to_owned()])),
        )
        .await?
        .client;

    println!("waiting for network readiness: cluster a,c");
    cluster.wait_network_ready().await?;

    wait_for_convergence(&node_a, &node_c).await
}

async fn wait_for_convergence(node_a: &NodeHttpClient, node_c: &NodeHttpClient) -> Result<()> {
    let start = tokio::time::Instant::now();

    loop {
        let a_height = node_a.consensus_info().await?.height;
        let c_height = node_c.consensus_info().await?.height;
        let diff = a_height.abs_diff(c_height);

        if diff <= MAX_HEIGHT_DIFF {
            println!("final heights: node-a={a_height}, node-c={c_height}, diff={diff}");
            return Ok(());
        }

        if start.elapsed() >= CONVERGENCE_TIMEOUT {
            return Err(anyhow!(
                "height diff too large after timeout: {diff} > {MAX_HEIGHT_DIFF} (node-a={a_height}, node-c={c_height})"
            ));
        }

        sleep(CONVERGENCE_POLL).await;
    }
}

fn node_start_options(peers: PeerSelection) -> StartNodeOptions<LbcEnv> {
    let mut options = StartNodeOptions::<LbcEnv>::default();
    options.peers = peers;
    options
}

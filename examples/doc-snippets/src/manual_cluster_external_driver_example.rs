use std::time::Duration;
use anyhow::Result;
use testing_framework_core::{
    scenario::{PeerSelection, StartNodeOptions},
    topology::config::TopologyConfig,
};
use testing_framework_runner_local::LocalDeployer;
use tokio::time::sleep;

#[allow(dead_code)]
async fn external_driver_example() -> Result<()> {
    // Step 1: Create cluster with capacity for 3 nodes
    let config = TopologyConfig::with_node_numbers(3);
    let deployer = LocalDeployer::new();
    let cluster = deployer.manual_cluster(config)?;

    // Step 2: External driver decides to start 2 nodes initially
    println!("Starting initial topology...");
    let node_a = cluster.start_node("a").await?.api;
    let node_b = cluster
        .start_node_with(
            "b",
            StartNodeOptions {
                peers: PeerSelection::Named(vec!["node-a".to_owned()]),
            },
        )
        .await?
        .api;

    cluster.wait_network_ready().await?;

    // Step 3: External driver runs some protocol operations
    let info = node_a.consensus_info().await?;
    println!("Initial cluster height: {}", info.height);

    // Step 4: Later, external driver decides to add third node
    println!("External driver adding third node...");
    let node_c = cluster
        .start_node_with(
            "c",
            StartNodeOptions {
                peers: PeerSelection::Named(vec!["node-a".to_owned()]),
            },
        )
        .await?
        .api;

    cluster.wait_network_ready().await?;

    // Step 5: External driver validates final state
    let heights = vec![
        node_a.consensus_info().await?.height,
        node_b.consensus_info().await?.height,
        node_c.consensus_info().await?.height,
    ];
    println!("Final heights: {:?}", heights);

    Ok(())
}

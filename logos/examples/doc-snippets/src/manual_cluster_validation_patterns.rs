use std::time::Duration;
use testing_framework_core::nodes::ApiClient;
use tokio::time::sleep;

#[allow(dead_code)]
async fn height_convergence(
    node_a: &ApiClient,
    node_b: &ApiClient,
    node_c: &ApiClient,
) -> anyhow::Result<()> {
    let start = tokio::time::Instant::now();
    loop {
        let heights: Vec<u64> = vec![
            node_a.consensus_info().await?.height,
            node_b.consensus_info().await?.height,
            node_c.consensus_info().await?.height,
        ];

        let max_diff = heights.iter().max().unwrap() - heights.iter().min().unwrap();
        if max_diff <= 5 {
            println!("Converged: heights={:?}", heights);
            break;
        }

        if start.elapsed() > Duration::from_secs(60) {
            return Err(anyhow::anyhow!("Convergence timeout: heights={:?}", heights));
        }

        sleep(Duration::from_secs(2)).await;
    }
    Ok(())
}

#[allow(dead_code)]
async fn peer_count_verification(node: &ApiClient) -> anyhow::Result<()> {
    let info = node.network_info().await?;
    assert_eq!(
        info.n_peers, 3,
        "Expected 3 peers, found {}",
        info.n_peers
    );
    Ok(())
}

#[allow(dead_code)]
async fn block_production(node_a: &ApiClient) -> anyhow::Result<()> {
    // Verify node is producing blocks
    let initial_height = node_a.consensus_info().await?.height;

    sleep(Duration::from_secs(10)).await;

    let current_height = node_a.consensus_info().await?.height;
    assert!(
        current_height > initial_height,
        "Node should have produced blocks: initial={}, current={}",
        initial_height,
        current_height
    );
    Ok(())
}

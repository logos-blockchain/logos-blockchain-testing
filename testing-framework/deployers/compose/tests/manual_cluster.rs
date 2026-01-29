use std::{path::Path, time::Duration};

use anyhow::Result;
use testing_framework_core::topology::config::TopologyConfig;
use testing_framework_runner_compose::ComposeDeployer;
use tokio::time::{sleep, timeout};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(180);
const CONSENSUS_POLL_TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::test(flavor = "multi_thread")]
async fn manual_cluster_compose_single_node() -> Result<()> {
    // Note: Prefer letting the image use its bundled /opt/circuits.
    // If you need to override circuits, set:
    //   LOGOS_BLOCKCHAIN_CIRCUITS=/path/to/host/circuits
    //   LOGOS_BLOCKCHAIN_CIRCUITS_DOCKER=/path/to/linux/circuits
    // and ensure they match the node/cfgsync versions.
    unsafe {
        std::env::set_var("POL_PROOF_DEV_MODE", "true");
    }

    unsafe {
        std::env::set_var(
            "LOGOS_BLOCKCHAIN_TESTNET_IMAGE",
            "logos-blockchain-testing:local",
        );
    }

    if let Ok(host_circuits) = std::env::var("LOGOS_BLOCKCHAIN_CIRCUITS") {
        if !Path::new(&host_circuits).exists() {
            return Err(anyhow::anyhow!(
                "host circuits directory not found at {host_circuits}"
            ));
        }
    }

    if let Ok(docker_circuits) = std::env::var("LOGOS_BLOCKCHAIN_CIRCUITS_DOCKER") {
        if !Path::new(&docker_circuits).exists() {
            return Err(anyhow::anyhow!(
                "docker circuits directory not found at {docker_circuits}"
            ));
        }
    }

    let config = TopologyConfig::with_node_numbers(1);
    let deployer = ComposeDeployer::new();
    let cluster = deployer.manual_cluster(config).await?;

    let node = cluster.start_node("seed").await?.api;

    let start = tokio::time::Instant::now();
    loop {
        match timeout(CONSENSUS_POLL_TIMEOUT, node.consensus_info()).await {
            Ok(Ok(_)) => break,
            Ok(Err(err)) => {
                if start.elapsed() >= STARTUP_TIMEOUT {
                    return Err(err.into());
                }
            }
            Err(_) => return Err(anyhow::anyhow!("consensus_info timed out")),
        }
        sleep(Duration::from_secs(2)).await;
    }

    Ok(())
}

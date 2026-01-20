use std::time::Duration;

use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use testing_framework_workflows::ScenarioBuilderExt;

pub async fn run_local_demo() -> Result<()> {
    // Define the scenario (2 nodes, tx workload)
    let mut plan = ScenarioBuilder::topology_with(|t| t.network_star().nodes(2))
        .wallets(1_000)
        .transactions_with(|txs| {
            txs.rate(5) // 5 transactions per block
                .users(500) // use 500 of the seeded wallets
        })
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(60))
        .build()?;

    // Deploy and run
    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&plan).await?;
    let _handle = runner.run(&mut plan).await?;

    Ok(())
}

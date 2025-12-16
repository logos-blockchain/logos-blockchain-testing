use std::time::Duration;

use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use testing_framework_workflows::ScenarioBuilderExt;

pub async fn run_local_demo() -> Result<()> {
    // Define the scenario (1 validator + 1 executor, tx + DA workload)
    let mut plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(1).executors(1))
        .wallets(1_000)
        .transactions_with(|txs| {
            txs.rate(5) // 5 transactions per block
                .users(500) // use 500 of the seeded wallets
        })
        .da_with(|da| {
            da.channel_rate(1) // 1 channel
                .blob_rate(1) // target 1 blob per block
                .headroom_percent(20) // default headroom when sizing channels
        })
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(60))
        .build();

    // Deploy and run
    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&plan).await?;
    let _handle = runner.run(&mut plan).await?;

    Ok(())
}

use std::time::Duration;

use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use testing_framework_workflows::ScenarioBuilderExt;

pub async fn run_test() -> Result<()> {
    let mut plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(3).executors(2))
        .wallets(50)
        .transactions_with(|txs| {
            txs.rate(5) // 5 transactions per block
                .users(20)
        })
        .da_with(|da| {
            da.channel_rate(1) // number of DA channels
                .blob_rate(2) // target 2 blobs per block
                .headroom_percent(20) // optional channel headroom
        })
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(90))
        .build();

    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&plan).await?;
    let _handle = runner.run(&mut plan).await?;

    Ok(())
}

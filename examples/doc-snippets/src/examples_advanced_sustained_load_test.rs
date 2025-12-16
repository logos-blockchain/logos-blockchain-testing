use std::time::Duration;

use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_compose::ComposeDeployer;
use testing_framework_workflows::ScenarioBuilderExt;

pub async fn sustained_load_test() -> Result<()> {
    let mut plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(4).executors(2))
        .wallets(100)
        .transactions_with(|txs| txs.rate(15).users(50))
        .da_with(|da| da.channel_rate(2).blob_rate(3))
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(300))
        .build();

    let deployer = ComposeDeployer::default();
    let runner = deployer.deploy(&plan).await?;
    let _handle = runner.run(&mut plan).await?;

    Ok(())
}

use std::time::Duration;

use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use testing_framework_workflows::ScenarioBuilderExt;

pub async fn transaction_workload() -> Result<()> {
    let mut plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(2).executors(0))
        .wallets(20)
        .transactions_with(|txs| txs.rate(5).users(10))
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(60))
        .build();

    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&plan).await?;
    let _handle = runner.run(&mut plan).await?;

    Ok(())
}

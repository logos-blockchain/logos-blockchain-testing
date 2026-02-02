use std::time::Duration;

use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use testing_framework_workflows::ScenarioBuilderExt;

pub async fn run_test() -> Result<()> {
    let mut plan = ScenarioBuilder::topology_with(|t| t.network_star().nodes(3))
        .wallets(50)
        .transactions_with(|txs| {
            txs.rate(5) // 5 transactions per block
                .users(20)
        })
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(90))
        .build()?;

    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&plan).await?;
    let _handle = runner.run(&mut plan).await?;

    Ok(())
}

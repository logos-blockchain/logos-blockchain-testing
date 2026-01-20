use std::time::Duration;

use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_compose::ComposeDeployer;
use testing_framework_workflows::ScenarioBuilderExt;

pub async fn load_progression_test() -> Result<()> {
    for rate in [5, 10, 20, 30] {
        println!("Testing with rate: {}", rate);

        let mut plan = ScenarioBuilder::topology_with(|t| t.network_star().nodes(5))
            .wallets(50)
            .transactions_with(|txs| txs.rate(rate).users(20))
            .expect_consensus_liveness()
            .with_run_duration(Duration::from_secs(60))
            .build()?;

        let deployer = ComposeDeployer::default();
        let runner = deployer.deploy(&plan).await?;
        let _handle = runner.run(&mut plan).await?;
    }

    Ok(())
}

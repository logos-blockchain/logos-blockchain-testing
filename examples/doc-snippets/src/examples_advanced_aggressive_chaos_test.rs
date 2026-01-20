use std::time::Duration;

use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_compose::ComposeDeployer;
use testing_framework_workflows::{ChaosBuilderExt, ScenarioBuilderExt};

pub async fn aggressive_chaos_test() -> Result<()> {
    let mut plan = ScenarioBuilder::topology_with(|t| t.network_star().nodes(6))
        .enable_node_control()
        .wallets(50)
        .transactions_with(|txs| txs.rate(10).users(20))
        .chaos_with(|c| {
            c.restart()
                .min_delay(Duration::from_secs(10))
                .max_delay(Duration::from_secs(20))
                .target_cooldown(Duration::from_secs(15))
                .apply()
        })
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(180))
        .build()?;

    let deployer = ComposeDeployer::default();
    let runner = deployer.deploy(&plan).await?;
    let _handle = runner.run(&mut plan).await?;

    Ok(())
}

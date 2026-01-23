use std::time::Duration;

use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_compose::ComposeDeployer;
use testing_framework_workflows::{ChaosBuilderExt, ScenarioBuilderExt};

pub async fn chaos_resilience() -> Result<()> {
    let mut plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(4))
        .enable_node_control()
        .wallets(20)
        .transactions_with(|txs| txs.rate(3).users(10))
        .chaos_with(|c| {
            c.restart()
                .min_delay(Duration::from_secs(20))
                .max_delay(Duration::from_secs(40))
                .target_cooldown(Duration::from_secs(30))
                .apply()
        })
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(120))
        .build()?;

    let deployer = ComposeDeployer::default();
    let runner = deployer.deploy(&plan).await?;
    let _handle = runner.run(&mut plan).await?;

    Ok(())
}

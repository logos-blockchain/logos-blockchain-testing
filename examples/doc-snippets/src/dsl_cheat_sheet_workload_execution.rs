use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use testing_framework_workflows::ScenarioBuilderExt;

pub async fn execution() -> Result<()> {
    let mut plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(1).executors(0))
        .expect_consensus_liveness()
        .build();

    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&plan).await?;
    let _handle = runner.run(&mut plan).await?;

    Ok(())
}

use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;

pub async fn step_6_deploy_and_execute() -> Result<()> {
    let mut plan = ScenarioBuilder::with_node_counts(1).build()?;

    let deployer = LocalDeployer::default(); // Use local process deployer
    let runner = deployer.deploy(&plan).await?; // Provision infrastructure
    let _handle = runner.run(&mut plan).await?; // Execute workloads & expectations

    Ok(())
}

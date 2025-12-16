use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_compose::ComposeDeployer;

pub async fn run_with_compose_deployer() -> Result<()> {
    // ... same scenario definition ...
    let mut plan = ScenarioBuilder::with_node_counts(1, 1).build();

    let deployer = ComposeDeployer::default(); // Use Docker Compose
    let runner = deployer.deploy(&plan).await?;
    let _handle = runner.run(&mut plan).await?;

    Ok(())
}

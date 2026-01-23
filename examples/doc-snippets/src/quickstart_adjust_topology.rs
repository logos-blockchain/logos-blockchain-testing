use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;

pub async fn run_with_env_overrides() -> Result<()> {
    // Uses NOMOS_DEMO_* env vars (or legacy *_DEMO_* vars)
    let mut plan = ScenarioBuilder::with_node_counts(3)
        .with_run_duration(std::time::Duration::from_secs(120))
        .build()?;

    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&plan).await?;
    let _handle = runner.run(&mut plan).await?;

    Ok(())
}

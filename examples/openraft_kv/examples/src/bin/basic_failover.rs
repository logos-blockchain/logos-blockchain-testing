use std::time::Duration;

use openraft_kv_examples::build_failover_scenario;
use testing_framework_app::AppHostDeployer;
use testing_framework_runner_local::LocalClusterProvisioner;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut scenario = build_failover_scenario(
        Duration::from_secs(45),
        Duration::from_secs(30),
        LocalClusterProvisioner,
    )?;

    let runner = AppHostDeployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;

    Ok(())
}

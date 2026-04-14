use std::time::Duration;

use openraft_kv_examples::build_failover_scenario;
use openraft_kv_runtime_ext::OpenRaftKvLocalDeployer;
use testing_framework_core::scenario::Deployer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut scenario = build_failover_scenario(Duration::from_secs(45), Duration::from_secs(30))?;

    let deployer = OpenRaftKvLocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;

    Ok(())
}

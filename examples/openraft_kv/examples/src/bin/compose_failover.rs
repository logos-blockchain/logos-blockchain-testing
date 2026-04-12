use std::time::Duration;

use openraft_kv_examples::build_failover_scenario;
use openraft_kv_runtime_ext::OpenRaftKvComposeDeployer;
use testing_framework_core::scenario::Deployer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut scenario = build_failover_scenario(Duration::from_secs(60), Duration::from_secs(40))?;

    let deployer = OpenRaftKvComposeDeployer::new();
    let runner = deployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;

    Ok(())
}

use std::time::Duration;

use nats_runtime_ext::NatsLocalDeployer;
use nats_runtime_workloads::{
    NatsBuilderExt, NatsClusterHealthy, NatsRoundTripWorkload, NatsScenarioBuilder,
};
use testing_framework_core::scenario::Deployer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,testing_framework_core=info".into()),
        )
        .init();

    let mut scenario = NatsScenarioBuilder::deployment_with(|topology| topology)
        .with_run_duration(Duration::from_secs(25))
        .with_workload(NatsRoundTripWorkload::new("tf.roundtrip").messages(200))
        .with_expectation(NatsClusterHealthy::new())
        .build()?;

    let deployer = NatsLocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;
    Ok(())
}

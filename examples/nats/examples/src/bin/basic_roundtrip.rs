use std::time::Duration;

use nats_runtime_workloads::{NatsClusterHealthy, NatsEnv, NatsRoundTripWorkload, NatsTopology};
use testing_framework_app::{AppHost, AppHostDeployer, AppScenarioBuilderExt as _, ClusterApp};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,testing_framework_core=info".into()),
        )
        .init();

    let mut scenario = AppHost::scenario()
        .with_app(ClusterApp::<NatsEnv>::new(NatsTopology::new(3)))
        .with_run_duration(Duration::from_secs(25))
        .with_workload(NatsRoundTripWorkload::new("tf.roundtrip").messages(200))
        .with_expectation(NatsClusterHealthy::new())
        .build()?;

    let deployer = AppHostDeployer;
    let runner = deployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;
    Ok(())
}

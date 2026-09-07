use std::time::Duration;

use queue_runtime_workloads::{QueueConverges, QueueEnv, QueueProduceWorkload, QueueTopology};
use testing_framework_app::{AppHost, AppHostDeployer, AppScenarioBuilderExt as _, ClusterApp};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let operations = 300;

    let mut scenario = AppHost::scenario()
        .with_app(ClusterApp::<QueueEnv>::new(QueueTopology::new(3)))
        .with_run_duration(Duration::from_secs(30))
        .with_workload(
            QueueProduceWorkload::new()
                .operations(operations)
                .rate_per_sec(30)
                .payload_prefix("demo"),
        )
        .with_expectation(QueueConverges::new(operations).timeout(Duration::from_secs(25)))
        .build()?;

    let runner = AppHostDeployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;
    Ok(())
}

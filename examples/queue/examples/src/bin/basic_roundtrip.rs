use std::time::Duration;

use queue_runtime_ext::QueueLocalDeployer;
use queue_runtime_workloads::{
    QueueBuilderExt, QueueDrained, QueueRoundTripWorkload, QueueScenarioBuilder, QueueTopology,
};
use testing_framework_core::scenario::Deployer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let operations = 200;

    let mut scenario = QueueScenarioBuilder::deployment_with(|_| QueueTopology::new(3))
        .with_run_duration(Duration::from_secs(30))
        .with_workload(
            QueueRoundTripWorkload::new()
                .operations(operations)
                .rate_per_sec(25),
        )
        .with_expectation(QueueDrained::new().timeout(Duration::from_secs(25)))
        .build()?;

    let deployer = QueueLocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;
    Ok(())
}

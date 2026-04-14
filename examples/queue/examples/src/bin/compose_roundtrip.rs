use std::time::Duration;

use anyhow::{Context as _, Result};
use queue_runtime_workloads::{
    QueueBuilderExt, QueueDrained, QueueRoundTripWorkload, QueueScenarioBuilder, QueueTopology,
};
use testing_framework_core::scenario::Deployer;
use testing_framework_runner_compose::ComposeRunnerError;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let operations = 200;

    let mut scenario = QueueScenarioBuilder::deployment_with(|_| QueueTopology::new(3))
        .with_run_duration(Duration::from_secs(30))
        .with_workload(
            QueueRoundTripWorkload::new()
                .operations(operations)
                .rate_per_sec(20),
        )
        .with_expectation(QueueDrained::new().timeout(Duration::from_secs(25)))
        .build()?;

    let deployer = queue_runtime_ext::QueueComposeDeployer::new();
    let runner = match deployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(ComposeRunnerError::DockerUnavailable) => {
            warn!("docker unavailable; skipping compose queue roundtrip run");
            return Ok(());
        }
        Err(error) => {
            return Err(anyhow::Error::new(error))
                .context("deploying queue compose roundtrip stack");
        }
    };

    info!("running queue compose roundtrip scenario");
    runner
        .run(&mut scenario)
        .await
        .context("running queue compose roundtrip scenario")?;
    Ok(())
}

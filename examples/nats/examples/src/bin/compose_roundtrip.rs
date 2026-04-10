use std::time::Duration;

use anyhow::{Context as _, Result};
use nats_runtime_ext::NatsComposeDeployer;
use nats_runtime_workloads::{
    NatsBuilderExt, NatsClusterHealthy, NatsRoundTripWorkload, NatsScenarioBuilder,
};
use testing_framework_core::scenario::Deployer;
use testing_framework_runner_compose::ComposeRunnerError;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,testing_framework_core=info".into()),
        )
        .init();

    let mut scenario = NatsScenarioBuilder::deployment_with(|topology| topology)
        .with_run_duration(Duration::from_secs(30))
        .with_workload(NatsRoundTripWorkload::new("tf.roundtrip").messages(200))
        .with_expectation(NatsClusterHealthy::new())
        .build()?;

    let deployer = NatsComposeDeployer::new();
    let runner = match deployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(ComposeRunnerError::DockerUnavailable) => {
            warn!("docker unavailable; skipping compose nats run");
            return Ok(());
        }
        Err(error) => {
            return Err(anyhow::Error::new(error)).context("deploying nats compose stack");
        }
    };

    info!("running nats compose roundtrip scenario");
    runner
        .run(&mut scenario)
        .await
        .context("running nats compose scenario")?;
    Ok(())
}

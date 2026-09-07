use std::time::Duration;

use anyhow::{Context as _, Result};
use queue_runtime_workloads::{QueueDrained, QueueEnv, QueueRoundTripWorkload, QueueTopology};
use testing_framework_app::{
    AppHost, AppHostDeployError, AppHostDeployer, AppScenarioBuilderExt as _, ClusterApp,
};
use testing_framework_runner_compose::{ComposeProvisioner, ComposeRunnerError};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let operations = 200;

    let mut scenario = AppHost::scenario()
        .with_app_using(
            ClusterApp::<QueueEnv>::new(QueueTopology::new(3)),
            ComposeProvisioner::default(),
        )
        .with_run_duration(Duration::from_secs(30))
        .with_workload(
            QueueRoundTripWorkload::new()
                .operations(operations)
                .rate_per_sec(20),
        )
        .with_expectation(QueueDrained::new().timeout(Duration::from_secs(25)))
        .build()?;

    let runner = match AppHostDeployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(error) if is_docker_unavailable(&error) => {
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

fn is_docker_unavailable(error: &AppHostDeployError) -> bool {
    let AppHostDeployError::RuntimeExtensions { source } = error else {
        return false;
    };
    matches!(
        source.downcast_ref::<ComposeRunnerError>(),
        Some(ComposeRunnerError::DockerUnavailable)
    )
}

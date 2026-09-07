use std::time::Duration;

use anyhow::{Context as _, Result};
use nats_runtime_workloads::{NatsClusterHealthy, NatsEnv, NatsRoundTripWorkload, NatsTopology};
use testing_framework_app::{
    AppHost, AppHostDeployError, AppHostDeployer, AppScenarioBuilderExt as _, ClusterApp,
};
use testing_framework_runner_compose::{ComposeProvisioner, ComposeRunnerError};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,testing_framework_core=info".into()),
        )
        .init();

    let mut scenario = AppHost::scenario()
        .with_app_using(
            ClusterApp::<NatsEnv>::new(NatsTopology::new(3)),
            ComposeProvisioner::default(),
        )
        .with_run_duration(Duration::from_secs(30))
        .with_workload(NatsRoundTripWorkload::new("tf.roundtrip").messages(200))
        .with_expectation(NatsClusterHealthy::new())
        .build()?;

    let deployer = AppHostDeployer;
    let runner = match deployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(error) if is_docker_unavailable(&error) => {
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

fn is_docker_unavailable(error: &AppHostDeployError) -> bool {
    match error {
        AppHostDeployError::RuntimeExtensions { source } => matches!(
            source.downcast_ref::<ComposeRunnerError>(),
            Some(ComposeRunnerError::DockerUnavailable)
        ),
        AppHostDeployError::Empty => false,
    }
}

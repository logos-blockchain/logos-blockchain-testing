use std::time::Duration;

use anyhow::{Context as _, Result};
use openraft_kv_examples::build_failover_scenario;
use testing_framework_app::{AppHostDeployError, AppHostDeployer};
use testing_framework_runner_compose::{ComposeProvisioner, ComposeRunnerError};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut scenario = build_failover_scenario(
        Duration::from_secs(60),
        Duration::from_secs(40),
        ComposeProvisioner::default(),
    )?;

    let runner = match AppHostDeployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(error) if docker_unavailable(&error) => {
            warn!("docker unavailable; skipping openraft compose failover run");
            return Ok(());
        }
        Err(error) => {
            return Err(anyhow::Error::new(error)).context("deploying openraft compose stack");
        }
    };

    info!("running openraft compose failover scenario");
    runner
        .run(&mut scenario)
        .await
        .context("running openraft compose failover scenario")?;

    Ok(())
}

fn docker_unavailable(error: &AppHostDeployError) -> bool {
    let AppHostDeployError::RuntimeExtensions { source } = error else {
        return false;
    };
    matches!(
        source.downcast_ref::<ComposeRunnerError>(),
        Some(ComposeRunnerError::DockerUnavailable)
    )
}

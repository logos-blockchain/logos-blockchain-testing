use std::time::Duration;

use anyhow::{Context as _, Result};
use nats_runtime_ext::{NatsComposeDeployer, NatsLocalDeployer};
use nats_runtime_workloads::{
    NatsBuilderExt, NatsClusterHealthy, NatsRoundTripWorkload, NatsScenarioBuilder,
};
use testing_framework_core::scenario::Deployer;
use testing_framework_runner_compose::ComposeRunnerError;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    run_compose().await?;
    run_local_if_available().await?;
    Ok(())
}

async fn run_compose() -> Result<()> {
    let mut scenario = build_scenario(Duration::from_secs(30)).build()?;
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

    info!("running nats compose parity check");
    runner
        .run(&mut scenario)
        .await
        .context("running nats compose scenario")?;
    Ok(())
}

async fn run_local_if_available() -> Result<()> {
    if !has_local_nats_server() {
        warn!(
            "nats-server binary not found; skipping local parity check (set NATS_SERVER_BIN or add to PATH)"
        );
        return Ok(());
    }

    let mut scenario = build_scenario(Duration::from_secs(25)).build()?;
    let deployer = NatsLocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;

    info!("running nats local parity check");
    runner.run(&mut scenario).await?;
    Ok(())
}

fn has_local_nats_server() -> bool {
    std::env::var("NATS_SERVER_BIN")
        .ok()
        .is_some_and(|path| std::path::Path::new(&path).exists())
        || which::which("nats-server").is_ok()
}

fn build_scenario(run_duration: Duration) -> NatsScenarioBuilder {
    NatsScenarioBuilder::deployment_with(|topology| topology)
        .with_run_duration(run_duration)
        .with_workload(NatsRoundTripWorkload::new("tf.roundtrip").messages(200))
        .with_expectation(NatsClusterHealthy::new())
}

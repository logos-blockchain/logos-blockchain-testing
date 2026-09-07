use std::time::Duration;

use anyhow::{Context as _, Result};
use nats_runtime_workloads::{NatsClusterHealthy, NatsEnv, NatsRoundTripWorkload, NatsTopology};
use testing_framework_app::{
    AppHost, AppHostDeployError, AppHostDeployer, AppScenarioBuilderExt as _, ClusterApp,
};
use testing_framework_core::scenario::ClusterProvisioner;
use testing_framework_runner_compose::{ComposeProvisioner, ComposeRunnerError};
use testing_framework_runner_local::LocalClusterProvisioner;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    run_parity(
        ComposeProvisioner::default(),
        Duration::from_secs(30),
        "compose",
    )
    .await?;
    run_local_if_available().await?;
    Ok(())
}

async fn run_local_if_available() -> Result<()> {
    if !has_local_nats_server() {
        warn!(
            "nats-server binary not found; skipping local parity check (set NATS_SERVER_BIN or add to PATH)"
        );
        return Ok(());
    }

    run_parity(LocalClusterProvisioner, Duration::from_secs(25), "local").await
}

async fn run_parity<P>(provisioner: P, run_duration: Duration, backend: &str) -> Result<()>
where
    P: ClusterProvisioner<NatsEnv> + Clone + Send + Sync + 'static,
{
    let mut scenario = AppHost::scenario()
        .with_app_using(
            ClusterApp::<NatsEnv>::new(NatsTopology::new(3)),
            provisioner,
        )
        .with_run_duration(run_duration)
        .with_workload(NatsRoundTripWorkload::new("tf.roundtrip").messages(200))
        .with_expectation(NatsClusterHealthy::new())
        .build()?;

    let deployer = AppHostDeployer;
    let runner = match deployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(error) if is_docker_unavailable(&error) => {
            warn!(backend, "docker unavailable; skipping nats parity run");
            return Ok(());
        }
        Err(error) => {
            return Err(anyhow::Error::new(error))
                .with_context(|| format!("deploying nats {backend} stack"));
        }
    };

    info!(backend, "running nats parity check");
    runner
        .run(&mut scenario)
        .await
        .with_context(|| format!("running nats {backend} scenario"))?;
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

fn has_local_nats_server() -> bool {
    std::env::var("NATS_SERVER_BIN")
        .ok()
        .is_some_and(|path| std::path::Path::new(&path).exists())
        || which::which("nats-server").is_ok()
}

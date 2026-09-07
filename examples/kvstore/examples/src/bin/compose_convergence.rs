use std::time::Duration;

use anyhow::{Context as _, Result};
use kvstore_runtime_workloads::{KvConverges, KvEnv, KvTopology, KvWriteWorkload};
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

    let mut scenario = AppHost::scenario()
        .with_app_using(
            ClusterApp::<KvEnv>::new(KvTopology::new(3)),
            ComposeProvisioner::default(),
        )
        .with_run_duration(Duration::from_secs(30))
        .with_workload(
            KvWriteWorkload::new()
                .operations(200)
                .key_count(20)
                .rate_per_sec(20),
        )
        .with_expectation(KvConverges::new("kv-demo", 20).timeout(Duration::from_secs(25)))
        .build()?;

    let runner = match AppHostDeployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(error) if docker_unavailable(&error) => {
            warn!("docker unavailable; skipping compose kv run");
            return Ok(());
        }
        Err(error) => return Err(anyhow::Error::new(error)).context("deploying kv compose stack"),
    };

    info!("running kv compose convergence scenario");
    runner
        .run(&mut scenario)
        .await
        .context("running kv compose scenario")?;
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

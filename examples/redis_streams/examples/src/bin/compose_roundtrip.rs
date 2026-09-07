use std::time::Duration;

use anyhow::{Context as _, Result};
use redis_streams_runtime_workloads::{
    RedisStreamsClusterHealthy, RedisStreamsEnv, RedisStreamsRoundTripWorkload,
    RedisStreamsTopology,
};
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
            ClusterApp::<RedisStreamsEnv>::new(RedisStreamsTopology::new(3)),
            ComposeProvisioner::default(),
        )
        .with_run_duration(Duration::from_secs(30))
        .with_workload(
            RedisStreamsRoundTripWorkload::new("tf-stream", "tf-group")
                .messages(300)
                .read_batch(50),
        )
        .with_expectation(RedisStreamsClusterHealthy::new())
        .build()?;

    let runner = match AppHostDeployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(error) if is_docker_unavailable(&error) => {
            warn!("docker unavailable; skipping redis streams compose run");
            return Ok(());
        }
        Err(error) => {
            return Err(anyhow::Error::new(error)).context("deploying redis streams compose stack");
        }
    };

    info!("running redis streams compose roundtrip scenario");
    runner
        .run(&mut scenario)
        .await
        .context("running redis streams compose scenario")?;

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

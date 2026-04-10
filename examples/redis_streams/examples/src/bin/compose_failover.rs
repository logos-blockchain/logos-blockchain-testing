use std::time::Duration;

use anyhow::{Context as _, Result};
use redis_streams_runtime_ext::RedisStreamsComposeDeployer;
use redis_streams_runtime_workloads::{
    RedisStreamsBuilderExt, RedisStreamsClusterHealthy, RedisStreamsReclaimFailoverWorkload,
    RedisStreamsScenarioBuilder,
};
use testing_framework_core::scenario::Deployer;
use testing_framework_runner_compose::ComposeRunnerError;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut scenario = RedisStreamsScenarioBuilder::deployment_with(|topology| topology)
        .with_run_duration(Duration::from_secs(30))
        .with_workload(
            RedisStreamsReclaimFailoverWorkload::new("tf-stream", "tf-group")
                .messages(300)
                .batch(64),
        )
        .with_expectation(RedisStreamsClusterHealthy::new())
        .build()?;

    let deployer = RedisStreamsComposeDeployer::new();
    let runner = match deployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(ComposeRunnerError::DockerUnavailable) => {
            warn!("docker unavailable; skipping redis streams compose failover run");
            return Ok(());
        }
        Err(error) => {
            return Err(anyhow::Error::new(error))
                .context("deploying redis streams compose failover stack");
        }
    };

    info!("running redis streams compose failover scenario");
    runner
        .run(&mut scenario)
        .await
        .context("running redis streams compose failover scenario")?;

    Ok(())
}

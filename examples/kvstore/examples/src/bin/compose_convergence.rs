use std::time::Duration;

use anyhow::{Context as _, Result};
use kvstore_runtime_workloads::{
    KvBuilderExt, KvConverges, KvScenarioBuilder, KvTopology, KvWriteWorkload,
};
use testing_framework_core::scenario::Deployer;
use testing_framework_runner_compose::ComposeRunnerError;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut scenario = KvScenarioBuilder::deployment_with(|_| KvTopology::new(3))
        .with_run_duration(Duration::from_secs(30))
        .with_workload(
            KvWriteWorkload::new()
                .operations(200)
                .key_count(20)
                .rate_per_sec(20),
        )
        .with_expectation(KvConverges::new("kv-demo", 20).timeout(Duration::from_secs(25)))
        .build()?;

    let deployer = kvstore_runtime_ext::KvComposeDeployer::new();
    let runner = match deployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(ComposeRunnerError::DockerUnavailable) => {
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

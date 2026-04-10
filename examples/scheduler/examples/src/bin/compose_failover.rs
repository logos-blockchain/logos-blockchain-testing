use std::time::Duration;

use anyhow::{Context as _, Result};
use scheduler_runtime_workloads::{
    SchedulerBuilderExt, SchedulerDrained, SchedulerLeaseFailoverWorkload,
    SchedulerScenarioBuilder, SchedulerTopology,
};
use testing_framework_core::scenario::Deployer;
use testing_framework_runner_compose::ComposeRunnerError;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let jobs = 100;

    let mut scenario = SchedulerScenarioBuilder::deployment_with(|_| SchedulerTopology::new(3))
        .with_run_duration(Duration::from_secs(35))
        .with_workload(
            SchedulerLeaseFailoverWorkload::new()
                .operations(jobs)
                .lease_ttl(Duration::from_secs(3))
                .rate_per_sec(20),
        )
        .with_expectation(SchedulerDrained::new(jobs).timeout(Duration::from_secs(30)))
        .build()?;

    let deployer = scheduler_runtime_ext::SchedulerComposeDeployer::new();
    let runner = match deployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(ComposeRunnerError::DockerUnavailable) => {
            warn!("docker unavailable; skipping scheduler compose run");
            return Ok(());
        }
        Err(error) => {
            return Err(anyhow::Error::new(error)).context("deploying scheduler compose stack");
        }
    };

    info!("running scheduler compose failover scenario");
    runner
        .run(&mut scenario)
        .await
        .context("running scheduler compose scenario")?;
    Ok(())
}

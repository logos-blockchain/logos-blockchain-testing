use std::time::Duration;

use scheduler_runtime_ext::SchedulerLocalDeployer;
use scheduler_runtime_workloads::{
    SchedulerBuilderExt, SchedulerDrained, SchedulerLeaseFailoverWorkload,
    SchedulerScenarioBuilder, SchedulerTopology,
};
use testing_framework_core::scenario::Deployer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
                .rate_per_sec(25),
        )
        .with_expectation(SchedulerDrained::new(jobs).timeout(Duration::from_secs(30)))
        .build()?;

    let deployer = SchedulerLocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;
    Ok(())
}

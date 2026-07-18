use std::time::Duration;

use multi_app_fixture::{AllJobsCompleted, EnqueueJobs, JobStackApp};
use testing_framework_app::{AppHost, AppHostLocalDeployer, AppScenarioBuilderExt};
use testing_framework_core::scenario::{Deployer, DynError};

const JOB_COUNT: usize = 10;

#[tokio::test]
async fn processes_queued_jobs_and_converges_results() -> Result<(), DynError> {
    let mut scenario = AppHost::scenario()
        .with_app(JobStackApp::new())
        .with_run_duration(Duration::from_secs(10))
        .with_workload(EnqueueJobs::new(JOB_COUNT))
        .with_expectation(AllJobsCompleted::new(JOB_COUNT))
        .build()?;

    let deployer = AppHostLocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;

    Ok(())
}

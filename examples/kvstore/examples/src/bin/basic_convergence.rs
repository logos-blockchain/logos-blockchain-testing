use std::time::Duration;

use kvstore_runtime_ext::KvLocalDeployer;
use kvstore_runtime_workloads::{
    KvBuilderExt, KvClusterAccessible, KvConverges, KvExistingClusterApp, KvScenarioBuilder,
    KvWriteWorkload,
};
use testing_framework_core::scenario::Deployer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut scenario = KvScenarioBuilder::with_existing_kvstore_app(KvExistingClusterApp::nodes(3))
        .with_run_duration(Duration::from_secs(30))
        .with_workload(KvClusterAccessible::new(3))
        .with_workload(
            KvWriteWorkload::new()
                .operations(300)
                .key_count(30)
                .rate_per_sec(30)
                .key_prefix("demo"),
        )
        .with_expectation(KvConverges::new("demo", 30).timeout(Duration::from_secs(25)))
        .build()?;

    let deployer = KvLocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;
    Ok(())
}

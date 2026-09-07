use std::time::Duration;

use kvstore_runtime_workloads::{
    KvClusterAccessible, KvConverges, KvEnv, KvTopology, KvWriteWorkload,
};
use testing_framework_app::{AppHost, AppHostDeployer, AppScenarioBuilderExt as _, ClusterApp};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut scenario = AppHost::scenario()
        .with_app(ClusterApp::<KvEnv>::new(KvTopology::new(3)))
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

    let runner = AppHostDeployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;
    Ok(())
}

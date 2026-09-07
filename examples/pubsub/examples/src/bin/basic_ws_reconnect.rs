use std::time::Duration;

use pubsub_runtime_workloads::{
    PubSubConverges, PubSubEnv, PubSubTopology, PubSubWsReconnectWorkload,
};
use testing_framework_app::{AppHost, AppHostDeployer, AppScenarioBuilderExt as _, ClusterApp};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let topic = "demo.reconnect";
    let workload = PubSubWsReconnectWorkload::new(topic)
        .phase_one_messages(40)
        .disconnected_messages(20)
        .phase_two_messages(40)
        .publish_rate_per_sec(20)
        .timeout(Duration::from_secs(20));

    let mut scenario = AppHost::scenario()
        .with_app(ClusterApp::<PubSubEnv>::new(PubSubTopology::new(3)))
        .with_run_duration(Duration::from_secs(35))
        .with_workload(workload.clone())
        .with_expectation(
            PubSubConverges::new(topic, workload.total_messages()).timeout(Duration::from_secs(30)),
        )
        .build()?;

    let runner = AppHostDeployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;
    Ok(())
}

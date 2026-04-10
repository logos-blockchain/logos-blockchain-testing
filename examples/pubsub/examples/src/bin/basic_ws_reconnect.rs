use std::time::Duration;

use pubsub_runtime_ext::PubSubLocalDeployer;
use pubsub_runtime_workloads::{
    PubSubBuilderExt, PubSubConverges, PubSubScenarioBuilder, PubSubTopology,
    PubSubWsReconnectWorkload,
};
use testing_framework_core::scenario::Deployer;

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

    let mut scenario = PubSubScenarioBuilder::deployment_with(|_| PubSubTopology::new(3))
        .with_run_duration(Duration::from_secs(35))
        .with_workload(workload.clone())
        .with_expectation(
            PubSubConverges::new(topic, workload.total_messages()).timeout(Duration::from_secs(30)),
        )
        .build()?;

    let deployer = PubSubLocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;
    Ok(())
}

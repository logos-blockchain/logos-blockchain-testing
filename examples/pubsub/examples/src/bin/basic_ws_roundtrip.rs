use std::time::Duration;

use pubsub_runtime_workloads::{
    PubSubConverges, PubSubFeedDelivers, PubSubStackApp, PubSubTopology, PubSubWsRoundTripWorkload,
};
use testing_framework_app::{AppHost, AppHostDeployer, AppScenarioBuilderExt as _};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let topic = "demo.topic";
    let messages = 120;

    let mut scenario = AppHost::scenario()
        .with_app(PubSubStackApp::new(PubSubTopology::new(3), topic))
        .with_run_duration(Duration::from_secs(30))
        .with_workload(
            PubSubWsRoundTripWorkload::new(topic)
                .messages(messages)
                .publish_rate_per_sec(25),
        )
        .with_expectation(PubSubFeedDelivers::new(topic, messages).timeout(Duration::from_secs(20)))
        .with_expectation(PubSubConverges::new(topic, messages).timeout(Duration::from_secs(25)))
        .build()?;

    let runner = AppHostDeployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;
    Ok(())
}

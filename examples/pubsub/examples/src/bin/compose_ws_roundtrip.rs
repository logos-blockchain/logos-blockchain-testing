use std::time::Duration;

use anyhow::{Context as _, Result};
use pubsub_runtime_workloads::{
    PubSubBuilderExt, PubSubConverges, PubSubFeedDelivers, PubSubScenarioBuilder, PubSubTopology,
    PubSubWsRoundTripWorkload,
};
use testing_framework_core::scenario::Deployer;
use testing_framework_runner_compose::ComposeRunnerError;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let topic = "demo.topic";
    let messages = 120;

    let mut scenario = PubSubScenarioBuilder::deployment_with(|_| PubSubTopology::new(3))
        .with_topic_feed(topic)
        .with_run_duration(Duration::from_secs(30))
        .with_workload(
            PubSubWsRoundTripWorkload::new(topic)
                .messages(messages)
                .publish_rate_per_sec(20),
        )
        .with_expectation(PubSubFeedDelivers::new(topic, messages).timeout(Duration::from_secs(20)))
        .with_expectation(PubSubConverges::new(topic, messages).timeout(Duration::from_secs(25)))
        .build()?;

    let deployer = pubsub_runtime_ext::PubSubComposeDeployer::new();
    let runner = match deployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(ComposeRunnerError::DockerUnavailable) => {
            warn!("docker unavailable; skipping pubsub compose run");
            return Ok(());
        }
        Err(error) => {
            return Err(anyhow::Error::new(error)).context("deploying pubsub compose stack");
        }
    };

    info!("running pubsub compose ws roundtrip scenario");
    runner
        .run(&mut scenario)
        .await
        .context("running pubsub compose scenario")?;
    Ok(())
}

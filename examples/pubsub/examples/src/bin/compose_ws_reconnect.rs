use std::time::Duration;

use anyhow::{Context as _, Result};
use pubsub_runtime_workloads::{
    PubSubBuilderExt, PubSubConverges, PubSubScenarioBuilder, PubSubTopology,
    PubSubWsReconnectWorkload,
};
use testing_framework_core::scenario::Deployer;
use testing_framework_runner_compose::ComposeRunnerError;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
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

    let deployer = pubsub_runtime_ext::PubSubComposeDeployer::new();
    let runner = match deployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(ComposeRunnerError::DockerUnavailable) => {
            warn!("docker unavailable; skipping pubsub reconnect compose run");
            return Ok(());
        }
        Err(error) => {
            return Err(anyhow::Error::new(error)).context("deploying pubsub compose stack");
        }
    };

    info!("running pubsub compose ws reconnect scenario");
    runner
        .run(&mut scenario)
        .await
        .context("running pubsub compose reconnect scenario")?;
    Ok(())
}

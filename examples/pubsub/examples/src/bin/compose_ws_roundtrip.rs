use std::time::Duration;

use anyhow::{Context as _, Result};
use pubsub_runtime_workloads::{
    PubSubConverges, PubSubFeedDelivers, PubSubStackApp, PubSubTopology, PubSubWsRoundTripWorkload,
};
use testing_framework_app::{
    AppHost, AppHostDeployError, AppHostDeployer, AppScenarioBuilderExt as _,
};
use testing_framework_runner_compose::{ComposeProvisioner, ComposeRunnerError};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let topic = "demo.topic";
    let messages = 120;

    let mut scenario = AppHost::scenario()
        .with_app_using(
            PubSubStackApp::new(PubSubTopology::new(3), topic),
            ComposeProvisioner::default(),
        )
        .with_run_duration(Duration::from_secs(30))
        .with_workload(
            PubSubWsRoundTripWorkload::new(topic)
                .messages(messages)
                .publish_rate_per_sec(20),
        )
        .with_expectation(PubSubFeedDelivers::new(topic, messages).timeout(Duration::from_secs(20)))
        .with_expectation(PubSubConverges::new(topic, messages).timeout(Duration::from_secs(25)))
        .build()?;

    let runner = match AppHostDeployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(error) if is_docker_unavailable(&error) => {
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

fn is_docker_unavailable(error: &AppHostDeployError) -> bool {
    let AppHostDeployError::RuntimeExtensions { source } = error else {
        return false;
    };
    matches!(
        source.downcast_ref::<ComposeRunnerError>(),
        Some(ComposeRunnerError::DockerUnavailable)
    )
}

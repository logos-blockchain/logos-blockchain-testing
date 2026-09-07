use std::time::Duration;

use anyhow::{Context as _, Result};
use pubsub_runtime_workloads::{
    PubSubConverges, PubSubEnv, PubSubTopology, PubSubWsReconnectWorkload,
};
use testing_framework_app::{
    AppHost, AppHostDeployError, AppHostDeployer, AppScenarioBuilderExt as _, ClusterApp,
};
use testing_framework_runner_compose::{ComposeProvisioner, ComposeRunnerError};
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

    let mut scenario = AppHost::scenario()
        .with_app_using(
            ClusterApp::<PubSubEnv>::new(PubSubTopology::new(3)),
            ComposeProvisioner::default(),
        )
        .with_run_duration(Duration::from_secs(35))
        .with_workload(workload.clone())
        .with_expectation(
            PubSubConverges::new(topic, workload.total_messages()).timeout(Duration::from_secs(30)),
        )
        .build()?;

    let runner = match AppHostDeployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(error) if is_docker_unavailable(&error) => {
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

fn is_docker_unavailable(error: &AppHostDeployError) -> bool {
    let AppHostDeployError::RuntimeExtensions { source } = error else {
        return false;
    };
    matches!(
        source.downcast_ref::<ComposeRunnerError>(),
        Some(ComposeRunnerError::DockerUnavailable)
    )
}

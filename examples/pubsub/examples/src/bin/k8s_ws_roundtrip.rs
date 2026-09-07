use std::time::Duration;

use anyhow::{Context as _, Result};
use pubsub_runtime_workloads::{
    PubSubConverges, PubSubFeedDelivers, PubSubStackApp, PubSubTopology, PubSubWsRoundTripWorkload,
};
use testing_framework_app::{
    AppHost, AppHostDeployError, AppHostDeployer, AppScenarioBuilderExt as _,
};
use testing_framework_runner_k8s::{K8sClusterProvisioner, ManualClusterError};
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
            K8sClusterProvisioner,
        )
        .with_run_duration(Duration::from_secs(40))
        .with_workload(
            PubSubWsRoundTripWorkload::new(topic)
                .messages(messages)
                .publish_rate_per_sec(15),
        )
        .with_expectation(PubSubFeedDelivers::new(topic, messages).timeout(Duration::from_secs(30)))
        .with_expectation(PubSubConverges::new(topic, messages).timeout(Duration::from_secs(35)))
        .build()?;

    let runner = match AppHostDeployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(error) => {
            if cluster_may_be_skipped()
                && let Some(reason) = k8s_unavailable_reason(&error)
            {
                warn!("k8s unavailable ({reason}); skipping pubsub k8s run");
                return Ok(());
            }
            return Err(anyhow::Error::new(error)).context("deploying pubsub k8s stack");
        }
    };

    info!("running pubsub k8s ws roundtrip scenario");
    runner
        .run(&mut scenario)
        .await
        .context("running pubsub k8s scenario")?;

    Ok(())
}

fn cluster_may_be_skipped() -> bool {
    std::env::var("K8S_RUNNER_REQUIRE_CLUSTER").as_deref() != Ok("1")
}

fn k8s_unavailable_reason(error: &AppHostDeployError) -> Option<String> {
    let AppHostDeployError::RuntimeExtensions { source } = error else {
        return None;
    };
    match source.downcast_ref::<ManualClusterError>() {
        Some(ManualClusterError::ClientInit { source }) => Some(source.to_string()),
        Some(ManualClusterError::InstallStack { source })
            if k8s_cluster_unavailable(&source.to_string()) =>
        {
            Some(source.to_string())
        }
        _ => None,
    }
}

fn k8s_cluster_unavailable(message: &str) -> bool {
    message.contains("Unable to connect to the server")
        || message.contains("TLS handshake timeout")
        || message.contains("connection refused")
}

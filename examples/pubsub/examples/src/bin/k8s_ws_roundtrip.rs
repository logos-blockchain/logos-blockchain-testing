use std::time::Duration;

use anyhow::{Context as _, Result};
use pubsub_runtime_ext::PubSubK8sDeployer;
use pubsub_runtime_workloads::{
    PubSubBuilderExt, PubSubConverges, PubSubFeedDelivers, PubSubScenarioBuilder, PubSubTopology,
    PubSubWsRoundTripWorkload,
};
use testing_framework_core::scenario::Deployer;
use testing_framework_runner_k8s::K8sRunnerError;
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
        .with_run_duration(Duration::from_secs(40))
        .with_workload(
            PubSubWsRoundTripWorkload::new(topic)
                .messages(messages)
                .publish_rate_per_sec(15),
        )
        .with_expectation(PubSubFeedDelivers::new(topic, messages).timeout(Duration::from_secs(30)))
        .with_expectation(PubSubConverges::new(topic, messages).timeout(Duration::from_secs(35)))
        .build()?;

    let deployer = PubSubK8sDeployer::new();
    let runner = match deployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(K8sRunnerError::ClientInit { source }) => {
            warn!("k8s unavailable ({source}); skipping pubsub k8s run");
            return Ok(());
        }
        Err(K8sRunnerError::InstallStack { source })
            if k8s_cluster_unavailable(&source.to_string()) =>
        {
            warn!("k8s unavailable ({source}); skipping pubsub k8s run");
            return Ok(());
        }
        Err(error) => {
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

fn k8s_cluster_unavailable(message: &str) -> bool {
    message.contains("Unable to connect to the server")
        || message.contains("TLS handshake timeout")
        || message.contains("connection refused")
}

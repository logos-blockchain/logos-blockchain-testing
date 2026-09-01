use std::{env, time::Duration};

use anyhow::{Context as _, Result};
use metrics_counter_runtime_ext::MetricsCounterK8sDeployer;
use metrics_counter_runtime_workloads::{
    CounterIncrementWorkload, MetricsCounterBuilderExt, MetricsCounterScenarioBuilder,
    MetricsCounterTopology, PrometheusCounterAtLeast,
};
use testing_framework_core::scenario::{Deployer, ObservabilityBuilderExt};
use testing_framework_runner_k8s::K8sRunnerError;
use tracing::{info, warn};

const DEFAULT_PROM_URL: &str = "http://127.0.0.1:30991";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let metrics_url = env::var("LOGOS_BLOCKCHAIN_METRICS_QUERY_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_PROM_URL.to_owned());

    let mut scenario =
        MetricsCounterScenarioBuilder::deployment_with(|_| MetricsCounterTopology::new(3))
            .enable_observability()
            .with_metrics_query_url_str(&metrics_url)
            .with_run_duration(Duration::from_secs(25))
            .with_workload(
                CounterIncrementWorkload::new()
                    .operations(240)
                    .rate_per_sec(20),
            )
            .with_expectation(PrometheusCounterAtLeast::new(240.0))
            .build()?;

    let deployer = MetricsCounterK8sDeployer::new();
    let runner = match deployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(K8sRunnerError::ClientInit { source }) if cluster_may_be_skipped() => {
            warn!("k8s unavailable ({source}); skipping metrics-counter k8s run");
            return Ok(());
        }
        Err(K8sRunnerError::InstallStack { source })
            if cluster_may_be_skipped() && k8s_cluster_unavailable(&source.to_string()) =>
        {
            warn!("k8s unavailable ({source}); skipping metrics-counter k8s run");
            return Ok(());
        }
        Err(error) => {
            return Err(anyhow::Error::new(error)).context("deploying metrics-counter k8s stack");
        }
    };

    info!(
        metrics_url,
        "running metrics-counter k8s prometheus scenario"
    );
    runner
        .run(&mut scenario)
        .await
        .context("running metrics-counter k8s scenario")?;

    Ok(())
}

fn cluster_may_be_skipped() -> bool {
    env::var("K8S_RUNNER_REQUIRE_CLUSTER").as_deref() != Ok("1")
}

fn k8s_cluster_unavailable(message: &str) -> bool {
    message.contains("Unable to connect to the server")
        || message.contains("TLS handshake timeout")
        || message.contains("connection refused")
}

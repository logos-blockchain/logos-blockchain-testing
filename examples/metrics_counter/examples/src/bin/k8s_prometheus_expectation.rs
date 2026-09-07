use std::{env, time::Duration};

use anyhow::{Context as _, Result};
use metrics_counter_runtime_workloads::{
    CounterIncrementWorkload, MetricsCounterEnv, MetricsCounterTopology, PrometheusCounterAtLeast,
};
use reqwest::Url;
use testing_framework_app::{
    AppHost, AppHostDeployError, AppHostDeployer, AppScenarioBuilderExt as _, ClusterApp,
};
use testing_framework_core::scenario::ObservabilityInputs;
use testing_framework_runner_k8s::{K8sClusterProvisioner, ManualClusterError};
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
    let observability = ObservabilityInputs {
        metrics_query_url: Some(Url::parse(&metrics_url).context("parsing metrics query url")?),
        ..ObservabilityInputs::default()
    };

    let mut scenario = AppHost::scenario()
        .with_app_using(
            ClusterApp::<MetricsCounterEnv>::new(MetricsCounterTopology::new(3))
                .with_observability(observability),
            K8sClusterProvisioner,
        )
        .with_run_duration(Duration::from_secs(25))
        .with_workload(
            CounterIncrementWorkload::new()
                .operations(240)
                .rate_per_sec(20),
        )
        .with_expectation(PrometheusCounterAtLeast::new(240.0))
        .build()?;

    let runner = match AppHostDeployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(error) => {
            if cluster_may_be_skipped()
                && let Some(reason) = k8s_unavailable_reason(&error)
            {
                warn!("k8s unavailable ({reason}); skipping metrics-counter k8s run");
                return Ok(());
            }
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

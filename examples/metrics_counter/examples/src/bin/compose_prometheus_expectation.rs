use std::{env, time::Duration};

use anyhow::{Context as _, Result};
use metrics_counter_runtime_workloads::{
    CounterIncrementWorkload, MetricsCounterBuilderExt, MetricsCounterScenarioBuilder,
    MetricsCounterTopology, PrometheusCounterAtLeast,
};
use testing_framework_core::scenario::{Deployer, ObservabilityBuilderExt};
use testing_framework_runner_compose::ComposeRunnerError;
use tracing::{info, warn};

const DEFAULT_PROM_URL: &str = "http://127.0.0.1:19091";

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
            .with_run_duration(Duration::from_secs(20))
            .with_workload(
                CounterIncrementWorkload::new()
                    .operations(300)
                    .rate_per_sec(30),
            )
            .with_expectation(PrometheusCounterAtLeast::new(300.0))
            .build()?;

    let deployer = metrics_counter_runtime_ext::MetricsCounterComposeDeployer::new();
    let runner = match deployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(ComposeRunnerError::DockerUnavailable) => {
            warn!("docker unavailable; skipping compose metrics-counter run");
            return Ok(());
        }
        Err(error) => {
            return Err(anyhow::Error::new(error))
                .context("deploying metrics-counter compose stack");
        }
    };

    info!(
        metrics_url,
        "running metrics-counter compose prometheus scenario"
    );
    runner
        .run(&mut scenario)
        .await
        .context("running metrics-counter compose scenario")?;

    Ok(())
}

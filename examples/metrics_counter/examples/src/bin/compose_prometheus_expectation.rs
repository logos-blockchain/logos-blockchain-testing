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
use testing_framework_runner_compose::{ComposeProvisioner, ComposeRunnerError};
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
    let observability = ObservabilityInputs {
        metrics_query_url: Some(Url::parse(&metrics_url).context("parsing metrics query url")?),
        ..ObservabilityInputs::default()
    };

    let mut scenario = AppHost::scenario()
        .with_app_using(
            ClusterApp::<MetricsCounterEnv>::new(MetricsCounterTopology::new(3))
                .with_observability(observability),
            ComposeProvisioner::default(),
        )
        .with_run_duration(Duration::from_secs(20))
        .with_workload(
            CounterIncrementWorkload::new()
                .operations(300)
                .rate_per_sec(30),
        )
        .with_expectation(PrometheusCounterAtLeast::new(300.0))
        .build()?;

    let runner = match AppHostDeployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(error) if docker_unavailable(&error) => {
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

fn docker_unavailable(error: &AppHostDeployError) -> bool {
    let AppHostDeployError::RuntimeExtensions { source } = error else {
        return false;
    };
    matches!(
        source.downcast_ref::<ComposeRunnerError>(),
        Some(ComposeRunnerError::DockerUnavailable)
    )
}

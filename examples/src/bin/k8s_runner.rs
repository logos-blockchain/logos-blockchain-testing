use std::{env, process, time::Duration};

use anyhow::{Context as _, Result, ensure};
use runner_examples::{ScenarioBuilderExt as _, read_env_any};
use testing_framework_core::scenario::{
    Deployer as _, ObservabilityCapability, Runner, ScenarioBuilder,
};
use testing_framework_runner_k8s::{K8sDeployer, K8sRunnerError};
use testing_framework_workflows::ObservabilityBuilderExt as _;
use tracing::{info, warn};

const DEFAULT_RUN_SECS: u64 = 60;
const DEFAULT_VALIDATORS: usize = 1;
const DEFAULT_EXECUTORS: usize = 1;
const MIXED_TXS_PER_BLOCK: u64 = 5;
const TOTAL_WALLETS: usize = 1000;
const TRANSACTION_WALLETS: usize = 500;
const DA_BLOB_RATE: u64 = 1;
const MIN_CONSENSUS_HEIGHT: u64 = 5;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let validators = read_env_any(
        &["NOMOS_DEMO_VALIDATORS", "K8S_DEMO_VALIDATORS"],
        DEFAULT_VALIDATORS,
    );
    let executors = read_env_any(
        &["NOMOS_DEMO_EXECUTORS", "K8S_DEMO_EXECUTORS"],
        DEFAULT_EXECUTORS,
    );
    let run_secs = read_env_any(
        &["NOMOS_DEMO_RUN_SECS", "K8S_DEMO_RUN_SECS"],
        DEFAULT_RUN_SECS,
    );
    info!(validators, executors, run_secs, "starting k8s runner demo");

    if let Err(err) = run_k8s_case(validators, executors, Duration::from_secs(run_secs)).await {
        warn!("k8s runner demo failed: {err}");
        process::exit(1);
    }
}

async fn run_k8s_case(validators: usize, executors: usize, run_duration: Duration) -> Result<()> {
    info!(
        validators,
        executors,
        duration_secs = run_duration.as_secs(),
        "building scenario plan"
    );
    let mut scenario = ScenarioBuilder::topology_with(|t| {
        t.network_star().validators(validators).executors(executors)
    })
    .with_capabilities(ObservabilityCapability::default())
    .wallets(TOTAL_WALLETS)
    .transactions_with(|txs| txs.rate(MIXED_TXS_PER_BLOCK).users(TRANSACTION_WALLETS))
    .da_with(|da| da.blob_rate(DA_BLOB_RATE))
    .with_run_duration(run_duration)
    .expect_consensus_liveness();

    if let Ok(url) = env::var("K8S_RUNNER_METRICS_QUERY_URL")
        .or_else(|_| env::var("NOMOS_METRICS_QUERY_URL"))
        .or_else(|_| env::var("K8S_RUNNER_EXTERNAL_PROMETHEUS_URL"))
        .or_else(|_| env::var("NOMOS_EXTERNAL_PROMETHEUS_URL"))
    {
        if !url.trim().is_empty() {
            scenario = scenario.with_metrics_query_url_str(url.trim());
        }
    }

    if let Ok(url) = env::var("K8S_RUNNER_METRICS_QUERY_GRAFANA_URL")
        .or_else(|_| env::var("NOMOS_METRICS_QUERY_GRAFANA_URL"))
        .or_else(|_| env::var("K8S_RUNNER_EXTERNAL_PROMETHEUS_GRAFANA_URL"))
        .or_else(|_| env::var("NOMOS_EXTERNAL_PROMETHEUS_GRAFANA_URL"))
    {
        if !url.trim().is_empty() {
            scenario = scenario.with_metrics_query_grafana_url_str(url.trim());
        }
    }

    if let Ok(url) = env::var("K8S_RUNNER_METRICS_OTLP_INGEST_URL")
        .or_else(|_| env::var("NOMOS_METRICS_OTLP_INGEST_URL"))
        .or_else(|_| env::var("K8S_RUNNER_EXTERNAL_OTLP_METRICS_ENDPOINT"))
        .or_else(|_| env::var("NOMOS_EXTERNAL_OTLP_METRICS_ENDPOINT"))
    {
        if !url.trim().is_empty() {
            scenario = scenario.with_metrics_otlp_ingest_url_str(url.trim());
        }
    }

    let mut plan = scenario.build();

    let deployer = K8sDeployer::new();
    info!("deploying k8s stack");

    let runner: Runner = match deployer.deploy(&plan).await {
        Ok(runner) => runner,
        Err(K8sRunnerError::ClientInit { source }) => {
            warn!("Kubernetes cluster unavailable ({source}); skipping");
            return Ok(());
        }
        Err(err) => return Err(anyhow::Error::new(err)).context("deploying k8s stack failed"),
    };

    if !runner.context().telemetry().is_configured() {
        warn!("metrics querying is disabled; set NOMOS_METRICS_QUERY_URL to enable PromQL queries");
    }

    let validator_clients = runner.context().node_clients().validator_clients().to_vec();

    info!("running scenario");
    // Keep the handle alive until after we query consensus info, so port-forwards
    // and services stay up while we inspect nodes.
    let handle = runner
        .run(&mut plan)
        .await
        .context("running k8s scenario failed")?;

    for (idx, client) in validator_clients.iter().enumerate() {
        let info = client
            .consensus_info()
            .await
            .with_context(|| format!("validator {idx} consensus_info failed"))?;
        ensure!(
            info.height >= MIN_CONSENSUS_HEIGHT,
            "validator {idx} height {} should reach at least {MIN_CONSENSUS_HEIGHT} blocks",
            info.height
        );
    }

    // Explicitly drop after checks, allowing cleanup to proceed.
    drop(handle);

    Ok(())
}

use std::{env, process, time::Duration};

use anyhow::{Context as _, Result};
use runner_examples::{ScenarioBuilderExt as _, demo, read_env_any};
use testing_framework_core::scenario::{
    Deployer as _, ObservabilityCapability, Runner, ScenarioBuilder,
};
use testing_framework_runner_k8s::{K8sDeployer, K8sRunnerError};
use testing_framework_workflows::ObservabilityBuilderExt as _;
use tracing::{info, warn};

const MIXED_TXS_PER_BLOCK: u64 = 2;
const TOTAL_WALLETS: usize = 200;
const TRANSACTION_WALLETS: usize = 50;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let nodes = read_env_any(&["NOMOS_DEMO_NODES"], demo::DEFAULT_NODES);
    let run_secs = read_env_any(&["NOMOS_DEMO_RUN_SECS"], demo::DEFAULT_RUN_SECS);
    info!(nodes, run_secs, "starting k8s runner demo");

    if let Err(err) = run_k8s_case(nodes, Duration::from_secs(run_secs)).await {
        warn!("k8s runner demo failed: {err:#}");
        process::exit(1);
    }
}

async fn run_k8s_case(nodes: usize, run_duration: Duration) -> Result<()> {
    info!(
        nodes,
        duration_secs = run_duration.as_secs(),
        "building scenario plan"
    );

    let mut scenario = ScenarioBuilder::topology_with(|t| t.network_star().nodes(nodes))
        .with_capabilities(ObservabilityCapability::default())
        .wallets(TOTAL_WALLETS)
        .transactions_with(|txs| txs.rate(MIXED_TXS_PER_BLOCK).users(TRANSACTION_WALLETS))
        .with_run_duration(run_duration)
        .expect_consensus_liveness();

    if let Ok(url) = env::var("NOMOS_METRICS_QUERY_URL") {
        if !url.trim().is_empty() {
            scenario = scenario.with_metrics_query_url_str(url.trim());
        }
    }

    if let Ok(url) = env::var("NOMOS_METRICS_OTLP_INGEST_URL") {
        if !url.trim().is_empty() {
            scenario = scenario.with_metrics_otlp_ingest_url_str(url.trim());
        }
    }

    let mut plan = scenario.build()?;

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

    info!("running scenario");
    runner
        .run(&mut plan)
        .await
        .context("running k8s scenario failed")?;

    Ok(())
}

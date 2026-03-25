use std::{process, time::Duration};

use anyhow::{Context as _, Result};
use lb_ext::{
    CoreBuilderExt as _, LbcExtEnv, LbcK8sDeployer as K8sDeployer, ScenarioBuilder,
    ScenarioBuilderExt as _, configs::network::Libp2pNetworkLayout,
};
use runner_examples::{demo, read_env_any, read_topology_seed_or_default};
use testing_framework_core::scenario::{Deployer as _, Runner};
use testing_framework_runner_k8s::K8sRunnerError;
use tracing::{info, warn};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let nodes = read_env_any(&["LOGOS_BLOCKCHAIN_DEMO_NODES"], demo::DEFAULT_NODES);
    let run_secs = read_env_any(&["LOGOS_BLOCKCHAIN_DEMO_RUN_SECS"], demo::DEFAULT_RUN_SECS);
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

    let seed = read_topology_seed_or_default();

    let scenario = ScenarioBuilder::deployment_with(|t| {
        t.with_network_layout(Libp2pNetworkLayout::Star)
            .with_node_count(nodes)
    })
    .with_observability()
    .with_run_duration(run_duration)
    .with_deployment_seed(seed)
    .initialize_wallet(
        demo::DEFAULT_TOTAL_WALLETS as u64 * 100,
        demo::DEFAULT_TOTAL_WALLETS,
    )
    .transactions_with(|txs| {
        txs.rate(demo::DEFAULT_MIXED_TXS_PER_BLOCK)
            .users(demo::DEFAULT_TRANSACTION_WALLETS)
    })
    .expect_consensus_liveness();

    let mut plan = scenario.build()?;

    let deployer = K8sDeployer::new();
    info!("deploying k8s stack");

    let runner: Runner<LbcExtEnv> = match deployer.deploy(&plan).await {
        Ok(runner) => runner,
        Err(K8sRunnerError::ClientInit { source }) => {
            warn!("Kubernetes cluster unavailable ({source}); skipping");
            return Ok(());
        }
        Err(err) => return Err(anyhow::Error::new(err)).context("deploying k8s stack failed"),
    };

    if !runner.context().telemetry().is_configured() {
        warn!(
            "metrics querying is disabled; set LOGOS_BLOCKCHAIN_METRICS_QUERY_URL to enable PromQL queries"
        );
    }

    info!("running scenario");
    runner
        .run(&mut plan)
        .await
        .context("running k8s scenario failed")?;

    Ok(())
}

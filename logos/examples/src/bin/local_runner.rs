use std::{process, time::Duration};

use anyhow::{Context as _, Result};
use lb_framework::{
    CoreBuilderExt as _, LbcEnv, LbcLocalDeployer, ScenarioBuilder, ScenarioBuilderExt as _,
    configs::network::Libp2pNetworkLayout,
};
use runner_examples::{DeployerKind, demo, read_env_any, read_topology_seed_or_default};
use testing_framework_core::scenario::{Deployer as _, Runner};
use tracing::{info, warn};

#[tokio::main]
async fn main() {
    runner_examples::defaults::init_node_log_dir_defaults(DeployerKind::Local);

    tracing_subscriber::fmt::init();

    let nodes = read_env_any(&["LOGOS_BLOCKCHAIN_DEMO_NODES"], demo::DEFAULT_NODES);
    let run_secs = read_env_any(&["LOGOS_BLOCKCHAIN_DEMO_RUN_SECS"], demo::DEFAULT_RUN_SECS);

    info!(nodes, run_secs, "starting local runner demo");

    if let Err(err) = run_local_case(nodes, Duration::from_secs(run_secs)).await {
        warn!("local runner demo failed: {err:#}");
        process::exit(1);
    }
}

async fn run_local_case(nodes: usize, run_duration: Duration) -> Result<()> {
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

    let deployer = LbcLocalDeployer::default();
    info!("deploying local nodes");

    let runner: Runner<LbcEnv> = deployer
        .deploy(&plan)
        .await
        .context("deploying local nodes failed")?;
    info!("running scenario");

    runner
        .run(&mut plan)
        .await
        .context("running local scenario failed")?;
    info!("scenario complete");

    Ok(())
}

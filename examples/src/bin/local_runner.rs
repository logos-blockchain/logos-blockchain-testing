use std::{env, process, time::Duration};

use anyhow::{Context as _, Result};
use runner_examples::{DeployerKind, ScenarioBuilderExt as _, demo, read_env_any};
use testing_framework_core::scenario::{Deployer as _, Runner, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use tracing::{info, warn};

const MIXED_TXS_PER_BLOCK: u64 = 5;
const TOTAL_WALLETS: usize = 1000;
const TRANSACTION_WALLETS: usize = 500;
const SMOKE_RUN_SECS_MAX: u64 = 30;

#[tokio::main]
async fn main() {
    runner_examples::defaults::init_node_log_dir_defaults(DeployerKind::Local);

    tracing_subscriber::fmt::init();

    if env::var("POL_PROOF_DEV_MODE").is_err() {
        warn!("POL_PROOF_DEV_MODE=true is required for the local runner demo");
        process::exit(1);
    }

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

    let scenario = ScenarioBuilder::topology_with(|t| t.network_star().nodes(nodes))
        .wallets(TOTAL_WALLETS)
        .with_run_duration(run_duration);

    let scenario = if run_duration.as_secs() <= SMOKE_RUN_SECS_MAX {
        scenario
    } else {
        scenario
            .transactions_with(|txs| txs.rate(MIXED_TXS_PER_BLOCK).users(TRANSACTION_WALLETS))
            .expect_consensus_liveness()
    };

    let mut plan = scenario.build()?;

    let deployer = LocalDeployer::default();
    info!("deploying local nodes");

    let runner: Runner = deployer
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

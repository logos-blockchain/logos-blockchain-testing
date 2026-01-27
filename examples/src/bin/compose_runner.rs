use std::{process, time::Duration};

use anyhow::{Context as _, Result};
use runner_examples::{
    ChaosBuilderExt as _, DeployerKind, ScenarioBuilderExt as _, demo, read_env_any,
};
use testing_framework_core::scenario::{Deployer as _, Runner, ScenarioBuilder};
use testing_framework_runner_compose::{ComposeDeployer, ComposeRunnerError};
use tracing::{info, warn};

const MIXED_TXS_PER_BLOCK: u64 = 5;
const TOTAL_WALLETS: usize = 1000;
const TRANSACTION_WALLETS: usize = 500;

// Chaos Testing Constants
const CHAOS_MIN_DELAY_SECS: u64 = 120;
const CHAOS_MAX_DELAY_SECS: u64 = 180;
const CHAOS_COOLDOWN_SECS: u64 = 240;
const CHAOS_DELAY_HEADROOM_SECS: u64 = 1;

#[tokio::main]
async fn main() {
    runner_examples::defaults::init_node_log_dir_defaults(DeployerKind::Compose);

    tracing_subscriber::fmt::init();

    let nodes = read_env_any(&["LOGOS_BLOCKCHAIN_DEMO_NODES"], demo::DEFAULT_NODES);

    let run_secs = read_env_any(&["LOGOS_BLOCKCHAIN_DEMO_RUN_SECS"], demo::DEFAULT_RUN_SECS);

    info!(nodes, run_secs, "starting compose runner demo");

    if let Err(err) = run_compose_case(nodes, Duration::from_secs(run_secs)).await {
        warn!("compose runner demo failed: {err:#}");
        process::exit(1);
    }
}

async fn run_compose_case(nodes: usize, run_duration: Duration) -> Result<()> {
    info!(
        nodes,
        duration_secs = run_duration.as_secs(),
        "building scenario plan"
    );

    let scenario =
        ScenarioBuilder::topology_with(|t| t.network_star().nodes(nodes)).enable_node_control();

    let scenario = if let Some((chaos_min_delay, chaos_max_delay, chaos_target_cooldown)) =
        chaos_timings(run_duration)
    {
        scenario.chaos_with(|c| {
            c.restart()
                .min_delay(chaos_min_delay)
                .max_delay(chaos_max_delay)
                .target_cooldown(chaos_target_cooldown)
                .apply()
        })
    } else {
        scenario
    };

    let mut plan = scenario
        .wallets(TOTAL_WALLETS)
        .transactions_with(|txs| txs.rate(MIXED_TXS_PER_BLOCK).users(TRANSACTION_WALLETS))
        .with_run_duration(run_duration)
        .expect_consensus_liveness()
        .build()?;

    let deployer = ComposeDeployer::new();
    info!("deploying compose stack");

    let runner: Runner = match deployer.deploy(&plan).await {
        Ok(runner) => runner,
        Err(ComposeRunnerError::DockerUnavailable) => {
            warn!("Docker is unavailable; cannot run compose demo");
            return Ok(());
        }
        Err(err) => return Err(anyhow::Error::new(err)).context("deploying compose stack failed"),
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
        .context("running compose scenario failed")?;
    Ok(())
}

fn chaos_timings(run_duration: Duration) -> Option<(Duration, Duration, Duration)> {
    let headroom = Duration::from_secs(CHAOS_DELAY_HEADROOM_SECS);
    let Some(max_allowed_delay) = run_duration.checked_sub(headroom) else {
        return None;
    };

    let chaos_min_delay = Duration::from_secs(CHAOS_MIN_DELAY_SECS);
    if max_allowed_delay <= chaos_min_delay {
        return None;
    }

    let chaos_max_delay = Duration::from_secs(CHAOS_MAX_DELAY_SECS)
        .min(max_allowed_delay)
        .max(chaos_min_delay);

    let chaos_target_cooldown = Duration::from_secs(CHAOS_COOLDOWN_SECS)
        .min(max_allowed_delay)
        .max(chaos_max_delay);

    Some((chaos_min_delay, chaos_max_delay, chaos_target_cooldown))
}

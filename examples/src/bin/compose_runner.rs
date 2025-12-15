use std::time::Duration;

use runner_examples::{ChaosBuilderExt as _, ScenarioBuilderExt as _};
use testing_framework_core::scenario::{Deployer as _, Runner, ScenarioBuilder};
use testing_framework_runner_compose::{ComposeDeployer, ComposeRunnerError};
use tracing::{info, warn};

const DEFAULT_VALIDATORS: usize = 1;
const DEFAULT_EXECUTORS: usize = 1;
const DEFAULT_RUN_SECS: u64 = 60;
const MIXED_TXS_PER_BLOCK: u64 = 5;
const TOTAL_WALLETS: usize = 1000;
const TRANSACTION_WALLETS: usize = 500;

#[tokio::main]
async fn main() {
    // Compose containers mount KZG params at /kzgrs_test_params; ensure the
    // generated configs point there unless the caller overrides explicitly.
    if std::env::var("NOMOS_KZGRS_PARAMS_PATH").is_err() {
        // Safe: setting a process-wide environment variable before any threads
        // or async tasks are spawned.
        unsafe {
            std::env::set_var(
                "NOMOS_KZGRS_PARAMS_PATH",
                "/kzgrs_test_params/kzgrs_test_params",
            );
        }
    }

    tracing_subscriber::fmt::init();

    let validators = read_env_any(
        &["NOMOS_DEMO_VALIDATORS", "COMPOSE_DEMO_VALIDATORS"],
        DEFAULT_VALIDATORS,
    );
    let executors = read_env_any(
        &["NOMOS_DEMO_EXECUTORS", "COMPOSE_DEMO_EXECUTORS"],
        DEFAULT_EXECUTORS,
    );
    let run_secs = read_env_any(
        &["NOMOS_DEMO_RUN_SECS", "COMPOSE_DEMO_RUN_SECS"],
        DEFAULT_RUN_SECS,
    );
    info!(
        validators,
        executors, run_secs, "starting compose runner demo"
    );

    if let Err(err) = run_compose_case(validators, executors, Duration::from_secs(run_secs)).await {
        warn!("compose runner demo failed: {err}");
        std::process::exit(1);
    }
}

#[rustfmt::skip]
async fn run_compose_case(
    validators: usize,
    executors: usize,
    run_duration: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    info!(
        validators,
        executors,
        duration_secs = run_duration.as_secs(),
        "building scenario plan"
    );

    let mut plan = ScenarioBuilder::topology_with(|t| {
        t.network_star()
            .validators(validators)
            .executors(executors)
    })
        .enable_node_control()
        .chaos_with(|c| {
            c.restart()
                // Keep chaos restarts outside the test run window to avoid crash loops on restart.
                .min_delay(Duration::from_secs(120))
                .max_delay(Duration::from_secs(180))
                .target_cooldown(Duration::from_secs(240))
                .apply()
        })
        .wallets(TOTAL_WALLETS)
        .transactions_with(|txs| {
            txs.rate(MIXED_TXS_PER_BLOCK)
                .users(TRANSACTION_WALLETS)
        })
        .da_with(|da| {
            da.channel_rate(1)
                .blob_rate(1)
        })
        .with_run_duration(run_duration)
        .expect_consensus_liveness()
        .build();

    let deployer = ComposeDeployer::new();
    info!("deploying compose stack");
    
    let runner: Runner = match deployer.deploy(&plan).await {
        Ok(runner) => runner,
        Err(ComposeRunnerError::DockerUnavailable) => {
            warn!("Docker is unavailable; cannot run compose demo");
            return Ok(());
        }
        Err(err) => return Err(err.into()),
    };
    if !runner.context().telemetry().is_configured() {
        warn!("compose runner should expose prometheus metrics");
    }

    info!("running scenario");
    runner.run(&mut plan).await.map(|_| ()).map_err(Into::into)
}

fn read_env_any<T>(keys: &[&str], default: T) -> T
where
    T: std::str::FromStr + Copy,
{
    keys.iter()
        .find_map(|key| {
            std::env::var(key)
                .ok()
                .and_then(|raw| raw.parse::<T>().ok())
        })
        .unwrap_or(default)
}

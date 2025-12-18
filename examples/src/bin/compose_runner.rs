use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
    time::Duration,
};

use anyhow::{Context as _, Result};
use runner_examples::{ChaosBuilderExt as _, ScenarioBuilderExt as _, read_env_any};
use testing_framework_core::scenario::{Deployer as _, Runner, ScenarioBuilder};
use testing_framework_runner_compose::{ComposeDeployer, ComposeRunnerError};
use tracing::{info, warn};

const DEFAULT_VALIDATORS: usize = 1;
const DEFAULT_EXECUTORS: usize = 1;
const DEFAULT_RUN_SECS: u64 = 60;
const MIXED_TXS_PER_BLOCK: u64 = 5;
const TOTAL_WALLETS: usize = 1000;
const TRANSACTION_WALLETS: usize = 500;

// Chaos Testing Constants
const CHAOS_MIN_DELAY_SECS: u64 = 120;
const CHAOS_MAX_DELAY_SECS: u64 = 180;
const CHAOS_COOLDOWN_SECS: u64 = 240;
const CHAOS_DELAY_HEADROOM_SECS: u64 = 1;

// DA Testing Constants
const DA_CHANNEL_RATE: u64 = 1;
const DA_BLOB_RATE: u64 = 1;

#[tokio::main]
async fn main() {
    init_node_log_dir_defaults();

    // Compose containers mount KZG params at /kzgrs_test_params; ensure the
    // generated configs point there unless the caller overrides explicitly.
    if env::var("NOMOS_KZGRS_PARAMS_PATH").is_err() {
        // Safe: setting a process-wide environment variable before any threads
        // or async tasks are spawned.
        unsafe {
            env::set_var(
                "NOMOS_KZGRS_PARAMS_PATH",
                "/kzgrs_test_params/kzgrs_test_params",
            );
        }
    }

    tracing_subscriber::fmt::init();

    let validators = read_env_any(&["NOMOS_DEMO_VALIDATORS"], DEFAULT_VALIDATORS);

    let executors = read_env_any(&["NOMOS_DEMO_EXECUTORS"], DEFAULT_EXECUTORS);

    let run_secs = read_env_any(&["NOMOS_DEMO_RUN_SECS"], DEFAULT_RUN_SECS);

    info!(
        validators,
        executors, run_secs, "starting compose runner demo"
    );

    if let Err(err) = run_compose_case(validators, executors, Duration::from_secs(run_secs)).await {
        warn!("compose runner demo failed: {err:#}");
        process::exit(1);
    }
}

fn init_node_log_dir_defaults() {
    if env::var_os("NOMOS_LOG_DIR").is_some() {
        return;
    }

    let repo_root = repo_root();
    let host_dir = repo_root.join("tmp").join("node-logs");
    let _ = fs::create_dir_all(&host_dir);

    // In compose mode, node processes run inside containers; configs should
    // point to the container path, while the compose deployer mounts the host
    // repo's `tmp/node-logs` there.
    unsafe {
        env::set_var("NOMOS_LOG_DIR", "/tmp/node-logs");
    }
}

fn repo_root() -> PathBuf {
    env::var("CARGO_WORKSPACE_DIR")
        .map(PathBuf::from)
        .ok()
        .or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .map(Path::to_path_buf)
        })
        .expect("repo root must be discoverable from CARGO_WORKSPACE_DIR or CARGO_MANIFEST_DIR")
}

async fn run_compose_case(
    validators: usize,
    executors: usize,
    run_duration: Duration,
) -> Result<()> {
    info!(
        validators,
        executors,
        duration_secs = run_duration.as_secs(),
        "building scenario plan"
    );

    let scenario = ScenarioBuilder::topology_with(|t| {
        t.network_star().validators(validators).executors(executors)
    })
    .enable_node_control();

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
        .da_with(|da| da.channel_rate(DA_CHANNEL_RATE).blob_rate(DA_BLOB_RATE))
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
        Err(err) => return Err(anyhow::Error::new(err)).context("deploying compose stack failed"),
    };

    if !runner.context().telemetry().is_configured() {
        warn!("metrics querying is disabled; set NOMOS_METRICS_QUERY_URL to enable PromQL queries");
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

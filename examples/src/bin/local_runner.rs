use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
    time::Duration,
};

use anyhow::{Context as _, Result};
use runner_examples::{ScenarioBuilderExt as _, read_env_any};
use testing_framework_core::scenario::{Deployer as _, Runner, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use tracing::{info, warn};

const DEFAULT_VALIDATORS: usize = 1;
const DEFAULT_EXECUTORS: usize = 1;
const DEFAULT_RUN_SECS: u64 = 60;
const MIXED_TXS_PER_BLOCK: u64 = 5;
const TOTAL_WALLETS: usize = 1000;
const TRANSACTION_WALLETS: usize = 500;
const DA_BLOB_RATE: u64 = 3;
const SMOKE_RUN_SECS_MAX: u64 = 30;

#[tokio::main]
async fn main() {
    init_node_log_dir_defaults();

    tracing_subscriber::fmt::init();

    if env::var("POL_PROOF_DEV_MODE").is_err() {
        warn!("POL_PROOF_DEV_MODE=true is required for the local runner demo");
        process::exit(1);
    }

    let validators = read_env_any(&["NOMOS_DEMO_VALIDATORS"], DEFAULT_VALIDATORS);
    let executors = read_env_any(&["NOMOS_DEMO_EXECUTORS"], DEFAULT_EXECUTORS);
    let run_secs = read_env_any(&["NOMOS_DEMO_RUN_SECS"], DEFAULT_RUN_SECS);

    info!(
        validators,
        executors, run_secs, "starting local runner demo"
    );

    if let Err(err) = run_local_case(validators, executors, Duration::from_secs(run_secs)).await {
        warn!("local runner demo failed: {err:#}");
        process::exit(1);
    }
}

fn init_node_log_dir_defaults() {
    if env::var_os("NOMOS_LOG_DIR").is_some() {
        return;
    }

    let host_dir = repo_root().join("tmp").join("node-logs");
    let _ = fs::create_dir_all(&host_dir);
    unsafe {
        env::set_var("NOMOS_LOG_DIR", host_dir);
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

async fn run_local_case(validators: usize, executors: usize, run_duration: Duration) -> Result<()> {
    info!(
        validators,
        executors,
        duration_secs = run_duration.as_secs(),
        "building scenario plan"
    );

    let scenario = ScenarioBuilder::topology_with(|t| {
        t.network_star().validators(validators).executors(executors)
    })
    .wallets(TOTAL_WALLETS)
    .with_run_duration(run_duration);

    let scenario = if run_duration.as_secs() <= SMOKE_RUN_SECS_MAX {
        scenario
    } else {
        scenario
            .transactions_with(|txs| txs.rate(MIXED_TXS_PER_BLOCK).users(TRANSACTION_WALLETS))
            .da_with(|da| da.blob_rate(DA_BLOB_RATE))
            .expect_consensus_liveness()
    };

    let mut plan = scenario.build();

    let deployer = LocalDeployer::default().with_membership_check(true);
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

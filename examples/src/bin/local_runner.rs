use std::time::Duration;

use runner_examples::ScenarioBuilderExt as _;
use testing_framework_core::scenario::{Deployer as _, Runner, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use tracing::{info, warn};

const DEFAULT_VALIDATORS: usize = 1;
const DEFAULT_EXECUTORS: usize = 1;
const DEFAULT_RUN_SECS: u64 = 60;
const MIXED_TXS_PER_BLOCK: u64 = 5;
const TOTAL_WALLETS: usize = 1000;
const TRANSACTION_WALLETS: usize = 500;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    if std::env::var("POL_PROOF_DEV_MODE").is_err() {
        warn!("POL_PROOF_DEV_MODE=true is required for the local runner demo");

        std::process::exit(1);
    }

    let validators = read_env_any(
        &["NOMOS_DEMO_VALIDATORS", "LOCAL_DEMO_VALIDATORS"],
        DEFAULT_VALIDATORS,
    );
    let executors = read_env_any(
        &["NOMOS_DEMO_EXECUTORS", "LOCAL_DEMO_EXECUTORS"],
        DEFAULT_EXECUTORS,
    );
    let run_secs = read_env_any(
        &["NOMOS_DEMO_RUN_SECS", "LOCAL_DEMO_RUN_SECS"],
        DEFAULT_RUN_SECS,
    );

    info!(
        validators,
        executors, run_secs, "starting local runner demo"
    );

    if let Err(err) = run_local_case(validators, executors, Duration::from_secs(run_secs)).await {
        warn!("local runner demo failed: {err}");

        std::process::exit(1);
    }
}

async fn run_local_case(
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
        t.network_star().validators(validators).executors(executors)
    })
    .wallets(TOTAL_WALLETS)
    .transactions_with(|txs| txs.rate(MIXED_TXS_PER_BLOCK).users(TRANSACTION_WALLETS))
    .da_with(|da| da.blob_rate(1))
    .with_run_duration(run_duration)
    .expect_consensus_liveness()
    .build();

    let deployer = LocalDeployer::default().with_membership_check(true);
    info!("deploying local nodes");

    let runner: Runner = deployer.deploy(&plan).await?;
    info!("running scenario");

    runner.run(&mut plan).await.map(|_| ())?;
    info!("scenario complete");

    Ok(())
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

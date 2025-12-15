use std::time::Duration;

use runner_examples::ScenarioBuilderExt as _;
use testing_framework_core::scenario::{Deployer as _, Runner, ScenarioBuilder};
use testing_framework_runner_k8s::{K8sDeployer, K8sRunnerError};
use tracing::{info, warn};

const DEFAULT_RUN_SECS: u64 = 60;
const DEFAULT_VALIDATORS: usize = 1;
const DEFAULT_EXECUTORS: usize = 1;
const MIXED_TXS_PER_BLOCK: u64 = 5;
const TOTAL_WALLETS: usize = 1000;
const TRANSACTION_WALLETS: usize = 500;

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
        std::process::exit(1);
    }
}

#[rustfmt::skip]
async fn run_k8s_case(
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
        .wallets(TOTAL_WALLETS)
        .transactions_with(|txs| {
            txs.rate(MIXED_TXS_PER_BLOCK)
                .users(TRANSACTION_WALLETS)
        })
        .da_with(|da| {
            da.blob_rate(1)
        })
        .with_run_duration(run_duration)
        .expect_consensus_liveness()
        .build();

    let deployer = K8sDeployer::new();
    info!("deploying k8s stack");
    
    let runner: Runner = match deployer.deploy(&plan).await {
        Ok(runner) => runner,
        Err(K8sRunnerError::ClientInit { source }) => {
            warn!("Kubernetes cluster unavailable ({source}); skipping");
            return Ok(());
        }
        Err(err) => return Err(err.into()),
    };

    if !runner.context().telemetry().is_configured() {
        warn!("k8s runner should expose prometheus metrics");
    }

    let validator_clients = runner.context().node_clients().validator_clients().to_vec();

    info!("running scenario");
    // Keep the handle alive until after we query consensus info, so port-forwards
    // and services stay up while we inspect nodes.
    let handle = runner
        .run(&mut plan)
        .await
        .map_err(|err| format!("k8s scenario failed: {err}"))?;

    for (idx, client) in validator_clients.iter().enumerate() {
        let info = client
            .consensus_info()
            .await
            .map_err(|err| format!("validator {idx} consensus_info failed: {err}"))?;
        if info.height < 5 {
            return Err(format!(
                "validator {idx} height {} should reach at least 5 blocks",
                info.height
            )
            .into());
        }
    }

    // Explicitly drop after checks, allowing cleanup to proceed.
    drop(handle);

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

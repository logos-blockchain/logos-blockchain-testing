use std::time::Duration;

use serial_test::serial;
use testing_framework_core::scenario::{Deployer as _, Runner, ScenarioBuilder};
use testing_framework_runner_compose::{ComposeRunner, ComposeRunnerError};
use tests_workflows::{ChaosBuilderExt as _, ScenarioBuilderExt as _};

const VALIDATORS: usize = 1;
const EXECUTORS: usize = 1;
const RUN_DURATION: Duration = Duration::from_secs(60);
const MIXED_TXS_PER_BLOCK: u64 = 5;
const TOTAL_WALLETS: usize = 64;
const TRANSACTION_WALLETS: usize = 8;

#[tokio::test]
#[serial]
async fn compose_runner_mixed_workloads() {
    run_compose_case(VALIDATORS, EXECUTORS).await;
}

async fn run_compose_case(validators: usize, executors: usize) {
    println!(
        "running compose chaos test with {validators} validator(s) and {executors} executor(s)"
    );

    let mut plan = ScenarioBuilder::with_node_counts(validators, executors)
        .enable_node_control()
        .chaos_random_restart()
        // Keep chaos restarts outside the test run window to avoid crash loops on restart.
        .min_delay(Duration::from_secs(120))
        .max_delay(Duration::from_secs(180))
        .target_cooldown(Duration::from_secs(240))
        .apply()
        .topology()
        .network_star()
        .validators(validators)
        .executors(executors)
        .apply()
        .wallets(TOTAL_WALLETS)
        .transactions()
        .rate(MIXED_TXS_PER_BLOCK)
        .users(TRANSACTION_WALLETS)
        .apply()
        .da()
        .channel_rate(1)
        .blob_rate(1)
        .apply()
        .with_run_duration(RUN_DURATION)
        .expect_consensus_liveness()
        .build();

    let deployer = ComposeRunner::new();
    let runner: Runner = match deployer.deploy(&plan).await {
        Ok(runner) => runner,
        Err(ComposeRunnerError::DockerUnavailable) => {
            eprintln!("Skipping compose_runner_mixed_workloads: Docker is unavailable");
            return;
        }
        Err(err) => panic!("scenario deployment: {err}"),
    };
    let context = runner.context();
    assert!(
        context.telemetry().is_configured(),
        "compose runner should expose prometheus metrics"
    );

    let _handle = runner.run(&mut plan).await.expect("scenario executed");
}

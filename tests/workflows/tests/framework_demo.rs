//! # Test Framework Demo Topology
//!
//! The demo showcases how the testing framework composes deployments:
//!
//! ```text
//! ┌────────────────────────────────────────────────────────┐
//! │ ScenarioBuilder                                        │
//! │   ├─ plan() ───────────▶ Runner::new(plan)             │
//! │   ├─ enable_node_control → chaos workloads             │
//! │   ├─ topology() → network layout → validators/executors│
//! │   └─ workloads (transactions + DA)                     │
//! └────────────────────────────────────────────────────────┘
//!
//! ┌─────────────────────────────┐
//! │ Deployers                   │
//! │   ├─ LocalDeployer          │
//! │   ├─ ComposeRunner          │
//! │   └─ K8sRunner              │
//! │                             │
//! │ Runner                      │
//! │   ├─ execute plan           │
//! │   ├─ telemetry              │
//! │   └─ control handles        │
//! └─────────────────────────────┘
//! ```
//!
//! Component responsibilities:
//!
//! ┌─────────────────────────────────────────────────────────────┐
//! │ Component    │ Role                                         │
//! │--------------│----------------------------------------------│
//! │ Workloads    │ drive traffic (tx, DA blobs, chaos restarts) │
//! │ Expectations │ assert cluster health                       │
//! │ Deployers    │ provision env (host, Docker, k8s)            │
//! │ Runner       │ drives workloads/expectations, telemetry     │
//! └─────────────────────────────────────────────────────────────┘
//!
//! Execution flow:
//!
//! ```text
//! ┌──────┐     ┌───────────────┐     ┌──────────────┐     ┌────────┐
//! │ 1.   │ ─▶ │ 2. Workloads/ │ ─▶ │ 3. Deployers  │ ─▶ │ Runner │
//! │ Plan │     │ Expectations  │     │ Environment   │     │        │
//! └──────┘     └───────────────┘     └──────────────┘     └────────┘
//!                                                           ├─ orchestrate
//!                                                           ├─ telemetry
//!                                                           └─ control
//! ```
//!
//! Cluster interaction:
//!
//! ```text
//!           ┌───────────────┐
//!           │ Deployers     │  provision VMs/containers
//!           └──────┬────────┘
//!                  │
//!        ┌─────────▼─────────┐
//!        │ Cluster Nodes     │ (validators, executors)
//!        └───────┬───────────┘
//!                │
//!      ┌─────────▼──────────┐
//!      │ Runner             │  command/control + telemetry
//!      └──────┬────────┬────┘
//!             │        │
//!        workloads   expectations
//! ```
//!
//! Each runner consumes the same scenario plan; only the deployment backend
//! changes. `full_plan` shows the high-level builder-style DSL, while
//! `explicit_workload_plan` wires the same components explicitly through
//! `with_workload` calls.

use std::{num::NonZeroUsize, time::Duration};

use testing_framework_core::scenario::{
    Deployer as _, NodeControlCapability, Runner, ScenarioBuilder,
};
use testing_framework_runner_compose::ComposeRunner;
use testing_framework_runner_k8s::K8sRunner;
use testing_framework_runner_local::LocalDeployer;
use testing_framework_workflows::ConsensusLiveness;
use tests_workflows::{ChaosBuilderExt as _, ScenarioBuilderExt as _};

const RUN_DURATION: Duration = Duration::from_secs(60);
const VALIDATORS: usize = 1;
const EXECUTORS: usize = 1;
const MIXED_TXS_PER_BLOCK: u64 = 5;
const TOTAL_WALLETS: usize = 64;
const TRANSACTION_WALLETS: usize = 8;

#[rustfmt::skip]
fn explicit_workload_plan() -> testing_framework_core::scenario::Builder<NodeControlCapability> {
    use testing_framework_workflows::workloads::{chaos::RandomRestartWorkload, da, transaction};

    let builder = ScenarioBuilder::with_node_counts(VALIDATORS, EXECUTORS).enable_node_control();

    let topology = builder
        .topology()
            .network_star()
            .validators(VALIDATORS)
            .executors(EXECUTORS)
        .apply();

    let chaos = RandomRestartWorkload::new(
        Duration::from_secs(45),
        Duration::from_secs(75),
        Duration::from_secs(120),
        true,
        true,
    );
    let tx = transaction::Workload::with_rate(MIXED_TXS_PER_BLOCK)
        .expect("transaction rate must be non-zero")
        .with_user_limit(Some(NonZeroUsize::new(TRANSACTION_WALLETS).unwrap()));

    let da_workload = da::Workload::with_channel_count(1);

    topology
        .with_workload(chaos)
        .with_workload(tx)
        .with_workload(da_workload)
        .with_run_duration(RUN_DURATION)
        .expect_consensus_liveness()
        .with_expectation(ConsensusLiveness::default())
}

#[rustfmt::skip]
fn full_plan() -> testing_framework_core::scenario::Builder<NodeControlCapability> {
    ScenarioBuilder::
         with_node_counts(VALIDATORS, EXECUTORS)
        .enable_node_control()
        // configure random restarts and schedule
        .chaos_random_restart()
            // earliest interval between restarts
            .min_delay(Duration::from_secs(45))
            // latest interval between restarts
            .max_delay(Duration::from_secs(75))
            // avoid restarting same node too soon
            .target_cooldown(Duration::from_secs(120))
        .apply()
        // shape the network layout
        .topology()
            // star network layout for libp2p topology
            .network_star()
            // validator count in the plan
            .validators(VALIDATORS)
            // executor count in the plan
            .executors(EXECUTORS)
            .apply()
        // seed wallet accounts
        .wallets(TOTAL_WALLETS)
        // transaction workload configuration
        .transactions()
            // submissions per block
           .rate(MIXED_TXS_PER_BLOCK)
            // number of unique wallet actors
           .users(TRANSACTION_WALLETS)
           .apply()
        // data-availability workload configuration
        .da()
            // channel operations per block
           .channel_rate(1)
            // number of blobs per channel
           .blob_rate(1)
           .apply()
        // run window and expectation
        .with_run_duration(RUN_DURATION)
        // assert consensus keeps up with workload
        .expect_consensus_liveness()
}

#[rustfmt::skip]
fn demo_plan() -> ScenarioBuilder {
    ScenarioBuilder::
         with_node_counts(VALIDATORS, EXECUTORS)
        .topology()
           .network_star()
           .validators(VALIDATORS)
           .executors(EXECUTORS)
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
}

#[tokio::test]
async fn demo_local_runner_mixed_workloads() {
    let mut plan = demo_plan().build();

    let deployer = LocalDeployer::default();

    let runner: Runner = deployer.deploy(&plan).await.expect("scenario deployment");

    let _handle = runner
        .run(&mut plan)
        .await
        .expect("scenario should execute");
}

#[tokio::test]
async fn demo_compose_runner_tx_workload() {
    // Keep the explicit wiring example compiled and linted.
    let _ = explicit_workload_plan();

    let mut plan = full_plan().build();

    let deployer = ComposeRunner::default();

    let runner: Runner = deployer.deploy(&plan).await.expect("scenario deployment");

    let _handle = runner
        .run(&mut plan)
        .await
        .expect("compose scenario should execute");
}

#[tokio::test]
async fn demo_k8s_runner_tx_workload() {
    let mut plan = demo_plan().build();

    let deployer = K8sRunner::default();
    let runner: Runner = deployer.deploy(&plan).await.expect("scenario deployment");

    let _handle = runner
        .run(&mut plan)
        .await
        .expect("k8s scenario should execute");
}

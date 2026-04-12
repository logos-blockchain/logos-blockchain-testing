use std::time::Duration;

use openraft_kv_runtime_ext::{OpenRaftKvBuilderExt, OpenRaftKvEnv, OpenRaftKvScenarioBuilder};
use openraft_kv_runtime_workloads::{OpenRaftKvConverges, OpenRaftKvFailoverWorkload};
use testing_framework_core::scenario::{NodeControlCapability, Scenario};

/// Number of writes issued before the leader restart.
pub const INITIAL_WRITE_BATCH: usize = 8;
/// Number of writes issued after the leader restart.
pub const SECOND_WRITE_BATCH: usize = 8;
/// Total write count expected after the scenario completes.
pub const TOTAL_WRITES: usize = INITIAL_WRITE_BATCH + SECOND_WRITE_BATCH;
/// Key prefix shared by the failover workload and convergence expectation.
pub const RAFT_KEY_PREFIX: &str = "raft-key";

/// Builds the standard failover scenario used by the local and compose
/// binaries.
pub fn build_failover_scenario(
    run_duration: Duration,
    workload_timeout: Duration,
) -> anyhow::Result<Scenario<OpenRaftKvEnv, NodeControlCapability>> {
    Ok(
        OpenRaftKvScenarioBuilder::deployment_with(|deployment| deployment)
            .with_cluster_observer()
            .enable_node_control()
            .with_run_duration(run_duration)
            .with_workload(
                OpenRaftKvFailoverWorkload::new()
                    .first_batch(INITIAL_WRITE_BATCH)
                    .second_batch(SECOND_WRITE_BATCH)
                    .timeout(workload_timeout)
                    .key_prefix(RAFT_KEY_PREFIX),
            )
            .with_expectation(
                OpenRaftKvConverges::new(TOTAL_WRITES)
                    .timeout(run_duration)
                    .key_prefix(RAFT_KEY_PREFIX),
            )
            .build()?,
    )
}

use std::time::Duration;

use openraft_kv_runtime_ext::{OpenRaftKvClusterApp, OpenRaftKvEnv};
use openraft_kv_runtime_workloads::{
    OpenRaftKvClusterAccessible, OpenRaftKvConverges, OpenRaftKvFailoverWorkload,
};
use testing_framework_app::{AppHost, AppHostEnv, AppScenarioBuilderExt as _};
use testing_framework_core::scenario::{ClusterProvisioner, Scenario};

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
pub fn build_failover_scenario<P>(
    run_duration: Duration,
    workload_timeout: Duration,
    provisioner: P,
) -> anyhow::Result<Scenario<AppHostEnv>>
where
    P: ClusterProvisioner<OpenRaftKvEnv>,
{
    Ok(AppHost::scenario()
        .with_app_using(OpenRaftKvClusterApp::nodes(3), provisioner)
        .with_run_duration(run_duration)
        .with_workload(OpenRaftKvClusterAccessible::new(3))
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
        .build()?)
}

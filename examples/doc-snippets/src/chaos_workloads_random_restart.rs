use std::time::Duration;

use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::{ScenarioBuilderExt, workloads::chaos::RandomRestartWorkload};

pub fn random_restart_plan() -> testing_framework_core::scenario::Scenario<
    testing_framework_core::scenario::NodeControlCapability,
> {
    ScenarioBuilder::topology_with(|t| t.network_star().validators(2).executors(1))
        .enable_node_control()
        .with_workload(RandomRestartWorkload::new(
            Duration::from_secs(45),  // min delay
            Duration::from_secs(75),  // max delay
            Duration::from_secs(120), // target cooldown
            true,                     // include validators
            true,                     // include executors
        ))
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(150))
        .build()
}

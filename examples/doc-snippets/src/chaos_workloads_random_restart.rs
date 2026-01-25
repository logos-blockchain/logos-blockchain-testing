use std::time::Duration;

use testing_framework_core::scenario::{NodeControlCapability, Scenario, ScenarioBuilder};
use testing_framework_workflows::{ScenarioBuilderExt, workloads::chaos::RandomRestartWorkload};

use crate::SnippetResult;

pub fn random_restart_plan() -> SnippetResult<Scenario<NodeControlCapability>> {
    ScenarioBuilder::topology_with(|t| t.network_star().validators(2))
        .enable_node_control()
        .with_workload(RandomRestartWorkload::new(
            Duration::from_secs(45),  // min delay
            Duration::from_secs(75),  // max delay
            Duration::from_secs(120), // target cooldown
            true,                     // include validators
        ))
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(150))
        .build()
}

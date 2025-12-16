use std::time::Duration;

use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

pub fn run_duration_plan() -> testing_framework_core::scenario::Scenario<()> {
    ScenarioBuilder::topology_with(|t| t.network_star().validators(1).executors(0))
        .with_run_duration(Duration::from_secs(120)) // Run for 120 seconds
        .build()
}

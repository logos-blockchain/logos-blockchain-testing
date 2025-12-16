use std::time::Duration;

use testing_framework_core::scenario::ScenarioBuilder;

pub fn step_5_run_duration() -> testing_framework_core::scenario::Builder<()> {
    ScenarioBuilder::with_node_counts(1, 1).with_run_duration(Duration::from_secs(60))
}

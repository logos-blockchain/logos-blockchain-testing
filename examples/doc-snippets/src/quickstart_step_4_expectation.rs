use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

pub fn step_4_expectation() -> testing_framework_core::scenario::Builder<()> {
    ScenarioBuilder::with_node_counts(1, 1).expect_consensus_liveness() // This says what success means: blocks must be produced continuously.
}

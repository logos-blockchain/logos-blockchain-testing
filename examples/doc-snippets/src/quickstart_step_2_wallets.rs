use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

pub fn step_2_wallets() -> testing_framework_core::scenario::Builder<()> {
    ScenarioBuilder::with_node_count(2).wallets(1_000) // Seed 1,000 funded wallet accounts
}

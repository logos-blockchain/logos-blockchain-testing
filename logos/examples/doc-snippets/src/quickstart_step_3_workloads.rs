use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

pub fn step_3_workloads() -> testing_framework_core::scenario::Builder<()> {
    ScenarioBuilder::with_node_counts(1)
        .wallets(1_000)
        .transactions_with(|txs| {
            txs.rate(5) // 5 transactions per block
                .users(500) // Use 500 of the 1,000 wallets
        })
}

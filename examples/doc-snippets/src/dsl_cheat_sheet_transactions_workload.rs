use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

pub fn transactions_plan() -> testing_framework_core::scenario::Scenario<()> {
    ScenarioBuilder::topology_with(|t| t.network_star().validators(1).executors(0))
        .wallets(50)
        .transactions_with(|txs| {
            txs.rate(5) // 5 transactions per block
                .users(20) // Use 20 of the seeded wallets
        }) // Finish transaction workload config
        .build()
}

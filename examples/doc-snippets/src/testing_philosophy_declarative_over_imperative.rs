use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

pub fn declarative_over_imperative() {
    // Good: declarative
    let _plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(2).executors(1))
        .transactions_with(|txs| {
            txs.rate(5) // 5 transactions per block
        })
        .expect_consensus_liveness()
        .build();

    // Bad: imperative (framework doesn't work this way)
    // spawn_validator(); spawn_executor();
    // loop { submit_tx(); check_block(); }
}

use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

use crate::SnippetResult;

pub fn declarative_over_imperative() -> SnippetResult<()> {
    // Good: declarative
    let _plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(2))
        .transactions_with(|txs| {
            txs.rate(5) // 5 transactions per block
        })
        .expect_consensus_liveness()
        .build()?;

    // Bad: imperative (framework doesn't work this way)
    // spawn_validator();
    // loop { submit_tx(); check_block(); }

    Ok(())
}

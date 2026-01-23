use std::time::Duration;

use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

use crate::SnippetResult;

pub fn protocol_time_not_wall_time() -> SnippetResult<()> {
    // Good: protocol-oriented thinking
    let _plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(2))
        .transactions_with(|txs| {
            txs.rate(5) // 5 transactions per block
        })
        .with_run_duration(Duration::from_secs(60)) // Let framework calculate expected blocks
        .expect_consensus_liveness() // "Did we produce the expected blocks?"
        .build()?;

    // Bad: wall-clock assumptions
    // "I expect exactly 30 blocks in 60 seconds"
    // This breaks on slow CI where slot timing might drift

    Ok(())
}

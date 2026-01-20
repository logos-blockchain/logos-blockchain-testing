use std::time::Duration;

use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::{ChaosBuilderExt, ScenarioBuilderExt};

use crate::SnippetResult;

pub fn determinism_first() -> SnippetResult<()> {
    // Separate: functional test (deterministic)
    let _plan = ScenarioBuilder::topology_with(|t| t.network_star().nodes(3))
        .transactions_with(|txs| {
            txs.rate(5) // 5 transactions per block
        })
        .expect_consensus_liveness()
        .build()?;

    // Separate: chaos test (introduces randomness)
    let _chaos_plan = ScenarioBuilder::topology_with(|t| t.network_star().nodes(5))
        .enable_node_control()
        .chaos_with(|c| {
            c.restart()
                .min_delay(Duration::from_secs(30))
                .max_delay(Duration::from_secs(60))
                .target_cooldown(Duration::from_secs(45))
                .apply()
        })
        .transactions_with(|txs| {
            txs.rate(5) // 5 transactions per block
        })
        .expect_consensus_liveness()
        .build()?;
    Ok(())
}

use std::time::Duration;

use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

use crate::SnippetResult;

pub fn minimum_run_windows() -> SnippetResult<()> {
    // Bad: too short (~2 blocks with default 2s slots, 0.9 coeff)
    let _too_short = ScenarioBuilder::with_node_count(1)
        .with_run_duration(Duration::from_secs(5))
        .expect_consensus_liveness()
        .build()?;

    // Good: enough blocks for assertions (~27 blocks with default 2s slots, 0.9
    // coeff)
    let _good = ScenarioBuilder::with_node_count(1)
        .with_run_duration(Duration::from_secs(60))
        .expect_consensus_liveness()
        .build()?;
    Ok(())
}

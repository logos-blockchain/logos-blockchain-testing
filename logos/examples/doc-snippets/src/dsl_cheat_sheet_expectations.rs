use testing_framework_core::scenario::{Scenario, ScenarioBuilder};
use testing_framework_workflows::ScenarioBuilderExt;

use crate::SnippetResult;

pub fn expectations_plan() -> SnippetResult<Scenario<()>> {
    ScenarioBuilder::topology_with(|t| t.network_star().nodes(1))
        .expect_consensus_liveness() // Assert blocks are produced continuously
        .build()
}

use std::time::Duration;

use testing_framework_core::scenario::{Scenario, ScenarioBuilder};
use testing_framework_workflows::ScenarioBuilderExt;

use crate::SnippetResult;

pub fn scenario_plan() -> SnippetResult<Scenario<()>> {
    ScenarioBuilder::topology_with(|t| t.network_star().validators(3).executors(2))
        .wallets(50)
        .transactions_with(|txs| txs.rate(5).users(20))
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(90))
        .build()
}

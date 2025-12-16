use std::time::Duration;

use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

pub fn scenario_plan() -> testing_framework_core::scenario::Scenario<()> {
    ScenarioBuilder::topology_with(|t| t.network_star().validators(3).executors(2))
        .wallets(50)
        .transactions_with(|txs| txs.rate(5).users(20))
        .da_with(|da| da.channel_rate(1).blob_rate(2))
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(90))
        .build()
}

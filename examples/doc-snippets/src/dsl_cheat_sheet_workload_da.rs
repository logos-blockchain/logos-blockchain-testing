use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

pub fn da_plan() -> testing_framework_core::scenario::Scenario<()> {
    ScenarioBuilder::topology_with(|t| t.network_star().validators(1).executors(1))
        .wallets(50)
        .da_with(|da| {
            da.channel_rate(1) // number of DA channels to run
                .blob_rate(2) // target 2 blobs per block (headroom applied)
                .headroom_percent(20) // optional headroom when sizing channels
        }) // Finish DA workload config
        .build()
}

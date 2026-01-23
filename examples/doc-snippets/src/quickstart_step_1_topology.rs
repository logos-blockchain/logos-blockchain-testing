use testing_framework_core::scenario::ScenarioBuilder;

pub fn step_1_topology() -> testing_framework_core::scenario::Builder<()> {
    ScenarioBuilder::topology_with(|t| {
        t.network_star() // Star topology: all nodes connect to seed
            .validators(2) // 2 validator nodes
    })
}

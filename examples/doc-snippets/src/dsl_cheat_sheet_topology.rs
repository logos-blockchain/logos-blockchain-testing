use testing_framework_core::scenario::{Builder, ScenarioBuilder};

pub fn topology() -> Builder<()> {
    ScenarioBuilder::topology_with(|t| {
        t.network_star() // Star topology (all connect to seed node)
            .validators(3) // Number of validator nodes
    })
}

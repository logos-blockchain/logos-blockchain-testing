use testing_framework_core::scenario::{Builder, ScenarioBuilder};

pub fn topology() -> Builder<()> {
    ScenarioBuilder::topology_with(|t| {
        t.network_star() // Star topology (all connect to seed node)
            .nodes(3) // Number of node nodes
    })
}

use std::time::Duration;

use testing_framework_core::scenario::{NodeControlCapability, ScenarioBuilder};
use testing_framework_workflows::{ChaosBuilderExt, ScenarioBuilderExt};

use crate::SnippetResult;

pub fn chaos_plan()
-> SnippetResult<testing_framework_core::scenario::Scenario<NodeControlCapability>> {
    ScenarioBuilder::topology_with(|t| t.network_star().nodes(3))
        .enable_node_control() // Enable node control capability
        .chaos_with(|c| {
            c.restart() // Random restart chaos
                .min_delay(Duration::from_secs(30)) // Min time between restarts
                .max_delay(Duration::from_secs(60)) // Max time between restarts
                .target_cooldown(Duration::from_secs(45)) // Cooldown after restart
                .apply() // Required for chaos configuration
        })
        .build()
}

use testing_framework_core::scenario::ScenarioBuilder;

use crate::{OpenRaftKvEnv, OpenRaftKvTopology};

/// Scenario builder alias used by the OpenRaft example binaries.
pub type OpenRaftKvScenarioBuilder = ScenarioBuilder<OpenRaftKvEnv>;

/// Convenience helpers for constructing the fixed three-node OpenRaft topology.
pub trait OpenRaftKvBuilderExt: Sized {
    /// Starts from the default three-node deployment and lets callers adjust
    /// it.
    fn deployment_with(f: impl FnOnce(OpenRaftKvTopology) -> OpenRaftKvTopology) -> Self;
}

impl OpenRaftKvBuilderExt for OpenRaftKvScenarioBuilder {
    fn deployment_with(f: impl FnOnce(OpenRaftKvTopology) -> OpenRaftKvTopology) -> Self {
        OpenRaftKvScenarioBuilder::with_deployment(f(OpenRaftKvTopology::new(3)))
    }
}

use testing_framework_core::scenario::{CoreBuilderExt, ScenarioBuilder};

use crate::{
    OpenRaftClusterObserver, OpenRaftKvEnv, OpenRaftKvTopology, openraft_cluster_source_provider,
};

/// Scenario builder alias used by the OpenRaft example binaries.
pub type OpenRaftKvScenarioBuilder = ScenarioBuilder<OpenRaftKvEnv>;

/// Convenience helpers for constructing the fixed three-node OpenRaft topology.
pub trait OpenRaftKvBuilderExt: Sized {
    /// Starts from the default three-node deployment and lets callers adjust
    /// it.
    fn deployment_with(f: impl FnOnce(OpenRaftKvTopology) -> OpenRaftKvTopology) -> Self;

    /// Attaches the default OpenRaft cluster observer to the scenario.
    fn with_cluster_observer(self) -> Self;
}

impl OpenRaftKvBuilderExt for OpenRaftKvScenarioBuilder {
    fn deployment_with(f: impl FnOnce(OpenRaftKvTopology) -> OpenRaftKvTopology) -> Self {
        OpenRaftKvScenarioBuilder::with_deployment(f(OpenRaftKvTopology::new(3)))
    }

    fn with_cluster_observer(self) -> Self {
        self.with_observer(
            OpenRaftClusterObserver,
            openraft_cluster_source_provider,
            OpenRaftClusterObserver::config(),
        )
    }
}

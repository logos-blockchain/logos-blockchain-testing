use testing_framework_core::scenario::ScenarioBuilder;

use crate::{SchedulerEnv, SchedulerTopology};

pub type SchedulerScenarioBuilder = ScenarioBuilder<SchedulerEnv>;

pub trait SchedulerBuilderExt: Sized {
    fn deployment_with(f: impl FnOnce(SchedulerTopology) -> SchedulerTopology) -> Self;
}

impl SchedulerBuilderExt for SchedulerScenarioBuilder {
    fn deployment_with(f: impl FnOnce(SchedulerTopology) -> SchedulerTopology) -> Self {
        SchedulerScenarioBuilder::with_deployment(f(SchedulerTopology::new(3)))
    }
}

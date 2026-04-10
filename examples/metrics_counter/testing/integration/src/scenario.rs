use testing_framework_core::scenario::ScenarioBuilder;

use crate::{MetricsCounterEnv, MetricsCounterTopology};

pub type MetricsCounterScenarioBuilder = ScenarioBuilder<MetricsCounterEnv>;

pub trait MetricsCounterBuilderExt: Sized {
    fn deployment_with(f: impl FnOnce(MetricsCounterTopology) -> MetricsCounterTopology) -> Self;
}

impl MetricsCounterBuilderExt for MetricsCounterScenarioBuilder {
    fn deployment_with(f: impl FnOnce(MetricsCounterTopology) -> MetricsCounterTopology) -> Self {
        MetricsCounterScenarioBuilder::with_deployment(f(MetricsCounterTopology::new(3)))
    }
}

use testing_framework_core::scenario::ScenarioBuilder;

use crate::{QueueEnv, QueueTopology};

pub type QueueScenarioBuilder = ScenarioBuilder<QueueEnv>;

pub trait QueueBuilderExt: Sized {
    fn deployment_with(f: impl FnOnce(QueueTopology) -> QueueTopology) -> Self;
}

impl QueueBuilderExt for QueueScenarioBuilder {
    fn deployment_with(f: impl FnOnce(QueueTopology) -> QueueTopology) -> Self {
        QueueScenarioBuilder::with_deployment(f(QueueTopology::new(3)))
    }
}

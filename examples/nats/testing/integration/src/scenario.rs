use testing_framework_core::scenario::ScenarioBuilder;

use crate::{NatsEnv, NatsTopology};

pub type NatsScenarioBuilder = ScenarioBuilder<NatsEnv>;

pub trait NatsBuilderExt: Sized {
    fn deployment_with(f: impl FnOnce(NatsTopology) -> NatsTopology) -> Self;
}

impl NatsBuilderExt for NatsScenarioBuilder {
    fn deployment_with(f: impl FnOnce(NatsTopology) -> NatsTopology) -> Self {
        NatsScenarioBuilder::with_deployment(f(NatsTopology::new(3)))
    }
}

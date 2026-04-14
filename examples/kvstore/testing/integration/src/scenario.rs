use testing_framework_core::scenario::ScenarioBuilder;

use crate::{KvEnv, KvTopology};

pub type KvScenarioBuilder = ScenarioBuilder<KvEnv>;

pub trait KvBuilderExt: Sized {
    fn deployment_with(f: impl FnOnce(KvTopology) -> KvTopology) -> Self;
}

impl KvBuilderExt for KvScenarioBuilder {
    fn deployment_with(f: impl FnOnce(KvTopology) -> KvTopology) -> Self {
        KvScenarioBuilder::with_deployment(f(KvTopology::new(3)))
    }
}

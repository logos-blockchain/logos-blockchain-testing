use testing_framework_core::scenario::ScenarioBuilder;

use crate::{RedisStreamsEnv, RedisStreamsTopology};

pub type RedisStreamsScenarioBuilder = ScenarioBuilder<RedisStreamsEnv>;

pub trait RedisStreamsBuilderExt: Sized {
    fn deployment_with(f: impl FnOnce(RedisStreamsTopology) -> RedisStreamsTopology) -> Self;
}

impl RedisStreamsBuilderExt for RedisStreamsScenarioBuilder {
    fn deployment_with(f: impl FnOnce(RedisStreamsTopology) -> RedisStreamsTopology) -> Self {
        RedisStreamsScenarioBuilder::with_deployment(f(RedisStreamsTopology::new(3)))
    }
}

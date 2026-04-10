mod app;
mod compose_env;
pub mod scenario;

pub use app::*;
pub use scenario::{RedisStreamsBuilderExt, RedisStreamsScenarioBuilder};

pub type RedisStreamsComposeDeployer =
    testing_framework_runner_compose::ComposeDeployer<RedisStreamsEnv>;

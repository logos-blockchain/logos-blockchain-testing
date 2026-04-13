mod app;
mod compose_env;
mod local_env;
pub mod scenario;

pub use app::*;
pub use scenario::{NatsBuilderExt, NatsScenarioBuilder};

pub type NatsLocalDeployer = testing_framework_runner_local::ProcessDeployer<NatsEnv>;
pub type NatsComposeDeployer = testing_framework_runner_compose::ComposeDeployer<NatsEnv>;

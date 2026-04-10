mod app;
mod compose_env;
mod local_env;
pub mod scenario;

pub use app::*;
pub use scenario::{QueueBuilderExt, QueueScenarioBuilder};

pub type QueueLocalDeployer = testing_framework_runner_local::ProcessDeployer<QueueEnv>;
pub type QueueComposeDeployer = testing_framework_runner_compose::ComposeDeployer<QueueEnv>;

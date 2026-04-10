mod app;
mod compose_env;
mod local_env;
pub mod scenario;

pub use app::*;
pub use scenario::{SchedulerBuilderExt, SchedulerScenarioBuilder};

pub type SchedulerLocalDeployer = testing_framework_runner_local::ProcessDeployer<SchedulerEnv>;
pub type SchedulerComposeDeployer = testing_framework_runner_compose::ComposeDeployer<SchedulerEnv>;

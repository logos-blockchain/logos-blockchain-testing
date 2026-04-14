mod app;
mod compose_env;
mod k8s_env;
mod local_env;
pub mod scenario;

pub use app::*;
pub use scenario::{KvBuilderExt, KvScenarioBuilder};

pub type KvLocalDeployer = testing_framework_runner_local::ProcessDeployer<KvEnv>;
pub type KvComposeDeployer = testing_framework_runner_compose::ComposeDeployer<KvEnv>;
pub type KvK8sDeployer = testing_framework_runner_k8s::K8sDeployer<KvEnv>;

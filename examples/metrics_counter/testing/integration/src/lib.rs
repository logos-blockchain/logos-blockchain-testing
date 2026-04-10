mod app;
mod compose_env;
mod k8s_env;
mod local_env;
pub mod scenario;

pub use app::*;
pub use scenario::{MetricsCounterBuilderExt, MetricsCounterScenarioBuilder};

pub type MetricsCounterComposeDeployer =
    testing_framework_runner_compose::ComposeDeployer<MetricsCounterEnv>;
pub type MetricsCounterK8sDeployer = testing_framework_runner_k8s::K8sDeployer<MetricsCounterEnv>;

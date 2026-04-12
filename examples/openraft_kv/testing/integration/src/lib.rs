mod app;
mod compose_env;
mod k8s_env;
mod local_env;
mod observation;
pub mod scenario;

pub use app::*;
pub use observation::*;
pub use scenario::{OpenRaftKvBuilderExt, OpenRaftKvScenarioBuilder};

/// Local process deployer for the OpenRaft example app.
pub type OpenRaftKvLocalDeployer = testing_framework_runner_local::ProcessDeployer<OpenRaftKvEnv>;
/// Docker Compose deployer for the OpenRaft example app.
pub type OpenRaftKvComposeDeployer =
    testing_framework_runner_compose::ComposeDeployer<OpenRaftKvEnv>;
/// Kubernetes deployer for the OpenRaft example app.
pub type OpenRaftKvK8sDeployer = testing_framework_runner_k8s::K8sDeployer<OpenRaftKvEnv>;

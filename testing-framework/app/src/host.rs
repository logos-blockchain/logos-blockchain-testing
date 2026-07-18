use async_trait::async_trait;
use testing_framework_core::{
    scenario::{Application, DynError, NodeAccess, ScenarioBuilder},
    topology::DeploymentDescriptor,
};
use testing_framework_runner_local::{LocalDeployerEnv, ProcessDeployer};

#[derive(Clone, Default)]
/// Empty outer topology for scenarios whose entire system is deployed as apps.
///
/// Application deployments registered with [`crate::AppScenarioBuilderExt`]
/// provide all processes and clusters, so this topology contains no nodes.
pub struct AppHostTopology;

impl DeploymentDescriptor for AppHostTopology {
    fn node_count(&self) -> usize {
        0
    }
}

/// Testing-framework application environment for [`AppHost`] scenarios.
///
/// This environment intentionally has no outer node client.
/// Application-specific clients are exposed as typed handles instead.
pub struct AppHostEnv;

#[async_trait]
impl Application for AppHostEnv {
    type Deployment = AppHostTopology;
    type NodeClient = ();
    type NodeConfig = ();

    fn build_node_client(_access: &NodeAccess) -> Result<Self::NodeClient, DynError> {
        Err(std::io::Error::other("app host does not expose node clients").into())
    }
}

#[async_trait]
impl LocalDeployerEnv for AppHostEnv {}

/// Entry point for a scenario composed entirely from application deployments.
pub struct AppHost;

impl AppHost {
    /// Creates an empty scenario builder ready for `.with_app(...)` calls.
    #[must_use]
    pub fn scenario() -> AppHostScenarioBuilder {
        ScenarioBuilder::with_deployment(AppHostTopology)
    }
}

/// Scenario builder for an application-hosted heterogeneous stack.
pub type AppHostScenarioBuilder = ScenarioBuilder<AppHostEnv>;
/// Local process deployer used to execute an [`AppHost`] scenario.
pub type AppHostLocalDeployer = ProcessDeployer<AppHostEnv>;

use async_trait::async_trait;
use testing_framework_core::{
    scenario::{
        Application, ClusterControlProfile, Deployer, DynError, Metrics, NodeAccess, NodeClients,
        Runner, Scenario, ScenarioBuilder, internal::RuntimeAssembly,
    },
    topology::DeploymentDescriptor,
};
use testing_framework_runner_local::{LocalDeployerEnv, ProcessDeployer};
use thiserror::Error;

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
/// Backend-neutral deployer for scenarios whose resources come entirely from
/// application runtime extensions.
#[derive(Clone, Copy, Debug, Default)]
pub struct AppHostDeployer;

/// Failures while preparing an application-hosted scenario.
#[derive(Debug, Error)]
pub enum AppHostDeployError {
    /// Application deployment failed before workloads started.
    #[error("application deployment failed: {source}")]
    RuntimeExtensions {
        /// Underlying application deployment error.
        #[source]
        source: DynError,
    },
    /// No application runtime was installed.
    #[error("application host requires at least one deployed application")]
    Empty,
}

#[async_trait]
impl Deployer<AppHostEnv> for AppHostDeployer {
    type Error = AppHostDeployError;

    async fn deploy(
        &self,
        scenario: &Scenario<AppHostEnv>,
    ) -> Result<Runner<AppHostEnv>, Self::Error> {
        let node_clients = NodeClients::default();
        let (runtime_extensions, runtime_cleanup) = scenario
            .prepare_runtime_extensions(node_clients.clone())
            .await
            .map_err(|source| AppHostDeployError::RuntimeExtensions { source })?;

        if runtime_extensions.is_empty() {
            return Err(AppHostDeployError::Empty);
        }

        let assembly = RuntimeAssembly::new(
            scenario.deployment().clone(),
            node_clients,
            scenario.duration(),
            scenario.expectation_cooldown(),
            ClusterControlProfile::ExternalUncontrolled,
            Metrics::empty(),
        )
        .with_runtime_extensions(runtime_extensions)
        .with_cleanup_guard(runtime_cleanup);

        Ok(assembly.build_runner(None))
    }
}

/// Local process deployer used to execute an [`AppHost`] scenario.
pub type AppHostLocalDeployer = ProcessDeployer<AppHostEnv>;

#[cfg(test)]
mod tests {
    use testing_framework_core::scenario::Deployer;

    use super::{AppHost, AppHostDeployError, AppHostDeployer};

    #[tokio::test]
    async fn backend_neutral_deployer_rejects_an_empty_app_host() {
        let scenario = AppHost::scenario().build().unwrap();

        let result = AppHostDeployer.deploy(&scenario).await;

        assert!(matches!(result, Err(AppHostDeployError::Empty)));
    }
}

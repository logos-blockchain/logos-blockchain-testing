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

impl AppHostDeployer {
    /// Deploys every registered application and returns the scenario runner.
    pub async fn deploy(
        &self,
        scenario: &Scenario<AppHostEnv>,
    ) -> Result<Runner<AppHostEnv>, AppHostDeployError> {
        let node_clients = NodeClients::default();
        let (runtime_extensions, runtime_cleanup, control) = scenario
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
            control
                .profile()
                .unwrap_or(ClusterControlProfile::ExternalUncontrolled),
            Metrics::empty(),
        )
        .with_node_control_granted(control.node_control_granted())
        .with_runtime_extensions(runtime_extensions)
        .with_cleanup_guard(runtime_cleanup);

        Ok(assembly.build_runner(None))
    }
}

#[async_trait]
impl Deployer<AppHostEnv> for AppHostDeployer {
    type Error = AppHostDeployError;

    async fn deploy(
        &self,
        scenario: &Scenario<AppHostEnv>,
    ) -> Result<Runner<AppHostEnv>, Self::Error> {
        Self::deploy(self, scenario).await
    }
}

/// Local process deployer used to execute an [`AppHost`] scenario.
pub type AppHostLocalDeployer = ProcessDeployer<AppHostEnv>;

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use testing_framework_core::{
        scenario::{
            Application, ClusterControlProfile, ClusterHandle, ClusterProvisioner, ClusterRequest,
            ClusterSource, ClusterUnit, DynError, NodeClients,
        },
        topology::NodeCountTopology,
    };

    use super::{AppHost, AppHostDeployError, AppHostDeployer, AppHostEnv};
    use crate::{AppDeployment, AppRunContextExt as _, AppScenarioBuilderExt as _, DeployContext};

    #[tokio::test]
    async fn backend_neutral_deployer_rejects_an_empty_app_host() {
        let scenario = AppHost::scenario().build().unwrap();

        let result = AppHostDeployer.deploy(&scenario).await;

        assert!(matches!(result, Err(AppHostDeployError::Empty)));
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct AlphaHandle(&'static str);

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct BetaHandle(&'static str);

    #[derive(Clone)]
    struct AlphaApp;

    #[async_trait]
    impl AppDeployment<AppHostEnv> for AlphaApp {
        type Handle = AlphaHandle;

        async fn deploy(
            self,
            _ctx: &mut DeployContext<AppHostEnv>,
        ) -> Result<Self::Handle, DynError> {
            Ok(AlphaHandle("alpha"))
        }
    }

    #[derive(Clone)]
    struct BetaApp;

    #[async_trait]
    impl AppDeployment<AppHostEnv> for BetaApp {
        type Handle = BetaHandle;

        async fn deploy(
            self,
            _ctx: &mut DeployContext<AppHostEnv>,
        ) -> Result<Self::Handle, DynError> {
            Ok(BetaHandle("beta"))
        }
    }

    #[tokio::test]
    async fn composing_two_apps_exposes_both_handles() {
        let scenario = AppHost::scenario()
            .with_app(AlphaApp)
            .with_app(BetaApp)
            .build()
            .unwrap();

        let runner = AppHostDeployer
            .deploy(&scenario)
            .await
            .expect("two composed apps must deploy");
        let context = runner.context();

        assert_eq!(
            context.require_app::<AlphaHandle>().unwrap(),
            AlphaHandle("alpha")
        );
        assert_eq!(
            context.require_app::<BetaHandle>().unwrap(),
            BetaHandle("beta")
        );
    }

    #[tokio::test]
    async fn duplicate_handles_across_apps_fail_with_a_clear_error() {
        let scenario = AppHost::scenario()
            .with_app(AlphaApp)
            .with_app(AlphaApp)
            .build()
            .unwrap();

        let error = AppHostDeployer
            .deploy(&scenario)
            .await
            .err()
            .expect("duplicate handles across apps must fail");

        assert!(error.to_string().contains("already exposed"));
    }

    struct ClusterEnv;

    #[async_trait]
    impl Application for ClusterEnv {
        type Deployment = NodeCountTopology;
        type NodeClient = u8;
        type NodeConfig = ();
    }

    #[derive(Clone)]
    struct ManagedProvisioner;

    #[async_trait]
    impl ClusterProvisioner<ClusterEnv> for ManagedProvisioner {
        async fn provision_cluster(
            &self,
            request: ClusterRequest<ClusterEnv>,
        ) -> Result<ClusterUnit<ClusterEnv>, DynError> {
            let ClusterSource::Managed { deployment, .. } = request.source() else {
                return Err("expected a managed cluster request".into());
            };
            Ok(ClusterUnit::new(
                Some(deployment.clone()),
                NodeClients::default(),
                ClusterControlProfile::FrameworkManaged,
            ))
        }
    }

    #[derive(Clone)]
    struct ManagedClusterApp;

    #[async_trait]
    impl AppDeployment<AppHostEnv, ManagedProvisioner> for ManagedClusterApp {
        type Handle = ClusterHandle<ClusterEnv>;

        async fn deploy(
            self,
            ctx: &mut DeployContext<AppHostEnv, ManagedProvisioner>,
        ) -> Result<Self::Handle, DynError> {
            ctx.deploy_cluster(ClusterRequest::managed(NodeCountTopology::new(2)))
                .await
        }
    }

    #[tokio::test]
    async fn managed_cluster_apps_report_framework_managed_control() {
        let scenario = AppHost::scenario()
            .with_app_using(ManagedClusterApp, ManagedProvisioner)
            .build()
            .unwrap();

        let runner = AppHostDeployer
            .deploy(&scenario)
            .await
            .expect("managed cluster app must deploy");

        assert_eq!(
            runner.context().cluster_control_profile(),
            ClusterControlProfile::FrameworkManaged
        );
    }
}
